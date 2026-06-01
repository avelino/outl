//! The tree CRDT.
//!
//! This module implements the algorithm described in:
//!
//! > Martin Kleppmann, Dominic P. Mulligan, Victor B. F. Gomes, Alastair
//! > R. Beresford. *"A highly-available move operation for replicated trees."*
//! > IEEE TPDS 2022. <https://martin.kleppmann.com/papers/move-op.pdf>
//!
//! The four public functions
//!
//! - [`Tree::do_op`]
//! - [`Tree::undo_op`]
//! - [`Tree::apply_op`]
//! - [`Tree::creates_cycle`]
//!
//! together carry the entire correctness contract of the CRDT. They must
//! match the paper line-by-line (see `docs/crdt.md`) and remain at 100%
//! coverage forever.
//!
//! See `crates/outl-core/CLAUDE.md` for the five invariants.

use crate::fractional::Fractional;
use crate::id::NodeId;
use crate::log::OpLog;
use crate::op::{LogOp, Op};
use crate::property::PropValue;
use std::collections::{HashMap, HashSet};

/// Materialized outline tree.
///
/// Stores `(parent, position)` for every node and property triples
/// `(node, key) -> value`. Block text content (Yrs `Doc`s) lives in
/// `Workspace`, not here — the tree CRDT itself is purely structural.
///
/// Construct via [`Tree::new`]; mutate via [`Tree::apply_op`].
#[derive(Debug, Default, Clone)]
pub struct Tree {
    nodes: HashMap<NodeId, (NodeId, Fractional)>,
    properties: HashMap<(NodeId, String), PropValue>,
    /// Nodes whose [`Op::SetCollapsed`] last resolved to `true`.
    /// Absence means expanded (the default for every node, including
    /// ones the op log has never set explicitly).
    ///
    /// Stored as a set rather than a `HashMap<_, bool>` so the "no
    /// entry" / "false" cases share representation and serialised
    /// projections (the sidecar, the wire JSON Mobile receives) don't
    /// distinguish "we know it's expanded" from "we never heard about
    /// this node".
    collapsed: HashSet<NodeId>,
}

impl Tree {
    /// Build an empty tree.
    ///
    /// `ROOT` and `TRASH_ROOT` are implicit — they don't appear in `nodes`
    /// but are valid parents for any other node.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the tree currently contains a record for this node.
    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    /// Current parent of a node, or `None` if the node is not in the tree.
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node).map(|(p, _)| *p)
    }

    /// Current position of a node, or `None` if the node is not in the tree.
    pub fn position(&self, node: NodeId) -> Option<&Fractional> {
        self.nodes.get(&node).map(|(_, p)| p)
    }

    /// Current value of a property, or `None` if unset.
    pub fn property(&self, node: NodeId, key: &str) -> Option<&PropValue> {
        self.properties.get(&(node, key.to_string()))
    }

    /// Whether `node` is currently rendered collapsed (children
    /// hidden in the outline view). Defaults to `false` for any node
    /// the op log has never explicitly set.
    pub fn is_collapsed(&self, node: NodeId) -> bool {
        self.collapsed.contains(&node)
    }

    /// Iterator over every node currently flagged collapsed. Used by
    /// projection layers (sidecar render, mobile wire format) to
    /// snapshot the fold state in one pass.
    pub fn collapsed_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.collapsed.iter().copied()
    }

    /// Iterate every (node, parent, position) triple. Useful for tests.
    pub fn iter_nodes(&self) -> impl Iterator<Item = (NodeId, NodeId, &Fractional)> {
        self.nodes.iter().map(|(n, (p, pos))| (*n, *p, pos))
    }

    /// Total number of nodes in the tree.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Total number of property bindings.
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Whether moving `node` under `new_parent` would create a cycle.
    ///
    /// `node == new_parent` is always a cycle. Otherwise we walk up from
    /// `new_parent` toward the root. If we encounter `node` along the way,
    /// `node` is an ancestor of `new_parent` and the move would close a
    /// loop. The ROOT / TRASH_ROOT sentinels terminate the walk.
    pub fn creates_cycle(&self, node: NodeId, new_parent: NodeId) -> bool {
        if node == new_parent {
            return true;
        }
        let mut cursor = new_parent;
        // Bound the walk by the number of edges in the tree as a safety
        // net against malformed state. A well-formed tree terminates in
        // at most `node_count` steps.
        let mut steps = 0usize;
        let max_steps = self.nodes.len() + 2;
        loop {
            if cursor == NodeId::root() || cursor == NodeId::trash() {
                return false;
            }
            match self.parent(cursor) {
                None => return false,
                Some(p) => {
                    if p == node {
                        return true;
                    }
                    cursor = p;
                }
            }
            steps += 1;
            debug_assert!(
                steps <= max_steps,
                "creates_cycle: malformed tree (loop without sentinel)"
            );
            if steps > max_steps {
                return true;
            }
        }
    }

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

    /// Apply a new op to the log, reordering via undo/replay if necessary.
    ///
    /// Algorithm (per `docs/crdt.md` §`apply_op`):
    ///
    /// 1. If `new_op.ts` already in the log → idempotent no-op.
    /// 2. Otherwise, pop and `undo_op` all log entries whose ts > new_op.ts.
    /// 3. `do_op(new_op)`, push to log.
    /// 4. Re-apply the popped ops in original order, calling `do_op` again
    ///    on each (which re-derives `old_*` against the new state).
    pub fn apply_op(&mut self, log: &mut OpLog, new_op: LogOp) {
        if log.contains_ts(&new_op.ts) {
            return;
        }

        // Undo phase: pop everything strictly newer than new_op.
        let mut undone: Vec<LogOp> = Vec::new();
        while let Some(last) = log.last() {
            if last.ts > new_op.ts {
                let op = log.pop().expect("just peeked Some");
                self.undo_op(&op);
                undone.push(op);
            } else {
                break;
            }
        }

        // Apply the new op.
        let mut op = new_op;
        self.do_op(&mut op);
        log.append(op);

        // Replay the undone ops in their original order. `undone` is
        // newest-first (it's a stack from the pop loop); popping it gives
        // us oldest-first.
        while let Some(mut op) = undone.pop() {
            self.do_op(&mut op);
            log.append(op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::{Hlc, HlcGenerator};
    use crate::id::ActorId;

    fn make_op(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    fn first_pos() -> Fractional {
        Fractional::first()
    }

    #[test]
    fn create_then_move_simple() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();

        let n = NodeId::new();
        let root = NodeId::root();

        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: root,
                    position: first_pos(),
                },
            ),
        );
        assert_eq!(tree.parent(n), Some(root));

        let new_parent = NodeId::new();
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::Create {
                    node: new_parent,
                    parent: root,
                    position: first_pos(),
                },
            ),
        );
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::Move {
                    node: n,
                    new_parent,
                    position: first_pos(),
                    old_parent: root,
                    old_position: first_pos(),
                },
            ),
        );
        assert_eq!(tree.parent(n), Some(new_parent));
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn move_cycle_is_noop_but_op_in_log() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let a = NodeId::new();
        let b = NodeId::new();
        let root = NodeId::root();

        for op in [
            Op::Create {
                node: a,
                parent: root,
                position: first_pos(),
            },
            Op::Create {
                node: b,
                parent: a,
                position: first_pos(),
            },
        ] {
            tree.apply_op(&mut log, make_op(&g, op));
        }

        // Move A under B → cycle (B is descendant of A).
        let cycle_op = Op::Move {
            node: a,
            new_parent: b,
            position: first_pos(),
            old_parent: NodeId::root(),
            old_position: first_pos(),
        };
        tree.apply_op(&mut log, make_op(&g, cycle_op));

        // Tree unchanged.
        assert_eq!(tree.parent(a), Some(root));
        assert_eq!(tree.parent(b), Some(a));
        // But op still recorded.
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn idempotent_apply_no_duplicate() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();
        let create = make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: first_pos(),
            },
        );
        tree.apply_op(&mut log, create.clone());
        tree.apply_op(&mut log, create.clone());
        tree.apply_op(&mut log, create);
        assert_eq!(log.len(), 1);
        assert_eq!(tree.node_count(), 1);
    }

    #[test]
    fn late_op_forces_reorder() {
        let actor_old = ActorId::new();
        let actor_new = ActorId::new();
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();
        let root = NodeId::root();

        // Build an "early" op with ts=1 but apply it second.
        let early = LogOp {
            ts: Hlc::new(1, 0, actor_old),
            actor: actor_old,
            op: Op::Create {
                node: n,
                parent: root,
                position: Fractional::parse("b").unwrap(),
            },
        };
        let late = LogOp {
            ts: Hlc::new(5, 0, actor_new),
            actor: actor_new,
            op: Op::Move {
                node: n,
                new_parent: root,
                position: Fractional::parse("m").unwrap(),
                old_parent: root,
                old_position: Fractional::first(),
            },
        };
        tree.apply_op(&mut log, late);
        tree.apply_op(&mut log, early);

        // Final state must be: Move applied after Create. Position == "m".
        assert_eq!(
            tree.position(n).map(|p| p.as_str().to_string()),
            Some("m".into())
        );
        assert_eq!(log.len(), 2);
        // Log is in HLC order.
        assert_eq!(log.iter().next().unwrap().ts.physical_ms, 1);
        assert_eq!(log.iter().last().unwrap().ts.physical_ms, 5);
    }

    #[test]
    fn property_set_and_undo() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();

        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: first_pos(),
                },
            ),
        );
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::SetProp {
                    node: n,
                    key: "priority".into(),
                    value: Some(PropValue::Text("high".into())),
                    old_value: None,
                },
            ),
        );
        assert_eq!(
            tree.property(n, "priority"),
            Some(&PropValue::Text("high".into()))
        );

        // Late op with smaller ts forces undo of SetProp then redo.
        let late = LogOp {
            ts: Hlc::new(0, 0, actor),
            actor,
            op: Op::SetProp {
                node: n,
                key: "priority".into(),
                value: Some(PropValue::Text("low".into())),
                old_value: None,
            },
        };
        tree.apply_op(&mut log, late);
        // After reorder: "low" applied first, then "high" overrides.
        assert_eq!(
            tree.property(n, "priority"),
            Some(&PropValue::Text("high".into()))
        );
    }

    #[test]
    fn set_collapsed_round_trip() {
        // Plain forward apply: SetCollapsed(true) flips the flag and
        // SetCollapsed(false) clears it. `is_collapsed` is the
        // canonical accessor.
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();

        assert!(!tree.is_collapsed(n), "default is expanded");
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::SetCollapsed {
                    node: n,
                    value: true,
                    old_value: false,
                },
            ),
        );
        assert!(tree.is_collapsed(n));
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::SetCollapsed {
                    node: n,
                    value: false,
                    old_value: false,
                },
            ),
        );
        assert!(!tree.is_collapsed(n));
        assert_eq!(log.len(), 2, "every op stays in the log");
    }

    #[test]
    fn set_collapsed_late_op_replays_correctly() {
        // Concurrent flip on the same node: a late op with smaller ts
        // forces undo+replay. Final state must match the op with the
        // larger HLC (the "winner" of the total order).
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();

        // Larger ts first: collapse to `true`.
        tree.apply_op(
            &mut log,
            make_op(
                &g,
                Op::SetCollapsed {
                    node: n,
                    value: true,
                    old_value: false,
                },
            ),
        );
        // Late-arriving op with ts==0 trying to set `false`. Reorder
        // pops the later op, applies the early one, then replays the
        // later — so `true` still wins.
        let late = LogOp {
            ts: Hlc::new(0, 0, actor),
            actor,
            op: Op::SetCollapsed {
                node: n,
                value: false,
                old_value: false,
            },
        };
        tree.apply_op(&mut log, late);
        assert!(tree.is_collapsed(n), "the larger-ts op wins after reorder");
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn set_collapsed_idempotent_replay() {
        // Re-applying the same `LogOp` (same ts) is a no-op — the HLC
        // dedup at the top of `apply_op` guards against double-applying
        // a peer's op we already saw.
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();

        let op = make_op(
            &g,
            Op::SetCollapsed {
                node: n,
                value: true,
                old_value: false,
            },
        );
        tree.apply_op(&mut log, op.clone());
        let len_before = log.len();
        tree.apply_op(&mut log, op);
        assert_eq!(log.len(), len_before, "duplicate ts must not append");
        assert!(tree.is_collapsed(n));
    }

    #[test]
    fn set_collapsed_undo_restores_previous_state() {
        // Direct exercise of `undo_op`: after applying SetCollapsed
        // with `value=true` (captured `old_value=false`), undoing the
        // op must restore the expanded state.
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let n = NodeId::new();

        let mut applied = make_op(
            &g,
            Op::SetCollapsed {
                node: n,
                value: true,
                old_value: false,
            },
        );
        tree.do_op(&mut applied);
        log.append(applied.clone());
        assert!(tree.is_collapsed(n));
        tree.undo_op(&applied);
        assert!(
            !tree.is_collapsed(n),
            "undo must restore the pre-apply flag"
        );
    }

    #[test]
    fn collapsed_ids_snapshots_current_set() {
        // Projection layers iterate `collapsed_ids()` to ship the fold
        // state to UIs / sidecars. Two nodes flipped collapsed must
        // both appear; a third (untouched) must not.
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut tree = Tree::new();
        let mut log = OpLog::new();
        let a = NodeId::new();
        let b = NodeId::new();
        let c = NodeId::new();
        for node in [a, b] {
            tree.apply_op(
                &mut log,
                make_op(
                    &g,
                    Op::SetCollapsed {
                        node,
                        value: true,
                        old_value: false,
                    },
                ),
            );
        }
        let snapshot: std::collections::HashSet<NodeId> = tree.collapsed_ids().collect();
        assert!(snapshot.contains(&a));
        assert!(snapshot.contains(&b));
        assert!(!snapshot.contains(&c));
    }

    #[test]
    fn set_collapsed_converges_across_three_replicas() {
        // Strong Eventual Consistency for `Op::SetCollapsed`.
        //
        // Three replicas observe the same five flips on the same two
        // nodes but in three different delivery orders. After every
        // op has been applied to every replica, the final
        // `collapsed_ids` set must be identical on all three.
        //
        // The fixture deliberately mixes:
        //   - flips on different nodes (independent — order shouldn't
        //     matter for the final state of either)
        //   - flips on the *same* node (HLC + actor tiebreak decides
        //     the winner; every replica must agree on the same winner)
        let actor_a = ActorId::new();
        let actor_b = ActorId::new();
        let g_a = HlcGenerator::new(actor_a);
        let g_b = HlcGenerator::new(actor_b);
        let n1 = NodeId::new();
        let n2 = NodeId::new();

        // Author the canonical sequence on actor A's generator (so
        // every LogOp has a monotonic ts from A), with two contender
        // ops minted by B against n1 to force a same-node race.
        let ops = [
            make_op(
                &g_a,
                Op::SetCollapsed {
                    node: n1,
                    value: true,
                    old_value: false,
                },
            ),
            make_op(
                &g_b,
                Op::SetCollapsed {
                    node: n1,
                    value: false,
                    old_value: false,
                },
            ),
            make_op(
                &g_a,
                Op::SetCollapsed {
                    node: n2,
                    value: true,
                    old_value: false,
                },
            ),
            make_op(
                &g_a,
                Op::SetCollapsed {
                    node: n1,
                    value: true,
                    old_value: false,
                },
            ),
            make_op(
                &g_b,
                Op::SetCollapsed {
                    node: n2,
                    value: false,
                    old_value: false,
                },
            ),
        ];

        // Three permutations: forward, reverse, and "interleaved"
        // (B's ops first, A's ops second).
        let perm_forward: Vec<usize> = (0..ops.len()).collect();
        let perm_reverse: Vec<usize> = (0..ops.len()).rev().collect();
        let perm_interleaved = vec![1, 4, 0, 2, 3];

        fn run(ops: &[LogOp], order: &[usize]) -> Tree {
            let mut tree = Tree::new();
            let mut log = OpLog::new();
            for &i in order {
                tree.apply_op(&mut log, ops[i].clone());
            }
            tree
        }

        let r1 = run(&ops, &perm_forward);
        let r2 = run(&ops, &perm_reverse);
        let r3 = run(&ops, &perm_interleaved);

        let set1: std::collections::HashSet<NodeId> = r1.collapsed_ids().collect();
        let set2: std::collections::HashSet<NodeId> = r2.collapsed_ids().collect();
        let set3: std::collections::HashSet<NodeId> = r3.collapsed_ids().collect();
        assert_eq!(set1, set2, "forward vs reverse delivery must converge");
        assert_eq!(set1, set3, "forward vs interleaved delivery must converge");
    }
}
