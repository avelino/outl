//! Strong eventual consistency: three replicas applying the same set of
//! ops in different orders all converge to the same materialized tree.
//!
//! This is the cornerstone invariant of the CRDT. Failing this means the
//! algorithm is broken; nothing else matters.

mod common;

use common::{assert_trees_equal, create_op, move_op, op_at, pos, Replica};
use outl_core::id::{ActorId, NodeId};

#[test]
fn three_replicas_three_orders_converge() {
    // Build a fixed set of ops (varied across all four Op variants), then
    // apply each permutation we care about to a fresh replica.
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_c = ActorId::new();

    let n1 = NodeId::new();
    let n2 = NodeId::new();
    let n3 = NodeId::new();
    let root = NodeId::root();

    let ops = [
        op_at(actor_a, 1, 0, create_op(n1, root, pos("a"))),
        op_at(actor_b, 2, 0, create_op(n2, root, pos("b"))),
        op_at(actor_c, 3, 0, create_op(n3, n1, pos("a"))),
        op_at(actor_a, 4, 0, move_op(n3, n2, pos("c"))),
        op_at(actor_b, 5, 0, move_op(n1, n2, pos("d"))),
    ];

    // Three different application orders.
    let orders: Vec<Vec<usize>> = vec![
        vec![0, 1, 2, 3, 4],
        vec![4, 3, 2, 1, 0],
        vec![2, 0, 4, 1, 3],
    ];

    let mut replicas: Vec<Replica> = orders
        .iter()
        .map(|order| {
            let mut r = Replica::new(actor_a);
            for idx in order {
                r.apply(ops[*idx].clone());
            }
            r
        })
        .collect();

    // Pairwise equality on all three.
    let first = replicas.remove(0);
    for r in &replicas {
        assert_trees_equal(&first.tree, &r.tree);
    }
}

#[test]
fn hundred_op_random_orders_converge() {
    // Generate 100 ops with a small RNG, apply in two distinct orders.
    let actor = ActorId::new();
    let root = NodeId::root();

    // Pre-create 20 nodes so subsequent moves have somewhere to go.
    let nodes: Vec<NodeId> = (0..20).map(|_| NodeId::new()).collect();

    let mut ops = Vec::with_capacity(100);
    let mut rng_state: u64 = 0xDEADBEEF;
    let next = |state: &mut u64| -> u64 {
        // xorshift64 — deterministic, no external dep.
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    };

    // First: all creates so subsequent moves are valid.
    for (i, n) in nodes.iter().enumerate() {
        ops.push(op_at(
            actor,
            (i as u64) + 1,
            0,
            create_op(*n, root, pos("m")),
        ));
    }
    // Then 80 random moves.
    for i in 0..80 {
        let a = (next(&mut rng_state) as usize) % nodes.len();
        let b = (next(&mut rng_state) as usize) % nodes.len();
        let node = nodes[a];
        let parent = nodes[b];
        ops.push(op_at(
            actor,
            (i as u64) + 100,
            0,
            move_op(node, parent, pos("m")),
        ));
    }

    let mut replica_a = Replica::new(actor);
    for op in &ops {
        replica_a.apply(op.clone());
    }

    let mut replica_b = Replica::new(actor);
    let mut shuffled = ops.clone();
    // Reverse order — most adversarial: every apply forces reorder.
    shuffled.reverse();
    for op in shuffled {
        replica_b.apply(op);
    }

    assert_trees_equal(&replica_a.tree, &replica_b.tree);
}
