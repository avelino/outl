//! Desktop plugin-service shim.
//!
//! The thread / channel machinery, the request protocol, and the DTOs
//! all live in `outl-tauri-shared` (`PluginService`, `plugin_dto`); this
//! module keeps only what identifies the desktop client:
//!
//! - the `CLIENT` id the host filters keybinding / toolbar
//!   contributions by, and
//! - the capability set (`keybinding` is desktop-only — mobile has no
//!   chord surface).
//!
//! The desktop's swap-capable `Arc<Mutex<Option<PathBuf>>>` storage root
//! goes straight into `PluginService::spawn` — it implements the shared
//! `StorageRootProvider`, so the plugin thread reloads the host against
//! the new root after a workspace swap. See the shared crate for the
//! full design (dedicated thread because Boa's `Context` is `!Send`,
//! lazy load gated on the open workspace, `apply_all_pages_md`
//! re-projection after a mutation).

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_plugins::{Capability, ClientCapabilities};
use parking_lot::Mutex;

pub(crate) use outl_tauri_shared::PluginService;

/// The client id the host filters plugin contributions by. Plugins
/// declare which surfaces a keybinding / toolbar button targets; the
/// desktop only wants the ones aimed at `"desktop"`.
const CLIENT: &str = "desktop";

/// Capabilities the desktop honors: slash commands (command palette),
/// op hooks, sandboxed `ui-render`, plugin-contributed `keybinding` +
/// `toolbar-button` surfaces, and both content-transformer kinds.
fn client_capabilities() -> ClientCapabilities {
    // UiRender: the desktop has a webview, so it can run a plugin's
    // `ctx.ui.render` HTML in a sandboxed iframe. Without it the host gates
    // those views out before they ever reach the frontend.
    //
    // Keybinding / ToolbarButton: the host filters these contributions by
    // declared capability before `keybindings(CLIENT)` / `toolbar_buttons(CLIENT)`
    // return anything, so without them the desktop would always see an
    // empty list. The frontend wires chords through `outl-shortcuts` for
    // native bindings and treats plugin chords as a Global overlay that
    // never steals a chord a native binding already owns.
    // ContentTransformerText / ContentTransformerRich: the desktop renders
    // a `text` transformer's output through its markdown renderer and a
    // `rich` transformer's HTML in a sandboxed iframe (same isolation as
    // `ui-render`). The host filters transformer contributions by declared
    // capability before `transformers()` returns anything, so without these
    // the desktop would always see an empty list.
    [
        Capability::SlashCommand,
        Capability::OpHook,
        Capability::UiRender,
        Capability::Keybinding,
        Capability::ToolbarButton,
        Capability::ContentTransformerText,
        Capability::ContentTransformerRich,
    ]
    .into_iter()
    .collect()
}

/// Spawn the shared plugin thread with the desktop's client id,
/// capability set, and swap-capable storage root. Called once in
/// `lib.rs::setup`; the returned handle goes into Tauri's managed state.
pub(crate) fn spawn_plugin_service(
    workspace: Arc<Mutex<Option<Workspace>>>,
    storage_root: Arc<Mutex<Option<PathBuf>>>,
    hlc: HlcGenerator,
) -> PluginService {
    PluginService::spawn(CLIENT, client_capabilities, workspace, storage_root, hlc)
}

#[cfg(test)]
#[path = "plugin_service_tests.rs"]
mod tests;
