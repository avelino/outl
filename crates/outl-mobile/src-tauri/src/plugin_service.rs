//! Plugin integration for the mobile client.
//!
//! [`outl_plugins::PluginHost`] embeds a Boa `Context`, which is **not
//! `Send`**. Tauri's `AppState` must be `Send + Sync`, so the host can
//! never live in `AppState` directly. Instead it lives on a **dedicated
//! plugin thread** that owns it for the process lifetime; [`PluginService`]
//! (which *is* `Send + Sync`) holds only the [`std::sync::mpsc::Sender`]
//! end of a request channel and goes into `AppState`.
//!
//! The `Workspace` itself **is** `Send` and is shared through the same
//! `Arc<Mutex<Option<Workspace>>>` every Tauri command already locks. The
//! plugin thread is handed a clone of that `Arc` (plus the per-device
//! `HlcGenerator` and the storage root) at construction. When a request
//! arrives the thread locks the workspace, calls `host.run_command(...)`
//! / `host.sync_hooks(...)`, re-projects the `.md` of every page (a plugin
//! can touch any page — same rationale as the TUI's
//! `reproject_after_plugin`), then replies over the request's reply
//! channel. The Boa `Context` therefore never crosses a thread boundary.
//!
//! **Divergence from the desktop crate.** The mobile `AppState` resolves a
//! single `storage_root: PathBuf` at boot (no live folder swap — switching
//! folders is a relaunch, see `state.rs`), so this service takes an
//! **owned `PathBuf`**, not the desktop's `Arc<Mutex<Option<PathBuf>>>`.
//! That removes the desktop's "re-load on root swap" branch: the host
//! loads plugins exactly once, lazily, on the first request after the
//! workspace opens.
//!
//! Best-effort end to end: a host that can't be built, a plugin that fails
//! to load, or a re-projection error never blocks the editor. The frontend
//! surfaces a plugin's `notify` / error output as a toast and re-renders
//! the current page from the freshly-projected workspace.
//!
//! iOS note: Boa is a pure-Rust interpreter (no JIT), so it ships under
//! iOS's ban on dynamic code generation — the same reason `outl-exec`'s
//! `lang-js` is allowed on mobile while `lang-rust` (wasmtime/Cranelift)
//! is not.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_plugins::{
    load_installed, plugins_dir, Capability, ClientCapabilities, CommandEntry, PluginHost,
    PluginRun, ToolbarButtonEntry, TransformResult, TransformerEntry,
};
use parking_lot::Mutex;
use serde::Serialize;
use tracing::warn;

use outl_plugins::MarketplaceItem;

/// One plugin command, projected to the wire shape the frontend lists.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginCommandDto {
    pub plugin_id: String,
    pub command_id: String,
    pub title: String,
}

impl From<CommandEntry> for PluginCommandDto {
    fn from(c: CommandEntry) -> Self {
        Self {
            plugin_id: c.plugin_id,
            command_id: c.command_id,
            title: c.title,
        }
    }
}

/// One toolbar button a plugin contributes to the mobile chrome,
/// projected to the wire shape the header renders. `icon` is the glyph
/// painted in the header; tapping it runs `command_id` via `plugin_run`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolbarButtonDto {
    pub plugin_id: String,
    pub command_id: String,
    pub icon: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl From<ToolbarButtonEntry> for ToolbarButtonDto {
    fn from(t: ToolbarButtonEntry) -> Self {
        Self {
            plugin_id: t.plugin_id,
            command_id: t.command_id,
            icon: t.icon,
            title: t.title,
        }
    }
}

/// One content transformer a plugin contributes, projected to the wire
/// shape the frontend lists. The frontend keys on `lang`: when a code
/// fence's language matches, it calls `plugin_transform(plugin_id, lang,
/// body)` and renders per `kind` (`"text"` → markdown/text inline, `"rich"`
/// → sandboxed iframe inline).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformerDto {
    pub plugin_id: String,
    pub lang: String,
    pub kind: String,
}

impl From<TransformerEntry> for TransformerDto {
    fn from(t: TransformerEntry) -> Self {
        Self {
            plugin_id: t.plugin_id,
            lang: t.lang,
            kind: t.kind,
        }
    }
}

/// The descriptor a transformer produced for a fenced block: `kind`
/// (`"text"` or `"rich"`) tells the frontend how to render `content`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformResultDto {
    pub kind: String,
    pub content: String,
}

impl From<TransformResult> for TransformResultDto {
    fn from(r: TransformResult) -> Self {
        Self {
            kind: r.kind,
            content: r.content,
        }
    }
}

/// Outcome of running a plugin command, surfaced to the frontend.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginRunDto {
    /// Number of intents the plugin applied to the workspace.
    pub applied: usize,
    /// `ctx.ui.notify` messages — shown as info toasts.
    pub notifications: Vec<String>,
    /// `ctx.ui.render(html)` payloads (author-written HTML/JS) — each is
    /// dropped into a sandboxed, ephemeral `<iframe>` overlay on the
    /// frontend. Only populated for plugins granted the `ui-render`
    /// capability (the core gates this in `PluginRun`).
    pub views: Vec<String>,
    /// Non-fatal plugin errors — shown as error toasts.
    pub errors: Vec<String>,
}

impl From<PluginRun> for PluginRunDto {
    fn from(r: PluginRun) -> Self {
        Self {
            applied: r.applied,
            notifications: r.notifications,
            views: r.views,
            errors: r.errors,
        }
    }
}

/// A request handed to the plugin thread. Each carries a one-shot reply
/// channel the thread sends the result back on; the caller blocks on the
/// matching `Receiver` (`recv()`), so the Tauri command stays synchronous
/// and never holds the workspace `Mutex` across the wait.
enum PluginRequest {
    /// List every command contributed by a loaded plugin.
    ListCommands {
        reply: Sender<Vec<PluginCommandDto>>,
    },
    /// List every toolbar button a plugin contributes for the mobile
    /// client (`host.toolbar_buttons("mobile")`).
    ListToolbar {
        reply: Sender<Vec<ToolbarButtonDto>>,
    },
    /// List every content transformer granted on this client
    /// (`host.transformers()`). Gated on the client∩plugin capability
    /// intersection — the mobile client declares both
    /// `ContentTransformerText` and `ContentTransformerRich`.
    ListTransformers { reply: Sender<Vec<TransformerDto>> },
    /// Run a plugin's content transformer for `lang` against `input`.
    /// Reply is `Err(String)` on a host-level failure (no such plugin,
    /// engine error); `Ok(None)` when the transformer declined.
    TransformBlock {
        plugin_id: String,
        lang: String,
        input: String,
        reply: Sender<Result<Option<TransformResultDto>, String>>,
    },
    /// Run a plugin command. Reply is `Err(String)` on a host-level
    /// failure (no such plugin, engine error); `Ok` carries the per-run
    /// notifications / errors even when individual intents were denied.
    RunCommand {
        plugin_id: String,
        command_id: String,
        reply: Sender<Result<PluginRunDto, String>>,
    },
    /// Run every plugin's `onOp` hook over ops applied since the last
    /// sweep. Reply carries the number of intents the hooks applied (so
    /// the caller knows whether to re-render) plus any `ctx.ui.render`
    /// payloads the hooks emitted (so the caller can paint them — this is
    /// the confetti path: DONE → hook → render). Best-effort.
    SyncHooks { reply: Sender<SyncHooksOutcome> },
    /// Fetch the official registry and cross-reference it with the
    /// lockfile → the marketplace rows. Network + lockfile read; runs on
    /// this (non-tokio) thread so blocking HTTP is fine.
    RegistryList {
        reply: Sender<Result<Vec<MarketplaceItem>, String>>,
    },
    /// Download + install an official plugin by id (tap-to-install), then
    /// reload the host so it's live. Reply is the installed name.
    InstallOfficial {
        id: String,
        reply: Sender<Result<String, String>>,
    },
    /// Flip a plugin's `enabled` flag in the lockfile, then reload.
    SetEnabled {
        id: String,
        enabled: bool,
        reply: Sender<Result<(), String>>,
    },
    /// Uninstall a plugin (delete its dir + lockfile entry), then reload.
    Uninstall {
        id: String,
        reply: Sender<Result<bool, String>>,
    },
}

/// What a `SyncHooks` sweep returns: how many intents the hooks applied
/// (drives whether the caller re-renders) and any `ctx.ui.render` views
/// they emitted (drives the iframe overlay — e.g. confetti on DONE).
#[derive(Debug, Clone, Default)]
pub(crate) struct SyncHooksOutcome {
    pub applied: usize,
    pub views: Vec<String>,
}

/// `Send + Sync` handle to the plugin thread. Stored in `AppState`.
///
/// Cloneable: every clone shares the same `mpsc::Sender`, so any Tauri
/// command can reach the single plugin thread.
#[derive(Clone)]
pub(crate) struct PluginService {
    tx: Sender<PluginRequest>,
}

impl PluginService {
    /// Spawn the plugin thread and return a handle to it.
    ///
    /// The thread loads every installed plugin from
    /// `<storage_root>/.outl/plugins/` on the first request after the
    /// workspace opens, marks the host synced (so pre-existing ops don't
    /// fire `onOp` at boot), then serves requests until the `Sender` is
    /// dropped.
    ///
    /// `workspace` is the **same** `Arc` `AppState` holds. `storage_root`
    /// is owned (mobile has one root for the process lifetime — no live
    /// swap), so there is no desktop-style re-load branch.
    pub(crate) fn spawn(
        workspace: Arc<Mutex<Option<Workspace>>>,
        storage_root: PathBuf,
        hlc: HlcGenerator,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<PluginRequest>();
        thread::Builder::new()
            .name("outl-plugin-host".into())
            .spawn(move || run_plugin_thread(rx, workspace, storage_root, hlc))
            .expect("spawn plugin host thread");
        Self { tx }
    }

    /// List plugin-contributed commands. Returns an empty vec if the
    /// plugin thread is gone or nothing is loaded yet.
    pub(crate) fn list_commands(&self) -> Vec<PluginCommandDto> {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::ListCommands { reply }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// List plugin-contributed toolbar buttons for the mobile chrome.
    /// Returns an empty vec if the plugin thread is gone or nothing is
    /// loaded yet.
    pub(crate) fn list_toolbar(&self) -> Vec<ToolbarButtonDto> {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::ListToolbar { reply }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// List content transformers granted on this client. Returns an empty
    /// vec if the plugin thread is gone or nothing is loaded yet.
    pub(crate) fn list_transformers(&self) -> Vec<TransformerDto> {
        let (reply, rx) = mpsc::channel();
        if self
            .tx
            .send(PluginRequest::ListTransformers { reply })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// Run a plugin's content transformer for `lang` against `input` and
    /// wait for the descriptor it produced (`None` when it declined).
    pub(crate) fn transform_block(
        &self,
        plugin_id: String,
        lang: String,
        input: String,
    ) -> Result<Option<TransformResultDto>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::TransformBlock {
                plugin_id,
                lang,
                input,
                reply,
            })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Run a plugin command and wait for its result.
    pub(crate) fn run_command(
        &self,
        plugin_id: String,
        command_id: String,
    ) -> Result<PluginRunDto, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::RunCommand {
                plugin_id,
                command_id,
                reply,
            })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Fire the `onOp` hook sweep and return how many intents the hooks
    /// applied (so the caller re-renders only when something changed) plus
    /// any `ctx.ui.render` views the hooks emitted (the confetti path).
    ///
    /// Blocks until the sweep is done so a follow-up page render sees any
    /// hook mutation, but a dead thread is a silent empty outcome (plugins
    /// must never block editing). The caller must NOT hold the workspace
    /// `Mutex` — the plugin thread locks it to run the hooks.
    pub(crate) fn sync_hooks(&self) -> SyncHooksOutcome {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::SyncHooks { reply }).is_err() {
            return SyncHooksOutcome::default();
        }
        rx.recv().unwrap_or_default()
    }

    /// Marketplace rows: the official registry crossed with the lockfile.
    pub(crate) fn registry_list(&self) -> Result<Vec<MarketplaceItem>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::RegistryList { reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Tap-to-install an official plugin by id; returns its display name.
    pub(crate) fn install_official(&self, id: String) -> Result<String, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::InstallOfficial { id, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Enable / disable an installed plugin.
    pub(crate) fn set_enabled(&self, id: String, enabled: bool) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::SetEnabled { id, enabled, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Uninstall a plugin; `true` if anything was removed.
    pub(crate) fn uninstall(&self, id: String) -> Result<bool, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::Uninstall { id, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }
}

/// Capabilities the mobile client honors: slash commands (surfaced in the
/// plugin sheet), op hooks, UI render, and toolbar buttons. Mirrors the
/// desktop's set minus keybinding (chords aren't a mobile surface — no
/// hardware keyboard / chord input).
fn client_capabilities() -> ClientCapabilities {
    // UiRender: the mobile webview can run a plugin's `ctx.ui.render` HTML in a
    // sandboxed iframe; without it the host gates those views out.
    // ToolbarButton: the header renders a glyph per contributed button; without
    // it `host.toolbar_buttons("mobile")` returns nothing (the cap is gated on
    // the granted set, which is the client∩plugin intersection).
    // ContentTransformerText / ContentTransformerRich: the mobile webview can
    // render a transformer's `text` (markdown/text) and `rich` (HTML in a
    // sandboxed iframe) output. WITHOUT both, `host.transformers()` filters the
    // entries out (the cap is gated on the client∩plugin intersection), so a
    // fenced block in a custom language renders as plain code instead of the
    // transformed view.
    [
        Capability::SlashCommand,
        Capability::OpHook,
        Capability::UiRender,
        Capability::ToolbarButton,
        Capability::ContentTransformerText,
        Capability::ContentTransformerRich,
    ]
    .into_iter()
    .collect()
}

/// The plugin thread's event loop. Owns the `PluginHost` (and thus the
/// Boa `Context`) for its entire lifetime — nothing here ever moves the
/// host across a thread boundary.
fn run_plugin_thread(
    rx: Receiver<PluginRequest>,
    workspace: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
) {
    let mut host = PluginHost::new(client_capabilities());
    // `false` until the first successful load once the workspace opens.
    // Unlike the desktop, the root never changes for the process, so a
    // single bool guards the one-time load.
    let mut loaded = false;

    while let Ok(req) = rx.recv() {
        match req {
            PluginRequest::ListCommands { reply } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let cmds = host.commands().into_iter().map(Into::into).collect();
                let _ = reply.send(cmds);
            }
            PluginRequest::ListToolbar { reply } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let buttons = host
                    .toolbar_buttons("mobile")
                    .into_iter()
                    .map(Into::into)
                    .collect();
                let _ = reply.send(buttons);
            }
            PluginRequest::ListTransformers { reply } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let transformers = host.transformers().into_iter().map(Into::into).collect();
                let _ = reply.send(transformers);
            }
            PluginRequest::TransformBlock {
                plugin_id,
                lang,
                input,
                reply,
            } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let result = host
                    .transform_block(&plugin_id, &lang, &input)
                    .map(|opt| opt.map(Into::into))
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            PluginRequest::RunCommand {
                plugin_id,
                command_id,
                reply,
            } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let result = run_command(
                    &mut host,
                    &workspace,
                    &storage_root,
                    &hlc,
                    &plugin_id,
                    &command_id,
                );
                let _ = reply.send(result);
            }
            PluginRequest::SyncHooks { reply } => {
                ensure_loaded(&mut host, &workspace, &storage_root, &mut loaded);
                let outcome = run_sync_hooks(&mut host, &workspace, &storage_root, &hlc);
                let _ = reply.send(outcome);
            }
            PluginRequest::RegistryList { reply } => {
                let _ = reply.send(registry_list(&storage_root));
            }
            PluginRequest::InstallOfficial { id, reply } => {
                let result = install_official(&storage_root, &hlc, &id);
                // Force a reload so the freshly-installed plugin is live on
                // the next request (op-hooks, commands, transformers).
                if result.is_ok() {
                    loaded = false;
                }
                let _ = reply.send(result);
            }
            PluginRequest::SetEnabled { id, enabled, reply } => {
                let result = set_enabled(&storage_root, &id, enabled);
                if result.is_ok() {
                    loaded = false;
                }
                let _ = reply.send(result);
            }
            PluginRequest::Uninstall { id, reply } => {
                let result = uninstall_plugin(&storage_root, &id);
                if result.is_ok() {
                    loaded = false;
                }
                let _ = reply.send(result);
            }
        }
    }
}

// Thin bridges to the shared marketplace API in `outl-plugins` (one owner,
// desktop wraps it too). Mobile's root is an owned `PathBuf`, so these only
// map the error to a String for the reply channel.
fn registry_list(root: &Path) -> Result<Vec<MarketplaceItem>, String> {
    outl_plugins::marketplace_list(root).map_err(|e| e.to_string())
}

fn install_official(root: &Path, hlc: &HlcGenerator, id: &str) -> Result<String, String> {
    outl_plugins::marketplace_install(root, &hlc.actor(), id).map_err(|e| e.to_string())
}

fn set_enabled(root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    outl_plugins::set_enabled(root, id, enabled).map_err(|e| e.to_string())
}

fn uninstall_plugin(root: &Path, id: &str) -> Result<bool, String> {
    outl_plugins::uninstall(&plugins_dir(root), id).map_err(|e| e.to_string())
}

/// Load installed plugins the first time the workspace is available.
///
/// Best-effort: a workspace not yet open (background opener still running)
/// leaves the host empty and tries again on the next request. A per-plugin
/// load failure is logged but never blocks the others. Loads exactly once
/// — the mobile root is fixed for the process lifetime.
fn ensure_loaded(
    host: &mut PluginHost,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &Path,
    loaded: &mut bool,
) {
    if *loaded {
        return;
    }
    // Don't load until the workspace is actually open — `mark_synced`
    // needs the live op log so pre-existing ops don't fire `onOp` at boot.
    let guard = workspace.lock();
    let Some(ws) = guard.as_ref() else {
        return; // opener still running — retry on the next request
    };

    let report = load_installed(host, &plugins_dir(storage_root));
    for (id, err) in &report.failed {
        warn!("plugin {id} failed to load: {err}");
    }
    if !report.loaded.is_empty() {
        tracing::info!("loaded {} plugin(s)", report.loaded.len());
    }

    // Mark synced against the live log so pre-existing ops don't fire
    // `onOp` hooks at boot — only ops produced *after* this point should.
    host.mark_synced(ws);
    *loaded = true;
}

/// Run a plugin command under the workspace lock, then re-project `.md`
/// if it mutated anything.
fn run_command(
    host: &mut PluginHost,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &Path,
    hlc: &HlcGenerator,
    plugin_id: &str,
    command_id: &str,
) -> Result<PluginRunDto, String> {
    let mut guard = workspace.lock();
    let Some(ws) = guard.as_mut() else {
        return Err("workspace_loading".to_string());
    };
    let run = host
        .run_command(ws, hlc, plugin_id, command_id)
        .map_err(|e| e.to_string())?;
    if run.applied > 0 {
        reproject(ws, storage_root);
    }
    Ok(run.into())
}

/// Dispatch the `onOp` sweep under the workspace lock, re-projecting if a
/// hook mutated the workspace. Returns how many intents were applied plus
/// any `ctx.ui.render` views the hooks emitted (the confetti path).
fn run_sync_hooks(
    host: &mut PluginHost,
    workspace: &Arc<Mutex<Option<Workspace>>>,
    storage_root: &Path,
    hlc: &HlcGenerator,
) -> SyncHooksOutcome {
    let mut guard = workspace.lock();
    let Some(ws) = guard.as_mut() else {
        return SyncHooksOutcome::default();
    };
    match host.sync_hooks(ws, hlc) {
        Ok(run) => {
            if run.applied > 0 {
                reproject(ws, storage_root);
            }
            SyncHooksOutcome {
                applied: run.applied,
                views: run.views,
            }
        }
        Err(e) => {
            warn!("plugin op-hook sweep failed: {e}");
            SyncHooksOutcome::default()
        }
    }
}

/// Re-project every page's `.md` after a plugin mutated the in-memory
/// workspace.
///
/// A plugin's intents land in the op log via `outl-actions` →
/// `Workspace::apply`, but **don't** touch the `.md` projection on their
/// own. A plugin can mutate any page (`archive-done` moves blocks to a
/// *different* page), so we render every page from the workspace
/// (`apply_all_pages_md`). Best-effort: a failure is logged, the op log
/// is still the source of truth.
fn reproject(ws: &Workspace, storage_root: &Path) {
    if let Err(e) = outl_actions::apply_all_pages_md(ws, storage_root) {
        warn!("re-projecting .md after plugin mutation failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_actions::{append_block, open_or_create_page, PageKind};
    use outl_core::id::ActorId;
    use tempfile::TempDir;

    #[test]
    fn client_declares_ui_render_so_views_are_not_gated_out() {
        // The host only forwards `ctx.ui.render` payloads for plugins granted
        // `ui-render`, which requires this client to declare it. Dropping it
        // silently breaks the confetti / overlay path.
        assert!(client_capabilities().contains(&Capability::UiRender));
    }

    #[test]
    fn client_declares_both_content_transformer_caps() {
        // The host gates `transformers()` on the client∩plugin intersection.
        // Dropping either capability silently filters that kind of transformer
        // out, so a fenced block in a custom language renders as plain code.
        let caps = client_capabilities();
        assert!(caps.contains(&Capability::ContentTransformerText));
        assert!(caps.contains(&Capability::ContentTransformerRich));
    }

    /// A dev-mode plugin (no lockfile — `_dev/` grants permissions
    /// implicitly) that contributes a slash command and an `onOp` hook.
    const BUNDLE: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('say-hi', () => {
                    ctx.ui.notify('hi from plugin');
                });
                ctx.ops.onOp((op) => {
                    if (op.kind === 'Edit') {
                        ctx.log.info('saw edit on ' + op.node);
                    }
                });
                ctx.content.register('upper', (text) => ({
                    kind: 'text',
                    content: text.toUpperCase(),
                }));
            }
        });
    "#;

    fn write_dev_plugin(root: &Path) {
        let dir = root.join(".outl/plugins/_dev/hello");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            br#"{
                "id": "run.avelino.hello",
                "name": "Hello",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command", "op-hook", "toolbar-button", "content-transformer:text"],
                "permissions": ["read-page", "write-page", "submit-op", "read-op-log"],
                "contributes": {
                    "commands": [{ "id": "say-hi", "title": "Say hi" }],
                    "toolbar": [{ "command": "say-hi", "icon": "*", "title": "Wave" }],
                    "transformers": [{ "lang": "upper", "kind": "text" }]
                }
            }"#,
        )
        .unwrap();
        std::fs::write(dir.join("index.js"), BUNDLE).unwrap();
    }

    /// The shared slots `AppState` holds, returned by [`slots`]. Mobile's
    /// `storage_root` is an owned `PathBuf` (not an `Arc<Mutex<...>>`).
    type Slots = (Arc<Mutex<Option<Workspace>>>, PathBuf, HlcGenerator);

    /// Build the shared slots the way `AppState` does: an open, persisted
    /// workspace rooted under `root` with one page + block.
    fn slots(root: &Path) -> Slots {
        std::fs::create_dir_all(root.join("ops")).unwrap();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let storage = outl_core::storage::JsonlStorage::open(root.join("ops"), actor).unwrap();
        let mut ws =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();
        let page = open_or_create_page(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("x")).unwrap();
        (Arc::new(Mutex::new(Some(ws))), root.to_path_buf(), hlc)
    }

    #[test]
    fn lists_dev_plugin_command() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let cmds = svc.list_commands();
        assert!(
            cmds.iter().any(|c| c.title == "Say hi"),
            "plugin command missing: {cmds:?}"
        );
    }

    #[test]
    fn lists_dev_plugin_toolbar_button() {
        // The dev plugin declares `toolbar-button` + a toolbar contribution
        // bound to `say-hi`. The client declares `ToolbarButton`, so the
        // intersection grants it and `toolbar_buttons("mobile")` surfaces it.
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let buttons = svc.list_toolbar();
        let btn = buttons
            .iter()
            .find(|b| b.command_id == "say-hi")
            .unwrap_or_else(|| panic!("toolbar button missing: {buttons:?}"));
        assert_eq!(btn.icon, "*");
        assert_eq!(btn.title.as_deref(), Some("Wave"));
        assert_eq!(btn.plugin_id, "run.avelino.hello");
    }

    #[test]
    fn lists_dev_plugin_transformer() {
        // The dev plugin declares `content-transformer:text` + a transformer
        // for `upper`. The client declares both transformer caps, so the
        // intersection grants it and `host.transformers()` surfaces it.
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let transformers = svc.list_transformers();
        let t = transformers
            .iter()
            .find(|t| t.lang == "upper")
            .unwrap_or_else(|| panic!("transformer missing: {transformers:?}"));
        assert_eq!(t.kind, "text");
        assert_eq!(t.plugin_id, "run.avelino.hello");
    }

    #[test]
    fn transform_block_returns_descriptor() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let out = svc
            .transform_block("run.avelino.hello".into(), "upper".into(), "hello".into())
            .expect("transform runs")
            .expect("transformer produced a descriptor");
        assert_eq!(out.kind, "text");
        assert_eq!(out.content, "HELLO");
    }

    #[test]
    fn transform_block_unknown_lang_is_none() {
        // A transformer that has no handler for the language declines with
        // `None` — the frontend then falls back to the plain code fence.
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let out = svc
            .transform_block("run.avelino.hello".into(), "mermaid".into(), "x".into())
            .expect("transform runs");
        assert!(out.is_none(), "expected decline, got {out:?}");
    }

    #[test]
    fn run_command_surfaces_notification() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        let run = svc
            .run_command("run.avelino.hello".into(), "say-hi".into())
            .expect("command runs");
        assert!(
            run.notifications.iter().any(|n| n == "hi from plugin"),
            "notes: {:?}",
            run.notifications
        );
    }

    #[test]
    fn run_unknown_command_is_a_clean_error() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path());
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);

        // Either an Err or an Ok with errors recorded — never a panic /
        // dead thread. Re-run to prove the thread is still alive.
        let _ = svc.run_command("run.avelino.hello".into(), "nope".into());
        assert!(!svc.list_commands().is_empty(), "thread still serving");
    }

    #[test]
    fn sync_hooks_never_panics_with_no_plugins() {
        let dir = TempDir::new().unwrap();
        // No plugin written — empty host.
        let (ws, root, hlc) = slots(dir.path());
        let svc = PluginService::spawn(ws, root, hlc);
        svc.sync_hooks();
        // Thread is still alive afterwards.
        assert!(svc.list_commands().is_empty());
    }
}
