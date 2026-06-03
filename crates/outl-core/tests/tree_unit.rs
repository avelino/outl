//! Unit-flavoured tests for the tree CRDT: do_op, undo_op, apply_op,
//! creates_cycle and the SetCollapsed convergence cases.
//!
//! Lived inline in `crates/outl-core/src/tree.rs` until the module
//! crossed the file-size-guard threshold; pulled out so the algorithm
//! itself can stay small and auditable. Each test exercises only the
//! public `Tree`/`OpLog` surface, which is the same contract every
//! other crate consumes.

use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::log::OpLog;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::tree::Tree;

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
