//! Per-op machinery: `do_op` applies one [`LogOp`] forward,
//! `undo_op` reverts it.
//!
//! Each branch of the `match` here mirrors the corresponding rule in
//! Kleppmann et al. 2022 §3 (Move) and the outl-specific extensions
//! for `SetProp`, `Create`, `SetCollapsed`. The `old_*` fields on
//! `Move` and `SetProp` are filled in by `do_op` so `undo_op` can
//! reverse the transition exactly — see the algorithm sketch in
//! `crates/outl-core/CLAUDE.md`.

use super::Tree;
use crate::op::{LogOp, Op};

impl Tree {
    /// Apply one op to the materialized tree, mutating it in place.
    ///
    /// For [`Op::Move`] and [`Op::SetProp`] this also fills in the
    /// `old_*` fields of the passed-in `LogOp` so that [`Self::undo_op`]
    /// can later revert exactly.
    ///
    /// Move with a cycle is a no-op on the materialized tree — but the
    /// caller still appends the `LogOp` to the log unchanged. Reordering
    /// may turn the same op into a non-cycle move later.
    pub fn do_op(&mut self, log_op: &mut LogOp) {
        match &mut log_op.op {
            Op::Move {
                node,
                new_parent,
                position,
                old_parent,
                old_position,
            } => {
                // Capture pre-state for undo. If the node doesn't yet
                // exist locally (Move arrived before its Create), the
                // op is a complete no-op on the tree — but it still
                // ends up in the log, and `undo_op` will skip it via
                // the "parent matches new_parent" check.
                match self.nodes.get(node) {
                    Some((p, pos)) => {
                        *old_parent = *p;
                        *old_position = pos.clone();
                        if !self.creates_cycle(*node, *new_parent) {
                            self.nodes.insert(*node, (*new_parent, position.clone()));
                        }
                        // If a cycle would be created, no mutation of
                        // the tree. The op still ends up in the log.
                    }
                    None => {
                        // Sentinel `old_*` values so undo's parent
                        // check (`parent == new_parent`) cannot match,
                        // making undo a guaranteed no-op.
                        *old_parent = *new_parent;
                        *old_position = position.clone();
                    }
                }
            }
            Op::Edit { .. } => {
                // Block text content lives outside `Tree` (in a Yrs `Doc`
                // managed by `Workspace`). Tree-level do_op is a no-op for
                // `Edit`; the caller dispatches the update separately.
            }
            Op::SetProp {
                node,
                key,
                value,
                old_value,
            } => {
                let key_owned = key.clone();
                *old_value = self.properties.get(&(*node, key_owned.clone())).cloned();
                match value {
                    Some(v) => {
                        self.properties.insert((*node, key_owned), v.clone());
                    }
                    None => {
                        self.properties.remove(&(*node, key_owned));
                    }
                }
            }
            Op::Create {
                node,
                parent,
                position,
            } => {
                // Idempotent: if the node already exists, keep its current
                // parent/position. The Create only seeds initial placement.
                self.nodes
                    .entry(*node)
                    .or_insert_with(|| (*parent, position.clone()));
            }
            Op::SetCollapsed {
                node,
                value,
                old_value,
            } => {
                // Capture previous state for `undo_op`. Membership in
                // `self.collapsed` is the source of truth (presence =
                // collapsed, absence = expanded).
                *old_value = self.collapsed.contains(node);
                if *value {
                    self.collapsed.insert(*node);
                } else {
                    self.collapsed.remove(node);
                }
            }
        }
    }

    /// Revert one previously-applied op, using its `old_*` fields.
    ///
    /// For [`Op::Move`] we only revert if the current parent matches the
    /// move's `new_parent` — otherwise the original `do_op` was a cycle
    /// no-op and there's nothing to undo.
    ///
    /// For [`Op::Edit`] tree-level undo is a no-op; Yrs handles its own
    /// merge semantics, accepting that we can't bit-for-bit reverse a
    /// text update that interleaved with concurrent edits.
    pub fn undo_op(&mut self, log_op: &LogOp) {
        match &log_op.op {
            Op::Move {
                node,
                new_parent,
                old_parent,
                old_position,
                ..
            } => {
                if self.parent(*node) == Some(*new_parent) {
                    self.nodes
                        .insert(*node, (*old_parent, old_position.clone()));
                }
                // else: move was a cycle no-op or was already reverted;
                // tree state is consistent.
            }
            Op::Edit { .. } => {
                // See module docs.
            }
            Op::SetProp {
                node,
                key,
                old_value,
                ..
            } => {
                let k = (*node, key.clone());
                match old_value {
                    Some(v) => {
                        self.properties.insert(k, v.clone());
                    }
                    None => {
                        self.properties.remove(&k);
                    }
                }
            }
            Op::Create { node, .. } => {
                self.nodes.remove(node);
            }
            Op::SetCollapsed {
                node, old_value, ..
            } => {
                // Restore the previous membership captured by `do_op`.
                // `undo_op` on a never-applied `LogOp` (one whose
                // `old_value` is still the default `false`) reduces to
                // "make sure the node is not collapsed", which is a
                // no-op when the materialised state matches.
                if *old_value {
                    self.collapsed.insert(*node);
                } else {
                    self.collapsed.remove(node);
                }
            }
        }
    }
}
