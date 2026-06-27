//! Plugin command surface.
//!
//! Thin shims over [`crate::plugin_service::PluginService`] — the Boa
//! `Context` lives on a dedicated thread (`!Send`), so these commands
//! only talk to it through the request channel. No business logic here:
//! listing, running, and re-projection all happen on the plugin thread
//! against the shared `outl-actions` surface.

use tauri::State;

use crate::helpers::{build_page_view, parse_node_id, storage_root_or_err, with_ws};
use crate::plugin_dto::{
    PluginCommandDto, PluginKeybindingDto, ToolbarButtonDto, TransformResultDto, TransformerDto,
};
use crate::plugin_service::PluginService;
use crate::state::{AppState, PageView};
use outl_plugins::MarketplaceItem;
use serde::Serialize;

/// Reply for [`plugin_run`]: the plugin's notifications / errors / applied
/// count, plus the refreshed [`PageView`] of the page that was on screen
/// when the command fired (so the frontend can re-render in one trip).
///
/// `view` is `None` when no page id was supplied (e.g. the palette fired
/// before a page loaded) or the page no longer resolves — a plugin can
/// move blocks off the current page, but the page node itself stays.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginRunReply {
    pub applied: usize,
    pub notifications: Vec<String>,
    pub errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    /// HTML documents the plugin emitted via `ctx.ui.render` (gated by
    /// the `ui-render` capability). The frontend plays each as an
    /// ephemeral sandboxed iframe overlay — untrusted plugin output.
    pub views: Vec<String>,
}

/// Reply for [`plugin_sync_hooks`]: the refreshed [`PageView`] when an
/// op-hook mutated the page on screen (`None` on the common no-op path),
/// plus any `ui-render` views the hooks emitted. The views path is the
/// confetti trigger: toggling a block DONE fires `onOp`, the plugin emits
/// HTML, and the frontend plays it as a sandboxed iframe overlay — so it
/// is populated even when no page re-render is needed.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginSyncHooksReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    pub views: Vec<String>,
}

/// List every command contributed by a loaded plugin whose
/// `slash-command` capability the desktop honors. Empty until the
/// workspace opens and plugins load (best-effort).
#[tauri::command]
pub(crate) fn plugin_list(plugins: State<'_, PluginService>) -> Vec<PluginCommandDto> {
    plugins.list_commands()
}

/// List every keybinding a loaded plugin contributed for the desktop
/// surface. The frontend folds these into its chord dispatcher as a
/// Global overlay — a plugin chord fires `plugin_run` only when no native
/// `outl-shortcuts` binding already owns that chord (native wins). Empty
/// until plugins load (best-effort).
#[tauri::command]
pub(crate) fn plugin_keybindings(plugins: State<'_, PluginService>) -> Vec<PluginKeybindingDto> {
    plugins.list_keybindings()
}

/// List every toolbar button a loaded plugin contributed for the desktop
/// surface. The frontend renders one chrome button per entry (glyph =
/// `icon`, tooltip = `title`, click = `plugin_run`). Empty until plugins
/// load (best-effort).
#[tauri::command]
pub(crate) fn plugin_toolbar(plugins: State<'_, PluginService>) -> Vec<ToolbarButtonDto> {
    plugins.list_toolbar()
}

/// List every content transformer a loaded plugin declared for a
/// code-fence language (gated by `content-transformer:text` / `:rich`).
/// The frontend loads this once per workspace open and renders any fence
/// whose language matches through [`plugin_transform`]. Empty until plugins
/// load (best-effort).
#[tauri::command]
pub(crate) fn plugin_transformers(plugins: State<'_, PluginService>) -> Vec<TransformerDto> {
    plugins.list_transformers()
}

/// Run a content transformer for `lang` against a fence `input` (its body).
///
/// Read-only: the transformer renders the body and never mutates the
/// workspace, so there is no re-projection and no on-screen page to refresh.
/// Returns `None` when the transformer declined or no plugin owns `lang`;
/// `Some({kind, content})` otherwise — `kind` is `"text"` (inline markdown)
/// or `"rich"` (HTML for a sandboxed iframe).
#[tauri::command]
pub(crate) fn plugin_transform(
    plugin_id: String,
    lang: String,
    input: String,
    plugins: State<'_, PluginService>,
) -> Result<Option<TransformResultDto>, String> {
    plugins.transform_block(plugin_id, lang, input)
}

/// Run a plugin command. `page_id` is the page currently on screen; the
/// reply carries its refreshed [`PageView`] so the frontend re-renders
/// without a second round-trip. A plugin can mutate any page — the whole
/// workspace was re-projected on the plugin thread before this returns —
/// but the view we return is the one the user is looking at.
#[tauri::command]
pub(crate) fn plugin_run(
    plugin_id: String,
    command_id: String,
    page_id: Option<String>,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<PluginRunReply, String> {
    let run = plugins.run_command(plugin_id, command_id)?;

    // Re-render the on-screen page from the now-mutated + re-projected
    // workspace. Failure to resolve the page is non-fatal: the mutation
    // already landed, the frontend can fall back to its own reload.
    let view = match page_id {
        Some(id) => {
            let page = parse_node_id(&id)?;
            let root = storage_root_or_err(&state)?;
            with_ws(&state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None)
        }
        None => None,
    };

    Ok(PluginRunReply {
        applied: run.applied,
        notifications: run.notifications,
        errors: run.errors,
        view,
        views: run.views,
    })
}

/// Fire the plugins' `onOp` hook sweep after a user mutation. Best-effort
/// op-hooks (root invariant 7's "any state that converges goes through the
/// op log" applies to the *plugin's* writes too — they route through
/// `outl-actions`).
///
/// The frontend calls this once after committing a block mutation. It must
/// be called with **no** workspace lock held by the webview side; the
/// plugin thread locks the workspace to run the hooks. Returns the
/// refreshed [`PageView`] of `page_id` **only** when a hook actually
/// mutated the workspace (so the caller skips a needless re-render on the
/// common no-op path).
#[tauri::command]
pub(crate) fn plugin_sync_hooks(
    page_id: Option<String>,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<PluginSyncHooksReply, String> {
    let outcome = plugins.sync_hooks();
    // Views ride back even on the no-op-mutation path: a `ui-render`
    // plugin (confetti) emits HTML from its `onOp` hook without
    // necessarily mutating the workspace, so the overlay must still
    // play. Only the page re-render is gated on `applied > 0`.
    let view = if outcome.applied > 0 {
        match page_id {
            Some(id) => {
                let page = parse_node_id(&id)?;
                let root = storage_root_or_err(&state)?;
                with_ws(&state, |ws| Ok(build_page_view(ws, &root, page).ok()))?
            }
            None => None,
        }
    } else {
        None
    };
    Ok(PluginSyncHooksReply {
        view,
        views: outcome.views,
    })
}

/// Marketplace rows: the official registry (`plugins.outl.app`) crossed with
/// this workspace's lockfile (installed / enabled flags). Network + lockfile
/// read on the plugin thread.
#[tauri::command]
pub(crate) fn plugin_registry_list(
    plugins: State<'_, PluginService>,
) -> Result<Vec<MarketplaceItem>, String> {
    plugins.registry_list()
}

/// Tap-to-install an official plugin by id. Downloads + installs + reloads
/// the host; returns the installed display name.
#[tauri::command]
pub(crate) fn plugin_install_official(
    id: String,
    plugins: State<'_, PluginService>,
) -> Result<String, String> {
    plugins.install_official(id)
}

/// Enable / disable an installed plugin (lockfile flag), then reload.
#[tauri::command]
pub(crate) fn plugin_set_enabled(
    id: String,
    enabled: bool,
    plugins: State<'_, PluginService>,
) -> Result<(), String> {
    plugins.set_enabled(id, enabled)
}

/// Uninstall a plugin (delete its dir + lockfile entry), then reload.
#[tauri::command]
pub(crate) fn plugin_uninstall(
    id: String,
    plugins: State<'_, PluginService>,
) -> Result<bool, String> {
    plugins.uninstall(id)
}
