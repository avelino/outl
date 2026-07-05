//! Tests for the plugin service. Lives in a sibling file (included via
//! `#[path]` from `plugin_service.rs`) to keep that module under the
//! file-size guard's threshold; it is still the in-crate `tests` child
//! module, so `super::*` resolves the private thread/channel items it
//! exercises.

use super::*;
use outl_actions::{append_block, open_or_create_page, PageKind};
use outl_core::id::ActorId;
use outl_shortcuts::Mode;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn client_declares_ui_render_so_views_are_not_gated_out() {
    // The host only forwards `ctx.ui.render` payloads for plugins granted
    // `ui-render`, which requires this client to declare it. Dropping it
    // silently breaks the confetti / overlay path.
    assert!(client_capabilities().contains(&Capability::UiRender));
}

#[test]
fn client_declares_content_transformer_caps_so_transformers_are_not_gated_out() {
    // The host only lists a transformer for a client that declares the
    // matching capability (`content-transformer:text` / `:rich`).
    // Dropping either silently breaks inline fence rendering for that
    // kind.
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
        }
    });
"#;

fn write_dev_plugin(root: &Path) {
    let dir = root.join(".outl/plugins/_dev/hello");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{
            "id": "run.avelino.hello",
            "name": "Hello",
            "version": "1.0.0",
            "api": "^1.0",
            "main": "index.js",
            "capabilities": ["slash-command", "op-hook", "keybinding", "toolbar-button"],
            "permissions": ["read-page", "write-page", "submit-op", "read-op-log"],
            "contributes": {
                "commands": [{ "id": "say-hi", "title": "Say hi" }],
                "keybindings": [{ "command": "say-hi", "key": "Ctrl+Shift+H" }],
                "toolbar": [{ "command": "say-hi", "icon": "👋", "title": "Wave" }]
            }
        }"#,
    )
    .unwrap();
    std::fs::write(dir.join("index.js"), BUNDLE).unwrap();
}

/// The shared slots `AppState` holds, returned by [`slots`].
type Slots = (
    Arc<Mutex<Option<Workspace>>>,
    Arc<Mutex<Option<PathBuf>>>,
    HlcGenerator,
);

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
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let page = open_or_create_page(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("x")).unwrap();
    (
        Arc::new(Mutex::new(Some(ws))),
        Arc::new(Mutex::new(Some(root.to_path_buf()))),
        hlc,
    )
}

#[test]
fn lists_dev_plugin_command() {
    let dir = TempDir::new().unwrap();
    write_dev_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    let cmds = svc.list_commands();
    assert!(
        cmds.iter().any(|c| c.title == "Say hi"),
        "plugin command missing: {cmds:?}"
    );
}

#[test]
fn lists_installed_plugin_command() {
    // The *installed* path (lockfile + bundleHash + capability
    // intersection), not the relaxed `_dev/` loader: this is what
    // `outl plugin install` produces and what `plugin_list` must surface
    // in the desktop palette. Regression guard for the "installed a
    // plugin, no command showed up in the desktop" report.
    let src = TempDir::new().unwrap();
    std::fs::write(
        src.path().join("plugin.json"),
        r#"{
            "id": "app.outl.examples.stats",
            "name": "Stats",
            "version": "1.0.0",
            "api": "^1.0",
            "main": "index.js",
            "capabilities": ["slash-command"],
            "permissions": ["read-page"],
            "contributes": { "commands": [{ "id": "stats", "title": "Workspace statistics" }] }
        }"#,
    )
    .unwrap();
    std::fs::write(
        src.path().join("index.js"),
        r#"globalThis.__outl_register({ activate(ctx) { ctx.commands.register('stats', () => {}); } });"#,
    )
    .unwrap();

    let dir = TempDir::new().unwrap();
    let (ws, root, hlc) = slots(dir.path());
    // Install into the same `.outl/plugins/` the service loads from.
    let plugins = outl_plugins::plugins_dir(dir.path());
    std::fs::create_dir_all(&plugins).unwrap();
    outl_plugins::install_from_dir(
        &plugins,
        src.path(),
        "local:./stats",
        vec![outl_plugins::Permission::ReadPage],
        None,
    )
    .unwrap();

    let svc = spawn_plugin_service(ws, root, hlc);
    let cmds = svc.list_commands();
    assert!(
        cmds.iter().any(|c| c.title == "Workspace statistics"),
        "installed plugin command missing from palette: {cmds:?}"
    );
}

#[test]
fn lists_dev_plugin_keybinding() {
    let dir = TempDir::new().unwrap();
    write_dev_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    let binds = svc.list_keybindings();
    let kb = binds
        .iter()
        .find(|b| b.command_id == "say-hi")
        .unwrap_or_else(|| panic!("plugin keybinding missing: {binds:?}"));
    // Plugin chords are always Global, and the description falls back
    // to the command's title.
    assert_eq!(kb.mode, Mode::Global);
    assert_eq!(kb.plugin_id, "run.avelino.hello");
    assert_eq!(kb.description, "Say hi");
    // The chord serializes to the same shape the frontend's
    // `eventToChord` produces (`Ctrl+Shift+H` → mods + Char('h')).
    let json = serde_json::to_value(kb).unwrap();
    assert!(
        json["chord"].is_array(),
        "chord must serialize transparently as an array: {json}"
    );
}

#[test]
fn lists_dev_plugin_toolbar_button() {
    let dir = TempDir::new().unwrap();
    write_dev_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    let buttons = svc.list_toolbar();
    let tb = buttons
        .iter()
        .find(|b| b.command_id == "say-hi")
        .unwrap_or_else(|| panic!("plugin toolbar button missing: {buttons:?}"));
    assert_eq!(tb.icon, "👋");
    assert_eq!(tb.title.as_deref(), Some("Wave"));
    assert_eq!(tb.plugin_id, "run.avelino.hello");
}

#[test]
fn run_command_surfaces_notification() {
    let dir = TempDir::new().unwrap();
    write_dev_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

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
    let svc = spawn_plugin_service(ws, root, hlc);

    let err = svc
        .run_command("run.avelino.hello".into(), "nope".into())
        .map(|_| ())
        .or_else(|e| if e.is_empty() { Err(e) } else { Ok(()) });
    // Either an Err or an Ok with errors recorded — never a panic /
    // dead thread. Re-run to prove the thread is still alive.
    let _ = err;
    assert!(!svc.list_commands().is_empty(), "thread still serving");
}

#[test]
fn sync_hooks_never_panics_with_no_plugins() {
    let dir = TempDir::new().unwrap();
    // No plugin written — empty host.
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);
    svc.sync_hooks();
    // Thread is still alive afterwards.
    assert!(svc.list_commands().is_empty());
}

/// A dev-mode plugin that declares a `text` content transformer for the
/// `upper` fence language (uppercases the body).
const XFORM_BUNDLE: &str = r#"
    globalThis.__outl_register({ activate(ctx) {
        ctx.content.register('upper', (text) => ({ kind: 'text', content: text.toUpperCase() }));
    }});
"#;

fn write_transformer_plugin(root: &Path) {
    let dir = root.join(".outl/plugins/_dev/upper");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        r#"{
            "id": "run.avelino.upper",
            "name": "Upper",
            "version": "1.0.0",
            "api": "^1.0",
            "main": "index.js",
            "capabilities": ["content-transformer:text"],
            "permissions": [],
            "contributes": {
                "transformers": [{ "lang": "upper", "kind": "text" }]
            }
        }"#,
    )
    .unwrap();
    std::fs::write(dir.join("index.js"), XFORM_BUNDLE).unwrap();
}

#[test]
fn lists_dev_plugin_transformer() {
    let dir = TempDir::new().unwrap();
    write_transformer_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    let transformers = svc.list_transformers();
    let t = transformers
        .iter()
        .find(|t| t.lang == "upper")
        .unwrap_or_else(|| panic!("transformer missing: {transformers:?}"));
    assert_eq!(t.kind, "text");
    assert_eq!(t.plugin_id, "run.avelino.upper");
}

#[test]
fn transform_block_runs_text_transformer() {
    let dir = TempDir::new().unwrap();
    write_transformer_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    let out = svc
        .transform_block("run.avelino.upper".into(), "upper".into(), "hello".into())
        .expect("transform runs")
        .expect("transformer produced a result");
    assert_eq!(out.kind, "text");
    assert_eq!(out.content, "HELLO");
}

#[test]
fn transform_block_unknown_lang_is_none() {
    let dir = TempDir::new().unwrap();
    write_transformer_plugin(dir.path());
    let (ws, root, hlc) = slots(dir.path());
    let svc = spawn_plugin_service(ws, root, hlc);

    // The plugin owns `upper`, not `mermaid` — declined → Ok(None).
    let out = svc
        .transform_block("run.avelino.upper".into(), "mermaid".into(), "x".into())
        .expect("transform call does not error");
    assert!(out.is_none(), "unknown lang should decline: {out:?}");
    // Thread still alive.
    assert!(!svc.list_transformers().is_empty());
}
