//! Plugin command surface.
//!
//! Thin shims over [`crate::plugin_service::PluginService`] — the Boa
//! `Context` lives on a dedicated thread (`!Send`), so these commands
//! only talk to it through the request channel. No business logic here:
//! listing, running, and re-projection all happen on the plugin thread
//! against the shared `outl-actions` surface. Mirrors
//! `outl-desktop/src-tauri/src/commands/plugin.rs` 1:1.

use tauri::State;

use crate::helpers::{build_page_view, parse_node_id, with_ws};
use crate::plugin_service::{
    PluginCommandDto, PluginService, ToolbarButtonDto, TransformResultDto, TransformerDto,
};
use crate::state::{AppState, PageView};
use outl_plugins::MarketplaceItem;
use serde::Serialize;

/// Reply for [`plugin_run`]: the plugin's notifications / errors / applied
/// count, plus the refreshed [`PageView`] of the page that was on screen
/// when the command fired (so the frontend can re-render in one trip).
///
/// `view` is `None` when no page id was supplied (e.g. the sheet fired
/// before a page loaded) or the page no longer resolves — a plugin can
/// move blocks off the current page, but the page node itself stays.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginRunReply {
    pub applied: usize,
    pub notifications: Vec<String>,
    pub errors: Vec<String>,
    /// `ctx.ui.render(html)` payloads — each is dropped into a sandboxed,
    /// ephemeral `<iframe>` overlay on the frontend. Empty unless the
    /// plugin holds the `ui-render` capability.
    pub views: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
}

/// Reply for [`plugin_sync_hooks`]: any `ctx.ui.render` views the `onOp`
/// hooks emitted (the confetti path — DONE → hook → render) plus the
/// refreshed [`PageView`] **only** when a hook actually mutated the
/// workspace (so the caller skips a needless re-render on the no-op path).
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginSyncReply {
    /// `ctx.ui.render(html)` payloads from the hooks — painted as iframe
    /// overlays. Independent of `view`: a hook can emit confetti without
    /// mutating the workspace.
    pub views: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
}

/// List every command contributed by a loaded plugin whose
/// `slash-command` capability the mobile client honors. Empty until the
/// workspace opens and plugins load (best-effort).
#[tauri::command]
pub(crate) fn plugin_list(plugins: State<'_, PluginService>) -> Vec<PluginCommandDto> {
    plugins.list_commands()
}

/// List every toolbar button a loaded plugin contributes for the mobile
/// chrome (`host.toolbar_buttons("mobile")`). The header renders one glyph
/// per entry; tapping it fires `plugin_run(plugin_id, command_id)`. Empty
/// until the workspace opens and plugins load (best-effort).
#[tauri::command]
pub(crate) fn plugin_toolbar(plugins: State<'_, PluginService>) -> Vec<ToolbarButtonDto> {
    plugins.list_toolbar()
}

/// List every content transformer granted on the mobile client
/// (`host.transformers()`). The frontend loads this once when the workspace
/// opens and keys on `lang`: a code fence whose language matches fires
/// `plugin_transform(plugin_id, lang, body)`. Empty until the workspace
/// opens and plugins load (best-effort).
#[tauri::command]
pub(crate) fn plugin_transformers(plugins: State<'_, PluginService>) -> Vec<TransformerDto> {
    plugins.list_transformers()
}

/// Run a plugin's content transformer for `lang` against `input`, returning
/// the descriptor (`{kind, content}`) it produced — `None` when the
/// transformer declined. Pure render: no workspace mutation, no
/// re-projection, so (unlike `plugin_run`) it carries no refreshed
/// `PageView`.
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
            let root = state.storage_root.clone();
            with_ws(&state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None)
        }
        None => None,
    };

    Ok(PluginRunReply {
        applied: run.applied,
        notifications: run.notifications,
        errors: run.errors,
        views: run.views,
        view,
    })
}

/// Fire the plugins' `onOp` hook sweep after a user mutation. Best-effort
/// op-hooks (root invariant 7's "any state that converges goes through the
/// op log" applies to the *plugin's* writes too — they route through
/// `outl-actions`).
///
/// The frontend calls this once after committing a block mutation. It must
/// be called with **no** workspace lock held by the webview side; the
/// plugin thread locks the workspace to run the hooks. Returns:
///
/// - `views`: any `ctx.ui.render` payloads the hooks emitted (the confetti
///   path — DONE → hook → render → iframe overlay). Present even when the
///   hooks didn't mutate the workspace (rendering a view is not a
///   mutation).
/// - `view`: the refreshed [`PageView`] of `page_id` **only** when a hook
///   actually mutated the workspace (so the caller skips a needless
///   re-render on the common no-op path).
#[tauri::command]
pub(crate) fn plugin_sync_hooks(
    page_id: Option<String>,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<PluginSyncReply, String> {
    let outcome = plugins.sync_hooks();
    // A hook can emit a view (confetti) without mutating the workspace, so
    // resolve the refreshed page only when something actually changed, but
    // always carry the views back.
    let view = if outcome.applied > 0 {
        match page_id {
            Some(id) => {
                let page = parse_node_id(&id)?;
                let root = state.storage_root.clone();
                with_ws(&state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None)
            }
            None => None,
        }
    } else {
        None
    };
    Ok(PluginSyncReply {
        views: outcome.views,
        view,
    })
}

/// Marketplace rows: the official registry (`plugins.outl.app`) crossed with
/// this workspace's lockfile (installed / enabled flags).
#[tauri::command]
pub(crate) fn plugin_registry_list(
    plugins: State<'_, PluginService>,
) -> Result<Vec<MarketplaceItem>, String> {
    plugins.registry_list()
}

/// Tap-to-install an official plugin by id; returns the installed name.
#[tauri::command]
pub(crate) fn plugin_install_official(
    id: String,
    plugins: State<'_, PluginService>,
) -> Result<String, String> {
    plugins.install_official(id)
}

/// Enable / disable an installed plugin, then reload.
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
