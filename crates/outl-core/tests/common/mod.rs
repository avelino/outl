//! Test helpers shared across the invariant battery.
//!
//! Each test in `crates/outl-core/tests/*.rs` is its own integration crate;
//! they pull in this module via `mod common;`.

#![allow(dead_code)]

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::tree::Tree;
use std::collections::BTreeMap;

/// A replica is a (Tree, OpLog) pair living under one ActorId.
pub struct Replica {
    pub actor: ActorId,
    pub tree: Tree,
    pub log: OpLog,
}

impl Replica {
    pub fn new(actor: ActorId) -> Self {
        Self {
            actor,
            tree: Tree::new(),
            log: OpLog::new(),
        }
    }

    pub fn apply(&mut self, op: LogOp) {
        self.tree.apply_op(&mut self.log, op);
    }
}

/// Build a deterministic `LogOp` at a chosen HLC.
pub fn op_at(actor: ActorId, physical: u64, logical: u32, op: Op) -> LogOp {
    LogOp {
        ts: Hlc::new(physical, logical, actor),
        actor,
        op,
    }
}

/// Compare two tree states for structural equivalence.
///
/// Returns `Ok(())` if both trees agree on every node's parent + position
/// and on every property binding. Otherwise returns a string describing
/// the first divergence.
pub fn assert_trees_equal(a: &Tree, b: &Tree) {
    // Compare node-set first.
    let a_nodes: BTreeMap<NodeId, (NodeId, String)> = a
        .iter_nodes()
        .map(|(n, p, pos)| (n, (p, pos.as_str().to_string())))
        .collect();
    let b_nodes: BTreeMap<NodeId, (NodeId, String)> = b
        .iter_nodes()
        .map(|(n, p, pos)| (n, (p, pos.as_str().to_string())))
        .collect();

    if a_nodes != b_nodes {
        // Print a helpful diff before panicking.
        let only_a: Vec<_> = a_nodes
            .keys()
            .filter(|k| !b_nodes.contains_key(k))
            .collect();
        let only_b: Vec<_> = b_nodes
            .keys()
            .filter(|k| !a_nodes.contains_key(k))
            .collect();
        let diffs: Vec<_> = a_nodes
            .iter()
            .filter(|(k, v)| b_nodes.get(k).is_some_and(|bv| bv != *v))
            .collect();
        panic!(
            "trees diverge:\n  only in A: {only_a:?}\n  only in B: {only_b:?}\n  conflicting: {diffs:?}"
        );
    }
}

/// Convenience: a fractional position parsed from a literal.
pub fn pos(s: &str) -> Fractional {
    Fractional::parse(s).unwrap()
}

/// Convenience: `Move` with sentinel `old_*` (the algorithm overwrites
/// these on `do_op`).
pub fn move_op(node: NodeId, new_parent: NodeId, position: Fractional) -> Op {
    Op::Move {
        node,
        new_parent,
        position,
        old_parent: NodeId::root(),
        old_position: Fractional::first(),
    }
}

/// Convenience: `Create` under `parent` at `position`.
pub fn create_op(node: NodeId, parent: NodeId, position: Fractional) -> Op {
    Op::Create {
        node,
        parent,
        position,
    }
}

/// One real `snap-<actor>.bin` from a schema-3 (bincode) build — the
/// upgrade every device lives through exactly once (#207).
/// See `crates/outl-core/fixtures/README.md`.
pub const LEGACY_BINCODE_SCHEMA_3_SNAPSHOT: &[u8] =
    include_bytes!("../../fixtures/legacy-snapshot-schema3.bin");
