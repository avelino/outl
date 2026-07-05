//! Mobile plugin-service shim.
//!
//! The thread / channel machinery, the request protocol, and the DTOs
//! all live in `outl-tauri-shared` (`PluginService`, `plugin_dto`); this
//! module keeps only what identifies the mobile client:
//!
//! - the `CLIENT` id the host filters toolbar contributions by, and
//! - the capability set — the desktop's minus `keybinding` (chords
//!   aren't a mobile surface: no hardware keyboard / chord input).
//!
//! The mobile `storage_root` is an owned `PathBuf` (folder swap is a
//! relaunch — see `state.rs`), so it goes straight into
//! `PluginService::spawn` as the fixed-root `StorageRootProvider`: the
//! plugin thread loads plugins exactly once, lazily, on the first
//! request after the workspace opens.
//!
//! iOS note: Boa is a pure-Rust interpreter (no JIT), so it ships under
//! iOS's ban on dynamic code generation — the same reason `outl-exec`'s
//! `lang-js` is allowed on mobile while `lang-rust` (wasmtime/Cranelift)
//! is not.

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_plugins::{Capability, ClientCapabilities};
use parking_lot::Mutex;

pub(crate) use outl_tauri_shared::PluginService;

/// The client id the host filters plugin contributions by.
const CLIENT: &str = "mobile";

/// Capabilities the mobile client honors: slash commands (surfaced in
/// the plugin sheet), op hooks, UI render, toolbar buttons, and both
/// content-transformer kinds. Mirrors the desktop's set minus
/// `keybinding`.
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

/// Spawn the shared plugin thread with the mobile client id, capability
/// set, and fixed storage root. Called once in `lib.rs::setup`; the
/// returned handle goes into Tauri's managed state.
pub(crate) fn spawn_plugin_service(
    workspace: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
) -> PluginService {
    PluginService::spawn(CLIENT, client_capabilities, workspace, storage_root, hlc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_actions::{append_block, open_or_create_page, PageKind};
    use outl_core::id::ActorId;
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
        let svc = spawn_plugin_service(ws, root, hlc);

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
        let svc = spawn_plugin_service(ws, root, hlc);

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
        let svc = spawn_plugin_service(ws, root, hlc);

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
        let svc = spawn_plugin_service(ws, root, hlc);

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
        let svc = spawn_plugin_service(ws, root, hlc);

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
        let svc = spawn_plugin_service(ws, root, hlc);
        svc.sync_hooks();
        // Thread is still alive afterwards.
        assert!(svc.list_commands().is_empty());
    }
}
