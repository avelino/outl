//! Cross-cutting helpers used by every command module.
//!
//! Argument parsing, workspace-lock acquisition, and the
//! `finish_in_page` pattern that every mutation funnels through.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar, backlinks_for_page, date_from_slug, page_meta as page_meta_action,
    read_page_view_with_workspace, ActionError,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use parking_lot::MutexGuard;
use tauri::State;
use tracing::warn;

use crate::state::{AppState, PageView, ERR_LOADING};

pub(crate) fn parse_node_id(s: &str) -> Result<NodeId, String> {
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| format!("invalid node id {s}: {e}"))
}

pub(crate) fn parse_date(slug: &str) -> Result<NaiveDate, String> {
    date_from_slug(slug).ok_or_else(|| format!("invalid date slug: {slug}"))
}

/// Acquire a read-only handle to the workspace. Returns the
/// `workspace_loading` sentinel string while the background opener is
/// still running or the user hasn't picked a workspace yet.
pub(crate) fn with_ws<F, T>(state: &State<'_, AppState>, f: F) -> Result<T, String>
where
    F: FnOnce(&Workspace) -> Result<T, String>,
{
    let guard: MutexGuard<'_, Option<Workspace>> = state.workspace.lock();
    match guard.as_ref() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// Acquire a mutable handle to the workspace.
pub(crate) fn with_ws_mut<F, T>(state: &State<'_, AppState>, f: F) -> Result<T, String>
where
    F: FnOnce(&mut Workspace) -> Result<T, String>,
{
    let mut guard = state.workspace.lock();
    match guard.as_mut() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

pub(crate) fn storage_root_or_err(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    state
        .storage_root
        .lock()
        .clone()
        .ok_or_else(|| ERR_LOADING.to_string())
}

pub(crate) fn build_page_view(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    let outline = read_page_view_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| Vec::new());
    let backlinks = backlinks_for_page(workspace, storage_root, &meta);
    Ok(PageView {
        page: meta,
        outline,
        backlinks,
    })
}

/// Apply a workspace mutation `f` and project the result back to
/// `.md` + sidecar. Mirrors the mobile crate's helper — see
/// `crates/outl-mobile/src-tauri/src/lib.rs::finish_in_page` for the
/// full rationale on why we never reconcile `.md` before `f`.
pub(crate) fn finish_in_page<F>(
    state: &State<'_, AppState>,
    page_id: NodeId,
    f: F,
) -> Result<PageView, String>
where
    F: FnOnce(&mut Workspace) -> Result<(), ActionError>,
{
    let root = storage_root_or_err(state)?;
    with_ws_mut(state, |ws| {
        f(ws).map_err(|e| e.to_string())?;
        if let Err(e) = apply_page_md_with_sidecar(ws, &root, page_id) {
            warn!("page md+sidecar sync failed: {e}");
        }
        build_page_view(ws, &root, page_id).map_err(|e| e.to_string())
    })
}
