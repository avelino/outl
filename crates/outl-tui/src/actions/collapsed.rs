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
