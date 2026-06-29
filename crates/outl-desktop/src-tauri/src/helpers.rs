//! Cross-cutting helpers used by every command module.
//!
//! Argument parsing, workspace-lock acquisition, and the
//! `finish_in_page` pattern that every mutation funnels through.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar, backlinks_for_page, date_from_slug, page_meta as page_meta_action,
    read_page_outline_with_workspace, render_page_md, ActionError, HistoryStacks, PageOutline,
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

/// Surgical undo invalidation across a peer-driven reload.
///
/// Drops the undo / redo stacks of **only** the pages whose `.md`
/// projection actually changed between `old` and `fresh`. Restoring a
/// pre-reload snapshot of a page the peer DID change would silently
/// revert the peer's edits, so those stacks have to go — but a blanket
/// clear capped `Cmd+Z` at a single step whenever a peer (e.g. the TUI
/// on the same workspace) wrote anything, because every peer write
/// fires `peer-ops-changed` → `reload_workspace`. Pages the reload
/// left untouched keep their full undo depth.
///
/// `old` is `None` only when the workspace wasn't loaded yet; with no
/// baseline to diff against, every stale stack is dropped.
///
/// Extracted from [`crate::commands::workspace::reload_workspace`] so
/// the rule is unit-testable without a Tauri `AppHandle`.
pub(crate) fn invalidate_changed_history(
    old: Option<&Workspace>,
    fresh: &Workspace,
    history: &mut HashMap<NodeId, HistoryStacks<String>>,
) {
    let stale: Vec<NodeId> = match old {
        Some(old) => history
            .keys()
            .filter(|id| render_page_md(old, **id) != render_page_md(fresh, **id))
            .copied()
            .collect(),
        None => history.keys().copied().collect(),
    };
    for id in stale {
        history.remove(&id);
    }
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
        announce_after_commit(state, ws, page_id);
        let view = build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?;
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
/// This is the desktop mirror of the TUI's `save()` tail. Without it, GUI edits
/// committed the op locally but never woke peers — so propagation depended
/// entirely on the catch-up timing (the "edit never reached the other device"
/// report).
pub(crate) fn announce_after_commit(state: &State<'_, AppState>, ws: &Workspace, page_id: NodeId) {
    let guard = state.iroh_transport.lock();
    let Some(transport) = guard.as_ref() else {
        return;
    };
    let Some(meta) = page_meta_action(ws, page_id) else {
        return;
    };
    transport.announce_local_ops(&meta.slug, state.hlc.next());
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_actions::{append_block, open_or_create_page as open_or_create, PageKind};
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    /// Build a workspace with two pages (`alpha`, `beta`), each with a
    /// single block. Page ids are derived deterministically from the
    /// slug (`page_id_from_slug`), so a second workspace built the same
    /// way agrees on the ids — exactly what a peer reload relies on.
    fn workspace_with_two_pages(actor: ActorId, hlc: &HlcGenerator) -> (Workspace, NodeId, NodeId) {
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let alpha = open_or_create(&mut ws, hlc, "alpha", "Alpha", PageKind::Page).unwrap();
        append_block(&mut ws, hlc, Some(alpha), Some("alpha one")).unwrap();
        let beta = open_or_create(&mut ws, hlc, "beta", "Beta", PageKind::Page).unwrap();
        append_block(&mut ws, hlc, Some(beta), Some("beta one")).unwrap();
        (ws, alpha, beta)
    }

    /// Avelino's requested integration test for the surgical history
    /// invalidation (#82): two pages, history recorded on both, a peer
    /// reload that changed **only one** of them must drop only that
    /// page's stack and leave the untouched page's `can_undo` intact.
    #[test]
    fn surgical_invalidation_keeps_untouched_page_undoable() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);

        // 1. Two pages, each with a block.
        let (old, alpha, beta) = workspace_with_two_pages(actor, &hlc);

        // 2. Record an undo snapshot on both (the pre-mutation render,
        //    exactly as `finish_in_page_with` does).
        let mut history: HashMap<NodeId, HistoryStacks<String>> = HashMap::new();
        history
            .entry(alpha)
            .or_default()
            .record(render_page_md(&old, alpha));
        history
            .entry(beta)
            .or_default()
            .record(render_page_md(&old, beta));
        assert!(history[&alpha].can_undo());
        assert!(history[&beta].can_undo());

        // 3. A peer reload that touched ONLY `beta`. Rebuilding the
        //    same two slugs yields the same page ids; `beta` gets an
        //    extra block so its projection diverges, `alpha`'s doesn't.
        let (mut fresh, _, fresh_beta) = workspace_with_two_pages(actor, &hlc);
        append_block(&mut fresh, &hlc, Some(fresh_beta), Some("beta two")).unwrap();
        assert_eq!(
            render_page_md(&old, alpha),
            render_page_md(&fresh, alpha),
            "alpha must be byte-identical across the reload"
        );
        assert_ne!(
            render_page_md(&old, beta),
            render_page_md(&fresh, beta),
            "beta must have diverged"
        );

        invalidate_changed_history(Some(&old), &fresh, &mut history);

        // 4. The untouched page keeps its undo depth; the changed page
        //    lost its (now-misleading) stack.
        assert!(
            history.get(&alpha).is_some_and(|h| h.can_undo()),
            "alpha was untouched by the reload — its undo must survive"
        );
        assert!(
            !history.contains_key(&beta),
            "beta changed across the reload — its stack must be dropped"
        );
    }

    /// With no prior workspace to diff against, every recorded stack is
    /// stale and must be dropped (the `None` arm).
    #[test]
    fn invalidation_with_no_baseline_clears_everything() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let (fresh, alpha, beta) = workspace_with_two_pages(actor, &hlc);

        let mut history: HashMap<NodeId, HistoryStacks<String>> = HashMap::new();
        history.entry(alpha).or_default().record(String::new());
        history.entry(beta).or_default().record(String::new());

        invalidate_changed_history(None, &fresh, &mut history);

        assert!(history.is_empty(), "no baseline → drop every stack");
    }
}
