//! Desktop-only helpers + re-exports of the shared command glue.
//!
//! The cross-client helpers (id parsing, workspace-lock acquisition, the
//! `finish_in_page` mutation funnel, the post-commit announce) live in
//! `outl_tauri_shared::helpers` — generic over the `AppHost` trait
//! `AppState` implements. This module re-exports them so the rest of the
//! crate keeps one import path, and adds the single genuinely
//! desktop-only rule: surgical undo invalidation across a peer reload.

use std::collections::HashMap;

use outl_actions::{render_page_md, HistoryStacks};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

pub(crate) use outl_tauri_shared::helpers::{
    build_page_view, parse_node_id, storage_root_or_err, with_ws_mut,
};

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
