//! Cross-command helpers: id parsing, workspace handles, page-view
//! builder, post-mutation `.md` + sidecar projection.
//!
//! Pure glue — no business logic. Anything that mutates the workspace
//! semantically belongs in `outl-actions`.

use std::path::Path;
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar, backlinks_for_page, date_from_slug, page_meta as page_meta_action,
    read_page_outline_with_workspace, ActionError, SyncTransport,
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
/// still running.
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

pub(crate) fn build_page_view(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    // Read the outline straight from the page's `.md` (+ sidecar for
    // stable block ids). This is the v0 contract: `.md` is the source
    // of truth, the op log is history. `project_outline(workspace,_)`
    // is still available for tools that need to materialise from the
    // op log, but the UI must not use it — it would silently disagree
    // with what the user sees in Files.app or any other editor.
    //
    // The workspace-aware variant overlays `Op::SetCollapsed` so the
    // returned `OutlineNode.collapsed` reflects the op log (the only
    // place that state legitimately lives — sidecars LWW under iCloud
    // and would lose flips).
    let page_outline = read_page_outline_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| outl_actions::PageOutline {
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
/// `.md` + sidecar.
///
/// The op log is the source of truth: every concurrent edit between
/// peers ends up there (each device appends to its own
/// `ops-<actor>.jsonl`, iCloud syncs files individually, and HLC
/// ordering merges them deterministically). The `.md` and the
/// sidecar are projections — we always regenerate them after the
/// workspace mutation so what the user reads on disk matches the
/// op-log state.
///
/// We do **not** run `reconcile_md` before `f`. The op log is already
/// up to date with whatever peers have delivered through the jsonl
/// files; trying to "catch up" from the `.md` would risk emitting
/// Delete cascades when the on-disk `.md` lagged behind the op log
/// (which it does on every iCloud propagation window).
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
    with_ws_mut(state, |ws| {
        let value = f(ws).map_err(|e| e.to_string())?;
        if let Err(e) = apply_page_md_with_sidecar(ws, &state.storage_root, page_id) {
            warn!("page md+sidecar sync failed: {e}");
        }
        announce_after_commit(state, ws, page_id);
        let view = build_page_view(ws, &state.storage_root, page_id).map_err(|e| e.to_string())?;
        Ok((value, view))
    })
}

/// Post-commit hook: tell connected peers this device just produced new ops so a
/// peer pulls **immediately** over iroh gossip, instead of waiting for the
/// catch-up loop's maintenance re-sync.
///
/// Best-effort and non-fatal by design:
/// - no transport wired (file-sync / not yet started) → nothing to announce;
/// - the announce never crosses (flaky cross-network link) → the catch-up loop's
///   periodic re-sync still converges.
///
/// This is the mobile mirror of the TUI's `save()` tail. Without it, GUI edits
/// committed the op locally but never woke peers — so propagation depended
/// entirely on the catch-up timing (the "edit never reached the other device"
/// report).
pub(crate) fn announce_after_commit(state: &State<'_, AppState>, ws: &Workspace, page_id: NodeId) {
    let Some(transport) = &state.iroh else { return };
    let Some(meta) = page_meta_action(ws, page_id) else {
        return;
    };
    transport.announce_local_ops(&meta.slug, state.hlc.next());
}
