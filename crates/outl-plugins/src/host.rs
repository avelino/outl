//! `PluginHost` — loads plugins and drives them on behalf of a client.
//!
//! The host is the only thing that holds both a [`PluginEngine`] and (briefly,
//! per call) `&mut Workspace`. It:
//!
//! - loads a plugin (manifest + bundle), intersecting capabilities with the
//!   client and freezing the approved [`PermissionSet`];
//! - lists the commands a plugin contributes (for a palette / slash menu);
//! - runs a command, applying the emitted intents through `outl-actions`;
//! - dispatches applied ops to `onOp` hooks via [`PluginHost::sync_hooks`].
//!
//! Anti-loop: the host tracks how far into the op log it has dispatched
//! (`last_seen`). Ops a plugin itself produces advance `last_seen` too, so they
//! never re-trigger hooks — no plugin → op → plugin cycle.

use std::rc::Rc;
use std::str::FromStr;

use serde_json::Value;

use outl_actions::block;
use outl_actions::page::{self, PageKind};
use outl_actions::template as tpl_actions;
use outl_actions::todo::{split_todo, TodoState};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;
use outl_shortcuts::{ChordSequence, Mode};

use crate::capability::{self, Capability, CapabilityMatch, ClientCapabilities};
use crate::error::{PluginError, Result};
use crate::manifest::PluginManifest;
use crate::model::{
    BlockView, HostIntent, LogOpView, MoveTarget, PageView, ReadModel, TemplateView,
    TransformResult,
};
use crate::permission::{Permission, PermissionSet};
use crate::runtime::PluginEngine;
use crate::secrets::{plugin_service, KeyringStore, SecretStore};

/// One loaded, activated plugin.
struct LoadedPlugin {
    manifest: PluginManifest,
    caps: CapabilityMatch,
    perms: PermissionSet,
    config: Value,
    engine: Box<dyn PluginEngine>,
}

impl LoadedPlugin {
    fn has(&self, cap: Capability) -> bool {
        self.caps.granted.contains(&cap)
    }
}

/// A command a plugin contributes, surfaced to the client's palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id (`contributes.commands[].id`).
    pub command_id: String,
    /// Human title.
    pub title: String,
}

/// A plugin keybinding, parsed and ready for a client to merge into its chord
/// dispatcher. The client runs `run_command(plugin_id, command_id)` when the
/// chord fires.
#[derive(Debug, Clone)]
pub struct PluginBinding {
    /// The parsed chord sequence.
    pub chord: ChordSequence,
    /// Mode the binding fires in (plugin chords are `Global`).
    pub mode: Mode,
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id to run.
    pub command_id: String,
    /// Human description (for the help overlay).
    pub description: String,
}

/// A content transformer a plugin declares for a code-fence language.
#[derive(Debug, Clone)]
pub struct TransformerEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Code-fence language this transformer handles.
    pub lang: String,
    /// `"text"` or `"rich"`.
    pub kind: String,
}

/// A toolbar button a plugin contributes to a GUI client's chrome.
#[derive(Debug, Clone)]
pub struct ToolbarButtonEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id to run on tap.
    pub command_id: String,
    /// Glyph/emoji to render.
    pub icon: String,
    /// Optional tooltip / accessible label.
    pub title: Option<String>,
}

/// The result of running a command or a hook sweep.
#[derive(Debug, Clone, Default)]
pub struct PluginRun {
    /// Number of intents successfully applied.
    pub applied: usize,
    /// `console.log` / `ctx.log` lines.
    pub logs: Vec<String>,
    /// `ctx.ui.notify` messages.
    pub notifications: Vec<String>,
    /// `ctx.ui.render` payloads (author-written HTML/JS) — only populated for
    /// plugins granted the `ui-render` capability on this client.
    pub views: Vec<String>,
    /// Non-fatal errors (denied permission, bad node id, action failure).
    pub errors: Vec<String>,
}

/// Loads plugins and runs them against a client's workspace.
pub struct PluginHost {
    client_caps: ClientCapabilities,
    plugins: Vec<LoadedPlugin>,
    last_seen: usize,
    last_pushed: usize,
    dispatching: bool,
    /// `<root>/.outl/plugins` — where each plugin's `storage.json` lives. When
    /// unset (tests), `ctx.storage` stays in-memory and is not persisted.
    storage_dir: Option<std::path::PathBuf>,
    /// Backing keychain for `ctx.secrets`. Defaults to the OS keychain
    /// ([`KeyringStore`]); tests swap in an in-memory store via
    /// [`PluginHost::set_secret_store`].
    secret_store: Rc<dyn SecretStore>,
}

impl PluginHost {
    /// Create a host for a client that implements `client_caps`.
    pub fn new(client_caps: ClientCapabilities) -> Self {
        Self {
            client_caps,
            plugins: Vec::new(),
            last_seen: 0,
            last_pushed: 0,
            dispatching: false,
            storage_dir: None,
            secret_store: Rc::new(KeyringStore::new()),
        }
    }

    /// Swap the backing secret store (tests use an in-memory store so
    /// `ctx.secrets` never touches the OS keychain or prompts in CI).
    pub fn set_secret_store(&mut self, store: Rc<dyn SecretStore>) {
        self.secret_store = store;
    }

    /// Configure `ctx.secrets` for the plugin at `idx` this turn: grant the
    /// keychain store only when the plugin holds the `secrets` permission, and
    /// namespace it to this plugin's service so it can never read another's.
    fn prepare_secrets(&mut self, idx: usize) {
        let p = &self.plugins[idx];
        let enabled = p.perms.check(&Permission::Secrets);
        let service = plugin_service(&p.manifest.id);
        let store = enabled.then(|| Rc::clone(&self.secret_store));
        self.plugins[idx]
            .engine
            .set_secrets(enabled, service, store);
    }

    /// Tell the host where to persist per-plugin `ctx.storage` KVs (the
    /// `.outl/plugins` directory). The loader sets this so `storage:local`
    /// survives restarts.
    pub fn set_storage_dir(&mut self, dir: std::path::PathBuf) {
        self.storage_dir = Some(dir);
    }

    /// Load a plugin's local KV from disk and hand it to its engine for this
    /// turn (no-op / disabled when `storage:local` isn't granted).
    fn prepare_storage(&mut self, idx: usize) {
        let p = &self.plugins[idx];
        let enabled = p.perms.check(&crate::permission::Permission::StorageLocal);
        let kv = if enabled {
            self.storage_path(&p.manifest.id)
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        self.plugins[idx].engine.set_storage(enabled, kv);
    }

    /// Persist a plugin's KV if it changed this turn.
    fn flush_storage(&mut self, idx: usize) {
        if let Some(kv) = self.plugins[idx].engine.take_dirty_storage() {
            let id = self.plugins[idx].manifest.id.clone();
            if let Some(path) = self.storage_path(&id) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(json) = serde_json::to_vec_pretty(&kv) {
                    let _ = std::fs::write(path, json);
                }
            }
        }
    }

    fn storage_path(&self, id: &str) -> Option<std::path::PathBuf> {
        self.storage_dir
            .as_ref()
            .map(|d| d.join(id).join("storage.json"))
    }

    /// Load and activate a plugin from an already-read manifest + bundle.
    ///
    /// `approved` is the permission set the user approved (from the lockfile);
    /// `config` is the plugin's stored config. The bundle is evaluated and
    /// `activate(ctx)` runs, registering the plugin's commands and hooks.
    pub fn load_plugin(
        &mut self,
        manifest: PluginManifest,
        bundle: &str,
        approved: PermissionSet,
        config: Value,
    ) -> Result<()> {
        let caps = capability::intersect(&manifest.capabilities, &self.client_caps);
        let mut engine = new_engine()?;
        // Grant the engine the network domains the user approved, so a
        // `ctx.net.fetch` to anything else is refused inside the engine.
        let net_domains: Vec<crate::permission::NetworkDomain> = approved
            .as_slice()
            .iter()
            .filter_map(|p| match p {
                crate::permission::Permission::Network(d) => Some(d.clone()),
                _ => None,
            })
            .collect();
        engine.set_network(net_domains);
        engine
            .load(bundle)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        self.plugins.push(LoadedPlugin {
            manifest,
            caps,
            perms: approved,
            config,
            engine,
        });
        Ok(())
    }

    /// Capabilities a plugin declared but this client can't honor — surface
    /// these to the user as warnings.
    pub fn missing_capabilities(&self, plugin_id: &str) -> Vec<Capability> {
        self.plugins
            .iter()
            .find(|p| p.manifest.id == plugin_id)
            .map(|p| p.caps.missing.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Every command contributed by a loaded plugin whose `slash-command`
    /// capability is granted on this client.
    pub fn commands(&self) -> Vec<CommandEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::SlashCommand) {
                continue;
            }
            for c in &p.manifest.contributes.commands {
                out.push(CommandEntry {
                    plugin_id: p.manifest.id.clone(),
                    command_id: c.id.clone(),
                    title: c.title.clone(),
                });
            }
        }
        out
    }

    /// Plugin keybindings for `client` (`"tui"` / `"desktop"` / `"mobile"`),
    /// parsed and ready to merge into the client's chord dispatcher. Only
    /// plugins granted the `keybinding` capability are included; a binding whose
    /// `when` names a different client, or whose chord string doesn't parse, is
    /// skipped.
    pub fn keybindings(&self, client: &str) -> Vec<PluginBinding> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::Keybinding) {
                continue;
            }
            for kb in &p.manifest.contributes.keybindings {
                if kb.when.as_deref().is_some_and(|w| w != client) {
                    continue;
                }
                let Some(chord) = ChordSequence::parse(&kb.key) else {
                    continue;
                };
                let description = p
                    .manifest
                    .contributes
                    .commands
                    .iter()
                    .find(|c| c.id == kb.command)
                    .map(|c| c.title.clone())
                    .unwrap_or_else(|| kb.command.clone());
                out.push(PluginBinding {
                    chord,
                    mode: Mode::Global,
                    plugin_id: p.manifest.id.clone(),
                    command_id: kb.command.clone(),
                    description,
                });
            }
        }
        out
    }

    /// Toolbar buttons for `client` (`"desktop"` / `"mobile"`). Only plugins
    /// granted the `toolbar-button` capability are included.
    pub fn toolbar_buttons(&self, client: &str) -> Vec<ToolbarButtonEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::ToolbarButton) {
                continue;
            }
            for tb in &p.manifest.contributes.toolbar {
                if tb.when.as_deref().is_some_and(|w| w != client) {
                    continue;
                }
                out.push(ToolbarButtonEntry {
                    plugin_id: p.manifest.id.clone(),
                    command_id: tb.command.clone(),
                    icon: tb.icon.clone(),
                    title: tb.title.clone(),
                });
            }
        }
        out
    }

    /// Content transformers granted on this client, keyed by code-fence
    /// language. A client renders a fence by looking up its language here; if a
    /// transformer matches, it calls [`PluginHost::transform_block`]. `text`
    /// transformers need `content-transformer:text`, `rich` ones need
    /// `content-transformer:rich` — a client lacking the capability never sees
    /// the entry.
    pub fn transformers(&self) -> Vec<TransformerEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            for t in &p.manifest.contributes.transformers {
                let cap = if t.kind == "rich" {
                    Capability::ContentTransformerRich
                } else {
                    Capability::ContentTransformerText
                };
                if !p.has(cap) {
                    continue;
                }
                out.push(TransformerEntry {
                    plugin_id: p.manifest.id.clone(),
                    lang: t.lang.clone(),
                    kind: t.kind.clone(),
                });
            }
        }
        out
    }

    /// Run a plugin's content transformer for `lang` against `input`, returning
    /// the descriptor (`{kind, content}`) it produced, or `None` when it
    /// declined / has no transformer for that language.
    pub fn transform_block(
        &mut self,
        plugin_id: &str,
        lang: &str,
        input: &str,
    ) -> Result<Option<TransformResult>> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| PluginError::Manifest(format!("no such plugin `{plugin_id}`")))?;
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        let json = self.plugins[idx]
            .engine
            .transform(lang, input, &config)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        match json {
            None => Ok(None),
            Some(s) => Ok(Some(serde_json::from_str::<TransformResult>(&s)?)),
        }
    }

    /// Mark the host as caught up with the current log — call after loading so
    /// pre-existing ops don't fire hooks on startup.
    pub fn mark_synced(&mut self, workspace: &Workspace) {
        self.last_seen = workspace.log().len();
    }

    /// Run a plugin command, applying whatever intents it emits.
    pub fn run_command(
        &mut self,
        workspace: &mut Workspace,
        hlc: &HlcGenerator,
        plugin_id: &str,
        command_id: &str,
    ) -> Result<PluginRun> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| PluginError::Manifest(format!("no such plugin `{plugin_id}`")))?;

        let read_model = build_read_model(workspace);
        self.prepare_storage(idx);
        self.prepare_secrets(idx);
        let (config, turn) = {
            let p = &mut self.plugins[idx];
            let config = p.config.clone();
            let turn = p
                .engine
                .run_command(command_id, &read_model, &config)
                .map_err(|e| PluginError::Engine(e.to_string()))?;
            (config, turn)
        };
        let _ = config;
        self.flush_storage(idx);

        let perms = self.plugins[idx].perms.clone();
        let has_ui = self.plugins[idx].has(Capability::UiRender);
        let mut run = PluginRun {
            logs: turn.logs,
            notifications: turn.notifications,
            views: if has_ui { turn.views } else { Vec::new() },
            ..Default::default()
        };
        apply_intents(workspace, hlc, &perms, plugin_id, &turn.intents, &mut run);
        // Command-applied ops should not re-fire op hooks on the next sweep.
        self.last_seen = workspace.log().len();
        Ok(run)
    }

    /// Dispatch every op applied since the last sweep to plugins' `onOp` hooks,
    /// applying any intents they emit. Idempotent and loop-safe.
    pub fn sync_hooks(
        &mut self,
        workspace: &mut Workspace,
        hlc: &HlcGenerator,
    ) -> Result<PluginRun> {
        let mut run = PluginRun::default();
        if self.dispatching {
            return Ok(run);
        }
        let total = workspace.log().len();
        if total <= self.last_seen {
            self.last_seen = total;
            return Ok(run);
        }

        // Project the new ops before touching the workspace mutably.
        let views: Vec<LogOpView> = workspace
            .log()
            .iter()
            .skip(self.last_seen)
            .filter_map(|lo| project_op(workspace, lo))
            .collect();
        // Mark seen now so plugin-applied ops below don't get re-dispatched.
        self.last_seen = total;

        self.dispatching = true;
        let read_model = build_read_model(workspace);
        let plugin_ids: Vec<usize> = (0..self.plugins.len())
            .filter(|&i| self.plugins[i].has(Capability::OpHook))
            .collect();

        for view in &views {
            for &i in &plugin_ids {
                self.prepare_storage(i);
                self.prepare_secrets(i);
                let (perms, plugin_id, has_ui, turn) = {
                    let p = &mut self.plugins[i];
                    let config = p.config.clone();
                    match p.engine.dispatch_op(view, &read_model, &config) {
                        Ok(turn) => (
                            p.perms.clone(),
                            p.manifest.id.clone(),
                            p.has(Capability::UiRender),
                            turn,
                        ),
                        Err(e) => {
                            run.errors.push(format!("{}: {e}", p.manifest.id));
                            continue;
                        }
                    }
                };
                self.flush_storage(i);
                run.logs.extend(turn.logs);
                run.notifications.extend(turn.notifications);
                if has_ui {
                    run.views.extend(turn.views);
                }
                apply_intents(workspace, hlc, &perms, &plugin_id, &turn.intents, &mut run);
            }
        }

        // Plugin-applied ops advanced the log; swallow them so they don't loop.
        self.last_seen = workspace.log().len();
        self.dispatching = false;
        Ok(run)
    }

    /// Index of the first plugin granted the `sync-transport` capability, if any.
    fn sync_plugin(&self) -> Option<usize> {
        self.plugins
            .iter()
            .position(|p| p.has(Capability::SyncTransport))
    }

    /// Hand the sync-transport plugin the JSONL of **locally-authored** ops
    /// produced since the last push, so it can ship them to its backend.
    /// Returns how many ops were shipped. Ops injected from peers (via
    /// [`PluginHost::sync_pull`]) carry a foreign actor and are filtered out, so
    /// they never echo back.
    pub fn sync_push(&mut self, workspace: &Workspace) -> Result<usize> {
        let Some(idx) = self.sync_plugin() else {
            return Ok(0);
        };
        let local = workspace.actor;
        let lines: Vec<String> = workspace
            .log()
            .iter()
            .skip(self.last_pushed)
            .filter(|lo| lo.actor == local)
            .map(serde_json::to_string)
            .collect::<std::result::Result<_, _>>()?;
        self.last_pushed = workspace.log().len();
        if lines.is_empty() {
            return Ok(0);
        }
        let jsonl = lines.join("\n");
        let count = lines.len();
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        self.plugins[idx]
            .engine
            .sync_push(&jsonl, &config)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        Ok(count)
    }

    /// Ask the sync-transport plugin for remote ops and apply each through
    /// `Workspace::apply` (HLC-observed, idempotent). The plugin only transports
    /// bytes — every op still goes through the CRDT, so a malformed line is
    /// skipped, never trusted into the tree raw. Returns how many applied.
    pub fn sync_pull(&mut self, workspace: &mut Workspace, hlc: &HlcGenerator) -> Result<usize> {
        let Some(idx) = self.sync_plugin() else {
            return Ok(0);
        };
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        let Some(jsonl) = self.plugins[idx]
            .engine
            .sync_pull(&config)
            .map_err(|e| PluginError::Engine(e.to_string()))?
        else {
            return Ok(0);
        };

        let mut applied = 0;
        for line in jsonl.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(op) = serde_json::from_str::<LogOp>(line) else {
                continue; // skip a malformed line, never panic
            };
            hlc.observe(op.ts); // advance the local clock so causality holds
            if workspace.apply(op).is_ok() {
                applied += 1;
            }
        }
        // Injected ops advanced the log but are foreign-actor, so sync_push
        // won't re-ship them; keep last_pushed in step so we don't rescan them.
        self.last_pushed = workspace.log().len();
        Ok(applied)
    }
}

/// Apply a plugin's intents, gating each on the approved permission set.
fn apply_intents(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    perms: &PermissionSet,
    plugin_id: &str,
    intents: &[HostIntent],
    run: &mut PluginRun,
) {
    for intent in intents {
        if !perms.check(&intent.required_permission()) {
            run.errors.push(format!(
                "{plugin_id}: denied `{}` for intent",
                intent.required_permission()
            ));
            continue;
        }
        match apply_one(workspace, hlc, intent) {
            Ok(()) => run.applied += 1,
            Err(e) => run.errors.push(format!("{plugin_id}: {e}")),
        }
    }
}

fn apply_one(workspace: &mut Workspace, hlc: &HlcGenerator, intent: &HostIntent) -> Result<()> {
    match intent {
        HostIntent::EditText { node, text } => {
            block::edit_text(workspace, hlc, parse_node(node)?, text).map_err(act)
        }
        HostIntent::CreateUnder { parent, text } => {
            block::create_under(workspace, hlc, parse_node(parent)?, Some(text))
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::CreateAfter { after, text } => {
            block::create_after(workspace, hlc, parse_node(after)?, Some(text))
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::ToggleTodo { node } => {
            block::toggle_todo(workspace, hlc, parse_node(node)?).map_err(act)
        }
        HostIntent::Delete { node } => {
            block::delete(workspace, hlc, parse_node(node)?).map_err(act)
        }
        HostIntent::EnsurePage { slug } => {
            page::open_or_create(workspace, hlc, slug, slug, PageKind::Page)
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::InstantiateTemplate { name, under } => {
            let target = parse_node(under)?;
            let slug = page_slug_of(workspace, target).unwrap_or_default();
            // Derive the page date from the target slug so `{{date}}`
            // resolves to the journal's own date on a daily note, matching
            // the CLI/TUI path — passing `None` here made it always render
            // today's date regardless of which page the block lives on.
            let page_date = outl_actions::dates::date_from_slug(&slug);
            tpl_actions::instantiate_template(workspace, hlc, name, target, &slug, page_date)
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::Move { node, target } => {
            let n = parse_node(node)?;
            let parent = match target {
                MoveTarget::ToParent { to_parent } => parse_node(to_parent)?,
                MoveTarget::ToPage { to_page } => {
                    page::open_or_create(workspace, hlc, to_page, to_page, PageKind::Page)
                        .map_err(act)?
                }
            };
            block::move_under(workspace, hlc, n, parent).map_err(act)
        }
        HostIntent::AppendTree { target, tree } => {
            let parent = match target {
                MoveTarget::ToParent { to_parent } => parse_node(to_parent)?,
                // `toPage` is a slug, same as `Move`/`EnsurePage`. Pages are
                // flat (`pages/<slug>.md`); the slug is also the page title, so
                // the plugin reads the day back with `query({ page: slug })`.
                MoveTarget::ToPage { to_page } => {
                    page::open_or_create(workspace, hlc, to_page, to_page, PageKind::Page)
                        .map_err(act)?
                }
            };
            append_tree(workspace, hlc, parent, tree).map_err(act)
        }
    }
}

/// Recursively create `nodes` under `parent`, descending into children with the
/// id the host gets back from each create. This is what lets `AppendTree`
/// materialize a nested structure in one turn — the plugin never sees the ids,
/// the host threads them through here.
fn append_tree(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    nodes: &[crate::model::TreeNode],
) -> std::result::Result<(), outl_actions::error::ActionError> {
    for node in nodes {
        let id = block::create_under(workspace, hlc, parent, Some(&node.text))?;
        if !node.children.is_empty() {
            append_tree(workspace, hlc, id, &node.children)?;
        }
    }
    Ok(())
}

fn act(e: outl_actions::error::ActionError) -> PluginError {
    PluginError::Engine(e.to_string())
}

fn parse_node(s: &str) -> Result<NodeId> {
    // `NodeId` is a `NodeId(pub Ulid)` newtype with no `FromStr`; parse the
    // ULID and wrap it, same as the desktop's `helpers::parse_node_id`.
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|_| PluginError::BadNodeId(s.to_string()))
}

/// Build the read-only snapshot the JS side queries this turn.
fn build_read_model(workspace: &Workspace) -> ReadModel {
    let pages: Vec<PageView> = page::list_all(workspace)
        .into_iter()
        .map(|m| PageView {
            slug: m.slug,
            title: m.title,
            kind: m.kind.as_str().to_string(),
        })
        .collect();

    let templates: Vec<TemplateView> = tpl_actions::list_templates(workspace)
        .into_iter()
        .map(|t| TemplateView {
            name: t.name,
            slug: t.slug,
            params: t.params,
        })
        .collect();

    let mut blocks = Vec::new();
    for (node, parent, _pos) in workspace.tree().iter_nodes() {
        if node == NodeId::root() || node == NodeId::trash() {
            continue;
        }
        // Skip page nodes themselves — plugins operate on blocks, not pages.
        if page::page_meta(workspace, node).is_some() {
            continue;
        }
        let Some(raw) = workspace.block_text(node) else {
            continue;
        };
        let (todo, body) = split_todo(&raw);
        // A block whose parent is the page root (or root/trash) is top-level:
        // report `null` so the plugin sees "no addressable parent block".
        let parent_id = if parent == NodeId::root()
            || parent == NodeId::trash()
            || page::page_meta(workspace, parent).is_some()
        {
            None
        } else {
            Some(parent.to_string())
        };
        blocks.push(BlockView {
            id: node.to_string(),
            text: body.to_string(),
            todo: todo.map(|t| t.as_str().to_string()),
            parent: parent_id,
            page: page_slug_of(workspace, node).unwrap_or_default(),
        });
    }
    ReadModel {
        blocks,
        pages,
        templates,
        op: None,
    }
}

/// Climb parents until one is a page; return its slug.
fn page_slug_of(workspace: &Workspace, node: NodeId) -> Option<String> {
    let mut cur = node;
    loop {
        let parent = workspace.tree().parent(cur)?;
        if let Some(meta) = page::page_meta(workspace, parent) {
            return Some(meta.slug);
        }
        if parent == NodeId::root() {
            return None;
        }
        cur = parent;
    }
}

/// Project an applied [`LogOp`] to the stable JS shape.
fn project_op(workspace: &Workspace, lo: &LogOp) -> Option<LogOpView> {
    let mk = |kind: &str, node: NodeId| LogOpView {
        kind: kind.to_string(),
        node: node.to_string(),
        text: None,
        todo: None,
    };
    Some(match &lo.op {
        Op::Create { node, .. } => mk("Create", *node),
        Op::Move { node, .. } => mk("Move", *node),
        Op::SetProp { node, .. } => mk("SetProp", *node),
        Op::SetCollapsed { node, .. } => mk("SetCollapsed", *node),
        Op::Edit { node, .. } => {
            let raw = workspace.block_text(*node).unwrap_or_default();
            let (todo, body) = split_todo(&raw);
            LogOpView {
                kind: "Edit".to_string(),
                node: node.to_string(),
                text: Some(body.to_string()),
                todo: todo.map(|t: TodoState| t.as_str().to_string()),
            }
        }
    })
}

#[cfg(feature = "js")]
fn new_engine() -> Result<Box<dyn PluginEngine>> {
    crate::engine::BoaEngine::new()
        .map(|e| Box::new(e) as Box<dyn PluginEngine>)
        .map_err(|e| PluginError::Engine(e.to_string()))
}

#[cfg(not(feature = "js"))]
fn new_engine() -> Result<Box<dyn PluginEngine>> {
    Err(PluginError::NoEngine)
}

#[cfg(all(test, feature = "js"))]
#[path = "host_tests.rs"]
mod tests;
