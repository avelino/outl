//! Cross-cutting helpers used by every command body.
//!
//! Argument parsing, workspace-lock acquisition, and the
//! `finish_in_page` pattern that every mutation funnels through. Pure
//! glue — anything that mutates the workspace semantically belongs in
//! `outl-actions`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar, backlinks_for_page, date_from_slug, page_meta as page_meta_action,
    read_page_outline_with_workspace, render_page_md, ActionError, PageOutline,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use tracing::warn;

use crate::host::AppHost;
use crate::state::{PageView, ERR_LOADING};

pub fn parse_node_id(s: &str) -> Result<NodeId, String> {
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| format!("invalid node id {s}: {e}"))
}

pub fn parse_date(slug: &str) -> Result<NaiveDate, String> {
    date_from_slug(slug).ok_or_else(|| format!("invalid date slug: {slug}"))
}

/// Acquire a read-only handle to the workspace. Returns the
/// `workspace_loading` sentinel string while the background opener is
/// still running or the user hasn't picked a workspace yet.
pub fn with_ws<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&Workspace) -> Result<T, String>,
{
    let guard = state.workspace().lock();
    match guard.as_ref() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// Acquire a mutable handle to the workspace.
pub fn with_ws_mut<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, String>,
{
    let mut guard = state.workspace().lock();
    match guard.as_mut() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// The current storage root, or the `workspace_loading` sentinel.
/// Convenience alias over [`AppHost::storage_root`] so command bodies
/// read like the pre-extraction client code.
pub fn storage_root_or_err<S: AppHost>(state: &S) -> Result<PathBuf, String> {
    state.storage_root()
}

pub fn build_page_view(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    // Read the outline straight from the page's `.md` (+ sidecar for
    // stable block ids) — the `.md` is the projection the user sees on
    // disk. The workspace-aware variant overlays `Op::SetCollapsed` so
    // `OutlineNode.collapsed` reflects the op log (the only place that
    // state legitimately lives).
    let page_outline = read_page_outline_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| PageOutline {
            nodes: Vec::new(),
            warnings: Vec::new(),
        });
    let mut backlinks = backlinks_for_page(workspace, storage_root, &meta);
    // Order per the user's `[display] backlinks_order` (issue #142).
    // The config read is negligible next to the `.md` read above.
    let backlinks_order = outl_config::load().display.backlinks_order;
    outl_actions::sort_backlinks(&mut backlinks, backlinks_order.newest_first());
    Ok(PageView {
        page: meta,
        outline: page_outline.nodes,
        backlinks,
        backlinks_order,
        warnings: page_outline.warnings,
    })
}

/// Apply a workspace mutation `f` and project the result back to
/// `.md` + sidecar.
///
/// The op log is the source of truth: the `.md` and the sidecar are
/// projections regenerated after the mutation so what the user reads on
/// disk matches the op-log state. We do **not** run `reconcile_md`
/// before `f` — the op log is already up to date with whatever peers
/// delivered, and "catching up" from a lagging on-disk `.md` would risk
/// emitting Delete cascades.
pub fn finish_in_page<S, F>(state: &S, page_id: NodeId, f: F) -> Result<PageView, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<(), ActionError>,
{
    finish_in_page_with(state, page_id, f).map(|(_, view)| view)
}

/// Variant of [`finish_in_page`] that also returns whatever value the
/// mutation produced (the new `NodeId` for `create_block`, etc.) so the
/// frontend never has to re-discover it from a DFS diff of the outline.
///
/// When the host exposes undo stacks ([`AppHost::history`] returns
/// `Some`), the pre-mutation `.md` render is recorded — but only when
/// the mutation actually changed the page render, so no-op commands
/// don't turn `Cmd+Z` into a visible nothing.
pub fn finish_in_page_with<S, F, T>(
    state: &S,
    page_id: NodeId,
    f: F,
) -> Result<(T, PageView), String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    let root = state.storage_root()?;
    with_ws_mut(state, |ws| {
        let before = state.history().map(|_| render_page_md(ws, page_id));
        let value = f(ws).map_err(|e| e.to_string())?;
        if let (Some(history), Some(before)) = (state.history(), before) {
            if render_page_md(ws, page_id) != before {
                history.lock().entry(page_id).or_default().record(before);
            }
        }
        if let Err(e) = apply_page_md_with_sidecar(ws, &root, page_id) {
            warn!("page md+sidecar sync failed: {e}");
        }
        announce_after_commit(state, ws, page_id);
        let view = build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?;
        Ok((value, view))
    })
}

/// Post-commit hook: tell connected peers this device just produced new
/// ops so a peer pulls **immediately** over iroh gossip, instead of
/// waiting for the catch-up loop's maintenance re-sync.
///
/// Best-effort and non-fatal by design:
/// - no transport wired (file-sync / not yet started) → nothing to
///   announce;
/// - the announce never crosses (flaky link) → the catch-up loop's
///   periodic re-sync still converges.
///
/// This is the GUI mirror of the TUI's `save()` tail. Without it, edits
/// committed the op locally but never woke peers — so propagation
/// depended entirely on the catch-up timing.
pub fn announce_after_commit<S: AppHost>(state: &S, ws: &Workspace, page_id: NodeId) {
    let Some(transport) = state.sync_transport() else {
        return;
    };
    let Some(meta) = page_meta_action(ws, page_id) else {
        return;
    };
    transport.announce_local_ops(&meta.slug, state.hlc().next());
}
