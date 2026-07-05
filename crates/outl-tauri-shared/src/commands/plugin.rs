//! Plugin command bodies that combine the [`PluginService`] with a
//! refreshed page view.
//!
//! The pure-delegation commands (`plugin_list`, `plugin_toolbar`, …) are
//! one-line calls into the shared [`PluginService`] and live directly in
//! the client wrappers; only [`run`] and [`sync_hooks`] have a real body
//! (service call + optional page re-render), so they live here.

use serde::Serialize;

use crate::helpers::{build_page_view, parse_node_id, with_ws};
use crate::host::AppHost;
use crate::plugin_service::PluginService;
use crate::state::PageView;

/// Reply for `plugin_run`: the plugin's notifications / errors / applied
/// count, plus the refreshed [`PageView`] of the page that was on screen
/// when the command fired (so the frontend can re-render in one trip).
///
/// `view` is `None` when no page id was supplied (e.g. the palette /
/// sheet fired before a page loaded) or the page no longer resolves — a
/// plugin can move blocks off the current page, but the page node itself
/// stays.
#[derive(Debug, Clone, Serialize)]
pub struct PluginRunReply {
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

/// Reply for `plugin_sync_hooks`: the refreshed [`PageView`] when an
/// op-hook mutated the page on screen (`None` on the common no-op path),
/// plus any `ui-render` views the hooks emitted. The views path is the
/// confetti trigger: toggling a block DONE fires `onOp`, the plugin
/// emits HTML, and the frontend plays it as a sandboxed iframe overlay —
/// so it is populated even when no page re-render is needed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginSyncHooksReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PageView>,
    pub views: Vec<String>,
}

/// Run a plugin command. `page_id` is the page currently on screen; the
/// reply carries its refreshed [`PageView`] so the frontend re-renders
/// without a second round-trip. A plugin can mutate any page — the whole
/// workspace was re-projected on the plugin thread before this returns —
/// but the view we return is the one the user is looking at.
pub fn run<S: AppHost>(
    state: &S,
    plugins: &PluginService,
    plugin_id: String,
    command_id: String,
    page_id: Option<String>,
) -> Result<PluginRunReply, String> {
    let run = plugins.run_command(plugin_id, command_id)?;

    // Re-render the on-screen page from the now-mutated + re-projected
    // workspace. Failure to resolve the page is non-fatal: the mutation
    // already landed, the frontend can fall back to its own reload.
    let view = match page_id {
        Some(id) => {
            let page = parse_node_id(&id)?;
            let root = state.storage_root()?;
            with_ws(state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None)
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

/// Fire the plugins' `onOp` hook sweep after a user mutation.
/// Best-effort op-hooks (root invariant 7's "any state that converges
/// goes through the op log" applies to the *plugin's* writes too — they
/// route through `outl-actions`).
///
/// The frontend calls this once after committing a block mutation. It
/// must be called with **no** workspace lock held by the webview side;
/// the plugin thread locks the workspace to run the hooks. Views ride
/// back even on the no-op-mutation path (a `ui-render` plugin can emit
/// HTML from its `onOp` hook without mutating the workspace); only the
/// page re-render is gated on `applied > 0`.
pub fn sync_hooks<S: AppHost>(
    state: &S,
    plugins: &PluginService,
    page_id: Option<String>,
) -> Result<PluginSyncHooksReply, String> {
    let outcome = plugins.sync_hooks();
    let view = if outcome.applied > 0 {
        match page_id {
            Some(id) => {
                let page = parse_node_id(&id)?;
                let root = state.storage_root()?;
                with_ws(state, |ws| Ok(build_page_view(ws, &root, page).ok())).unwrap_or(None)
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
