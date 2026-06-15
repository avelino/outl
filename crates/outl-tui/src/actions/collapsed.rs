//! `c` chord — fold / unfold the selected block.
//!
//! `App::toggle_collapse_selected` flips the block via
//! `outl_actions::toggle_block_collapsed`, which generates an
//! `Op::SetCollapsed` and routes it through `Workspace::apply`. The
//! op enters the local `ops-<actor>.jsonl`, iCloud / Syncthing
//! propagate the file, and every peer's CRDT replays it in HLC
//! order. The local mirror (`App.collapsed`) is patched to keep the
//! renderer reacting in the same frame the chord fired.

use crate::state::{App, Focus, Mode, ToastKind};
use outl_md::parse::OutlineNode;

impl App {
    /// Rebuild `hidden_by_collapse` from the current `page.blocks`,
    /// `collapsed`, and `id_by_flat`. The vector is the same length
    /// as `id_by_flat`; entry `i` is `true` when block `i` (DFS
    /// preorder) sits under at least one collapsed ancestor.
    ///
    /// O(N) — called every load and every toggle. With N typically
    /// under a few hundred blocks per page, this is in the same noise
    /// as the existing reparse work.
    pub(crate) fn recompute_hidden_by_collapse(&mut self) {
        let mut hidden: Vec<bool> = Vec::with_capacity(self.id_by_flat.len());
        let mut cursor = 0usize;
        walk_hidden(
            &self.page.blocks,
            false,
            &self.collapsed,
            &self.id_by_flat,
            &mut cursor,
            &mut hidden,
        );
        self.hidden_by_collapse = hidden;
    }

    /// Toggle the collapsed flag on the currently selected outline
    /// block. No-op when the cursor is in Insert / Visual or focused
    /// on the backlinks pane.
    ///
    /// The local mirror (`App.collapsed`) is patched optimistically
    /// before the op log apply so the next frame already reflects
    /// the new state. On apply failure we roll the mirror back to
    /// what the workspace believes and surface a toast.
    pub(crate) fn toggle_collapse_selected(&mut self) {
        // Only the outline owns this chord. Backlinks render their
        // own view of source blocks, not the local fold state.
        if !matches!(self.focus, Focus::Outline) {
            return;
        }
        if !matches!(self.mode, Mode::Normal) {
            return;
        }

        let Some(&id) = self.id_by_flat.get(self.selected) else {
            // No sidecar entry for this flat index yet — most likely
            // a brand-new bullet that the next `save()` will reconcile.
            self.toast(
                ToastKind::Info,
                "block has no sidecar entry yet; save first",
            );
            return;
        };

        // Optimistic local flip — the renderer reacts in this frame.
        // We re-sync from the workspace below in case the CRDT
        // resolution differs (e.g. a peer's flip arrived in the same
        // tick).
        let was_collapsed = self.collapsed.contains(&id);
        if was_collapsed {
            self.collapsed.remove(&id);
        } else {
            self.collapsed.insert(id);
        }

        match outl_actions::toggle_block_collapsed(&mut self.workspace, &self.hlc, id) {
            Ok(new_value) => {
                // Adopt the workspace's authoritative value (paranoia:
                // protects against a stale mirror after a peer op
                // arrived between the read and the apply).
                if new_value {
                    self.collapsed.insert(id);
                } else {
                    self.collapsed.remove(&id);
                }
                self.status = if new_value {
                    "collapsed".into()
                } else {
                    "expanded".into()
                };
            }
            Err(e) => {
                // Roll the optimistic mirror back to what the workspace
                // believes — that's still authoritative.
                if self.workspace.tree().is_collapsed(id) {
                    self.collapsed.insert(id);
                } else {
                    self.collapsed.remove(&id);
                }
                self.toast(ToastKind::Error, format!("collapse failed: {e}"));
            }
        }
        self.recompute_hidden_by_collapse();
    }

    /// `zR` — unfold every block on the current page. Emits one
    /// `Op::SetCollapsed(false)` per id in flat order. Every flip lands
    /// in the op log, including no-ops on already-expanded blocks:
    /// `outl_actions::set_block_collapsed` always appends an
    /// `Op::SetCollapsed` (the CRDT needs every flip in the log to
    /// converge concurrent flips across devices via HLC ordering).
    /// `Ok(false)` from the action only reports "value didn't change";
    /// it does **not** mean "log untouched".
    pub(crate) fn unfold_all(&mut self) {
        self.set_all_collapsed(false, "unfolded all");
    }

    /// `zM` — fold every block on the current page **that has
    /// children**. Leaves are skipped because foldar um leaf hoje é
    /// invisível, mas grava `Op::SetCollapsed(true)` no log; quando o
    /// usuário adicionar children embaixo depois, eles aparecem
    /// colapsados — surpresa real. Skip-leaves elimina esse smell e
    /// reduz N (typical pages têm muitos leaves).
    /// See `unfold_all` for the per-op-log-write cost note.
    pub(crate) fn fold_all(&mut self) {
        self.set_all_collapsed(true, "folded all");
    }

    /// Shared body for `unfold_all` / `fold_all`. Walks the AST in
    /// DFS preorder so a node's `children` is in scope; picks ids by
    /// `should_emit` (see [`collect_collapse_candidates`]); asks the
    /// workspace to flip each one and resyncs the local mirror.
    /// Errors are aggregated into a single toast — partial progress
    /// stands so the user doesn't get a half-folded outline.
    fn set_all_collapsed(&mut self, value: bool, success_msg: &str) {
        if !matches!(self.focus, Focus::Outline) {
            return;
        }
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        if self.id_by_flat.is_empty() {
            return;
        }
        let candidates = collect_collapse_candidates(&self.page.blocks, &self.id_by_flat, value);
        let mut errors = 0usize;
        let mut changed = 0usize;
        for id in candidates {
            match outl_actions::set_block_collapsed(&mut self.workspace, &self.hlc, id, value) {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(_) => errors += 1,
            }
        }
        // Resync the mirror from the authoritative tree state.
        self.collapsed.clear();
        for &id in &self.id_by_flat {
            if self.workspace.tree().is_collapsed(id) {
                self.collapsed.insert(id);
            }
        }
        self.recompute_hidden_by_collapse();
        self.status = if errors > 0 {
            format!("{success_msg} ({changed} ok, {errors} failed)")
        } else {
            format!("{success_msg} ({changed})")
        };
    }
}

/// Walk `blocks` in DFS preorder (mirror of `id_by_flat`'s build
/// order) and return the ids `set_all_collapsed` should target.
///
/// When `value == true` (foldar), skip leaves: a `SetCollapsed(true)`
/// on a leaf is invisible today but turns into a surprise the moment
/// the user adds children embaixo. When `value == false` (descolapsar),
/// include every id — descolapsar leaf é no-op no tree mas mantém o
/// log consistente (every flip lands per CRDT contract).
fn collect_collapse_candidates(
    blocks: &[OutlineNode],
    id_by_flat: &[outl_core::id::NodeId],
    value: bool,
) -> Vec<outl_core::id::NodeId> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    walk_candidates(blocks, id_by_flat, &mut cursor, &mut out, value);
    out
}

fn walk_candidates(
    blocks: &[OutlineNode],
    id_by_flat: &[outl_core::id::NodeId],
    cursor: &mut usize,
    out: &mut Vec<outl_core::id::NodeId>,
    value: bool,
) {
    for b in blocks {
        let id = id_by_flat.get(*cursor).copied();
        *cursor += 1;
        let keep = !value || !b.children.is_empty();
        if let (true, Some(id)) = (keep, id) {
            out.push(id);
        }
        walk_candidates(&b.children, id_by_flat, cursor, out, value);
    }
}

/// DFS walk over `blocks` populating `hidden` (in flat preorder).
/// `ancestor_hidden` carries down whether any ancestor is collapsed.
fn walk_hidden(
    blocks: &[OutlineNode],
    ancestor_hidden: bool,
    collapsed: &std::collections::HashSet<outl_core::id::NodeId>,
    id_by_flat: &[outl_core::id::NodeId],
    cursor: &mut usize,
    hidden: &mut Vec<bool>,
) {
    for b in blocks {
        let id = id_by_flat.get(*cursor).copied();
        hidden.push(ancestor_hidden);
        *cursor += 1;
        // A block is itself a collapsed ancestor for its descendants
        // when its id is in `collapsed`. We OR with `ancestor_hidden`
        // so a child of a child of a collapsed node stays hidden too.
        let am_collapsed = id.is_some_and(|i| collapsed.contains(&i));
        let child_hidden = ancestor_hidden || am_collapsed;
        walk_hidden(
            &b.children,
            child_hidden,
            collapsed,
            id_by_flat,
            cursor,
            hidden,
        );
    }
}
