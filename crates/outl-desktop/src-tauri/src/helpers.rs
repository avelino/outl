//! Cross-cutting helpers used by every command module.
//!
//! Argument parsing, workspace-lock acquisition, and the
//! `finish_in_page` pattern that every mutation funnels through.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar, backlinks_for_page, date_from_slug, page_meta as page_meta_action,
    read_page_outline_with_workspace, render_page_md, ActionError, PageOutline,
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
    let page_outline = read_page_outline_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| PageOutline {
            nodes: Vec::new(),
            warnings: Vec::new(),
        });
    let backlinks = backlinks_for_page(workspace, storage_root, &meta);
    Ok(PageView {
        page: meta,
        outline: page_outline.nodes,
        backlinks,
        warnings: page_outline.warnings,
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
    finish_in_page_with(state, page_id, f).map(|(_, view)| view)
}

/// Variant of [`finish_in_page`] that also returns whatever value the
/// mutation produced (the new `NodeId` for `create_block`, etc.) so
/// the frontend never has to re-discover it from a DFS diff of the
/// outline.
pub(crate) fn finish_in_page_with<F, T>(
    state: &State<'_, AppState>,
    page_id: NodeId,
    f: F,
) -> Result<(T, PageView), String>
where
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    let root = storage_root_or_err(state)?;
    with_ws_mut(state, |ws| {
        // Capture the pre-mutation projection for undo. Recorded only
        // when the mutation actually changed the page render, so
        // no-op commands (indent with no previous sibling, an edit
        // committing identical text) don't turn `Cmd+Z` into a
        // visible nothing.
        let before = render_page_md(ws, page_id);
        let value = f(ws).map_err(|e| e.to_string())?;
        if render_page_md(ws, page_id) != before {
            state
                .history
                .lock()
                .entry(page_id)
                .or_default()
                .record(before);
        }
        if let Err(e) = apply_page_md_with_sidecar(ws, &root, page_id) {
            warn!("page md+sidecar sync failed: {e}");
        }
        let view = build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?;
        Ok((value, view))
    })
}
