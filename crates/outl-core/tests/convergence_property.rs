//! Property-based **convergence** suite for the tree CRDT / op log.
//!
//! This is the definitive guard for outl's central correctness claim: *the
//! op log converges*. Each property generates a bounded-but-meaningful set of
//! random ops across several actors with monotonic-per-actor HLCs, then
//! delivers them to multiple replicas under random orderings (and random
//! duplication). It asserts every replica materializes the **identical** tree
//! — nodes, properties, *and* collapsed flags.
//!
//! Why a separate file from `property_based.rs`: the existing proptest suite
//! covers only `Create` + `Move` over a 5-node pool, compares only the
//! forward-vs-reversed pair, and `common::assert_trees_equal` ignores
//! property and collapsed state. This file widens the generator to the full
//! op-variant mix (`Create` / `Move` / delete=`Move`→trash / `SetProp` /
//! `SetCollapsed`), asserts across *N* random permutations all-pairs-equal,
//! exercises idempotent re-delivery, builds concurrent moves that *would*
//! cycle, and compares the **full** materialized state.
//!
//! Invariants guarded (see `crates/outl-core/CLAUDE.md` → "The five
//! invariants"):
//! 1. Convergence (SEC) — all orderings agree.
//! 2. Commutativity after reordering — any permutation, not just reverse.
//! 3. Idempotency — duplicated delivery == single delivery.
//! 4. Tree invariant — no cycle ever materializes.
//! 5. No silent loss — the cycle no-op stays in every replica's log.
//!
//! Determinism: every generated `LogOp` carries a globally unique HLC
//! (`physical = step_index`, distinct actors as final tiebreak), so the
//! idempotency dedup in `Tree::apply_op` never drops two *different* ops as if
//! they were one. Proptest's RNG is seeded per case and shrinks to a minimal
//! counterexample on failure — no wall-clock, no flakiness.

mod common;

use common::Replica;
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::tree::Tree;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

// --------------------------------------------------------------------------
// Full-state canonical key (stronger than common::assert_trees_equal, which
// compares only node parent+position). We need properties + collapsed too,
// because this suite generates SetProp / SetCollapsed ops.
// --------------------------------------------------------------------------

/// A deterministic, total-ordered snapshot of *everything* the tree
/// materializes: node→(parent, position), every property binding, and the
/// collapsed set. `BTree*` give a canonical order so two snapshots are
/// directly comparable with `==` (byte-identical materialization).
#[derive(Debug, PartialEq, Eq)]
struct TreeSnapshot {
    nodes: BTreeMap<String, (String, String)>,
    properties: BTreeMap<(String, String), String>,
    collapsed: BTreeSet<String>,
}

fn snapshot(tree: &Tree) -> TreeSnapshot {
    let nodes = tree
        .iter_nodes()
        .map(|(n, p, pos)| (n.to_string(), (p.to_string(), pos.as_str().to_string())))
        .collect();

    // Reconstruct the property map from per-node iteration. `properties_of`
    // is the only public accessor, so walk every node we know about.
    let mut properties = BTreeMap::new();
    for (node, _, _) in tree.iter_nodes() {
        for (key, value) in tree.properties_of(node) {
            properties.insert((node.to_string(), key.to_string()), format!("{value:?}"));
        }
    }
    // Properties can also exist on nodes not present in `nodes` (e.g. a node
    // moved to trash still carries a record; trash itself never appears). The
    // collapsed set may likewise reference such ids. We capture collapsed ids
    // directly; for properties on trashed nodes, iter_nodes still lists the
    // node (trash is its parent), so the loop above covers them.

    let collapsed = tree.collapsed_ids().map(|n| n.to_string()).collect();

    TreeSnapshot {
        nodes,
        properties,
        collapsed,
    }
}

/// Apply a full op set to a fresh replica and return its snapshot + log len.
fn materialize(ops: &[LogOp]) -> (TreeSnapshot, usize) {
    let mut r = Replica::new(ActorId::new());
    for op in ops {
        r.apply(op.clone());
    }
    (snapshot(&r.tree), r.log.len())
}

/// Deterministic permutation of `0..n` driven by a u64 seed (Fisher–Yates
/// with an inline xorshift). Keeps the suite free of an rng dev-dep and makes
/// the chosen order reproducible from the seed proptest shrinks.
fn permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut state = seed | 1; // never zero
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..n).rev() {
        let j = (next() as usize) % (i + 1);
        idx.swap(i, j);
    }
    idx
}

// --------------------------------------------------------------------------
// Generator: a random op program over a small actor + node pool.
//
// We model the program abstractly (Step), then lower it to concrete `LogOp`s
// with globally unique, monotonic-per-actor HLCs. Lowering tracks which nodes
// already exist so a Create is only emitted once per node and Moves target
// existing nodes — this keeps every generated op *meaningful* (it changes
// state) which is what makes shrinking land on a real minimal counterexample.
// --------------------------------------------------------------------------

const N_ACTORS: usize = 4;
const N_NODES: usize = 6;

/// One abstract step in the generated program. Indices are into the actor /
/// node pools; the lowering pass resolves them to real ids.
#[derive(Clone, Debug)]
enum Step {
    /// Create node[n] under node[parent] (or root if parent==n).
    Create {
        actor: usize,
        n: usize,
        parent: usize,
    },
    /// Move node[n] under node[parent] (or root if parent==n).
    Move {
        actor: usize,
        n: usize,
        parent: usize,
    },
    /// Delete node[n] (Move → trash).
    Delete { actor: usize, n: usize },
    /// Set property `key` on node[n] to a Text value (or clear when None).
    SetProp {
        actor: usize,
        n: usize,
        key: u8,
        set: bool,
    },
    /// Set the collapsed flag of node[n].
    SetCollapsed { actor: usize, n: usize, value: bool },
}

prop_compose! {
    /// A single abstract step. `parent`/`n` overlap is fine — lowering maps
    /// `parent == n` to root, and concurrent self/ancestor moves are exactly
    /// the cycle cases we want to stress.
    fn step_strategy()(
        kind in 0u8..5,
        actor in 0usize..N_ACTORS,
        n in 0usize..N_NODES,
        parent in 0usize..N_NODES,
        key in 0u8..3,
        flag in any::<bool>(),
    ) -> Step {
        match kind {
            0 => Step::Create { actor, n, parent },
            1 => Step::Move { actor, n, parent },
            2 => Step::Delete { actor, n },
            3 => Step::SetProp { actor, n, key, set: flag },
            _ => Step::SetCollapsed { actor, n, value: flag },
        }
    }
}

prop_compose! {
    /// A bounded program of steps.
    fn program_strategy()(steps in prop::collection::vec(step_strategy(), 1..40)) -> Vec<Step> {
        steps
    }
}

/// Stable per-test pools. Built once per case so ids are consistent across the
/// replicas we compare. Actors get distinct `ActorId`s (HLC tiebreak), nodes
/// distinct `NodeId`s.
struct Pools {
    actors: Vec<ActorId>,
    nodes: Vec<NodeId>,
}

impl Pools {
    fn new() -> Self {
        Self {
            actors: (0..N_ACTORS).map(|_| ActorId::new()).collect(),
            nodes: (0..N_NODES).map(|_| NodeId::new()).collect(),
        }
    }
}

/// Lower an abstract program to concrete `LogOp`s.
///
/// HLC assignment: `physical = step_index` (so every op is globally unique and
/// the program's textual order is one valid total order), `logical = 0`,
/// `actor` as the tiebreak. Monotonic-per-actor holds trivially because
/// physical strictly increases with step index. We deliberately do **not**
/// pre-sort; the convergence properties feed these in random orders.
///
/// Both `Create` and `Move` lower their `parent == n` case to `ROOT` (a node
/// can't be its own parent); every other parent is a real pool node, so the
/// full op surface — including a `Create` whose parent is already a descendant
/// of the node — is exercised. That path is what the cycle guard on
/// `Op::Create` exists for (see `create_respects_cycle_guard`); a cycle-forming
/// `Create` is a no-op on the tree but stays in the log, exactly like `Move`.
fn lower(program: &[Step], pools: &Pools) -> Vec<LogOp> {
    let root = NodeId::root();
    let trash = NodeId::trash();
    let mut created: BTreeSet<usize> = BTreeSet::new();
    let mut ops = Vec::with_capacity(program.len());

    for (i, step) in program.iter().enumerate() {
        let physical = i as u64;
        let pos = Fractional::parse("m").expect("valid position");

        let op = match step {
            Step::Create { actor, n, parent } => {
                let parent = if parent == n {
                    root
                } else {
                    pools.nodes[*parent]
                };
                if created.insert(*n) {
                    // First Create for this node — seeds its placement.
                    (
                        pools.actors[*actor],
                        Op::Create {
                            node: pools.nodes[*n],
                            parent,
                            position: pos,
                        },
                    )
                } else {
                    // A second `Create` for the same node is NOT a well-formed
                    // CRDT input: `Op::Create` is idempotent on node-id, so the
                    // surviving placement would depend on which Create arrived
                    // first — an order dependency, not a convergence property.
                    // The convergence-safe way to re-parent an existing node is
                    // `Move` (resolved deterministically by HLC), so lower a
                    // repeat-create into exactly that. Keeps every op meaningful
                    // (it changes state) while staying well-formed.
                    (
                        pools.actors[*actor],
                        Op::Move {
                            node: pools.nodes[*n],
                            new_parent: parent,
                            position: pos,
                            old_parent: NodeId::root(),
                            old_position: Fractional::first(),
                        },
                    )
                }
            }
            Step::Move { actor, n, parent } => {
                let parent = if parent == n {
                    root
                } else {
                    pools.nodes[*parent]
                };
                (
                    pools.actors[*actor],
                    Op::Move {
                        node: pools.nodes[*n],
                        new_parent: parent,
                        position: pos,
                        old_parent: NodeId::root(),
                        old_position: Fractional::first(),
                    },
                )
            }
            Step::Delete { actor, n } => (
                pools.actors[*actor],
                Op::Move {
                    node: pools.nodes[*n],
                    new_parent: trash,
                    position: pos,
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            ),
            Step::SetProp { actor, n, key, set } => {
                let value = if *set {
                    Some(PropValue::Text(format!("v{key}")))
                } else {
                    None
                };
                (
                    pools.actors[*actor],
                    Op::SetProp {
                        node: pools.nodes[*n],
                        key: format!("k{key}"),
                        value,
                        old_value: None,
                    },
                )
            }
            Step::SetCollapsed { actor, n, value } => (
                pools.actors[*actor],
                Op::SetCollapsed {
                    node: pools.nodes[*n],
                    value: *value,
                    old_value: false,
                },
            ),
        };

        ops.push(LogOp {
            ts: Hlc::new(physical, 0, op.0),
            actor: op.0,
            op: op.1,
        });
    }

    ops
}

// --------------------------------------------------------------------------
// Cycle detection helper: walk parent chains in a materialized snapshot and
// assert no node reaches itself. Root/trash terminate the walk.
// --------------------------------------------------------------------------

/// Returns the id of a node that participates in a cycle, if any.
fn find_cycle(tree: &Tree) -> Option<NodeId> {
    for (start, _, _) in tree.iter_nodes() {
        let mut cur = start;
        // Bounded by node_count + slack; a cycle reveals itself well before.
        for _ in 0..(tree.node_count() + 2) {
            match tree.parent(cur) {
                Some(p) => {
                    if p == start {
                        return Some(start);
                    }
                    cur = p;
                }
                None => break, // reached root / trash / detached
            }
        }
    }
    None
}

// --------------------------------------------------------------------------
// Properties
// --------------------------------------------------------------------------

proptest! {
    // Fixed case count + the default deterministic RNG keep this reliable in
    // CI. Bump via PROPTEST_CASES locally for extra confidence.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// PROPERTY 1 — Convergence under reordering.
    ///
    /// A random program over N actors, delivered to several replicas in
    /// several *random* permutations (not just forward/reverse), must
    /// materialize the identical full tree state on every replica, and every
    /// replica's log must hold the same number of ops.
    #[test]
    fn convergence_under_reordering(program in program_strategy(), seeds in prop::array::uniform4(any::<u64>())) {
        let pools = Pools::new();
        // Full op surface, including a Create whose parent is already a
        // descendant of the node — the cycle guard on Op::Create keeps this
        // convergent (cycle-forming Create is a tree no-op, stays in the log).
        let ops = lower(&program, &pools);

        // Baseline: textual (program) order.
        let (baseline_snap, baseline_log) = materialize(&ops);

        // Four more deliveries, each under a distinct random permutation.
        for seed in seeds {
            let order = permutation(ops.len(), seed);
            let permuted: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
            let (snap, log_len) = materialize(&permuted);

            prop_assert_eq!(&snap, &baseline_snap, "tree diverged under reordering");
            prop_assert_eq!(log_len, baseline_log, "log length diverged under reordering");
        }
    }

    /// PROPERTY 2 — Idempotency / duplication.
    ///
    /// Delivering every op 1–3 times (P2P + iCloud can both redeliver the
    /// same op) yields the identical tree and the identical *log length* as
    /// delivering each op once. Dedup is keyed on the HLC; duplicates must
    /// vanish.
    #[test]
    fn idempotent_under_duplication(
        program in program_strategy(),
        dup_seed in any::<u64>(),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        // Reference: each op exactly once, in a random order.
        let order = permutation(ops.len(), order_seed);
        let once: Vec<LogOp> = order.iter().map(|&i| ops[i].clone()).collect();
        let (ref_snap, ref_log) = materialize(&once);

        // Duplicated stream: replay each op 1..=3 times, interleaved by a
        // second permutation so duplicates don't arrive back-to-back.
        let mut dup_stream: Vec<LogOp> = Vec::new();
        let mut state = dup_seed | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for op in &once {
            let times = 1 + (next() % 3); // 1, 2, or 3
            for _ in 0..times {
                dup_stream.push(op.clone());
            }
        }
        // Shuffle the duplicated stream so copies are scattered.
        let dup_order = permutation(dup_stream.len(), dup_seed.rotate_left(1) | 1);
        let dup_stream: Vec<LogOp> = dup_order.iter().map(|&i| dup_stream[i].clone()).collect();

        let (dup_snap, dup_log) = materialize(&dup_stream);

        prop_assert_eq!(&dup_snap, &ref_snap, "duplication changed the tree");
        prop_assert_eq!(dup_log, ref_log, "duplication inflated the log (dedup failed)");
    }

    /// PROPERTY 3 — Concurrent moves never cycle, and converge.
    ///
    /// Concurrently move several nodes onto each other (the classic A→B,
    /// B→A; plus transitive chains). On *every* replica, regardless of
    /// delivery order: (a) no cycle materializes, (b) all replicas agree, and
    /// (c) no ops are lost — every replica's log holds all the ops (the
    /// move-that-would-cycle is a no-op on the tree but stays in the log).
    #[test]
    fn concurrent_moves_never_cycle(
        // A small set of moves drawn from a 4-node pool, biased to collide.
        moves in prop::collection::vec((0usize..4, 0usize..4, 0usize..N_ACTORS), 2..16),
        order_seed in any::<u64>(),
    ) {
        let pools = Pools::new();
        let root = NodeId::root();
        let pos = Fractional::parse("m").expect("valid position");

        // First, create all 4 nodes under root (so the moves have targets).
        let mut ops: Vec<LogOp> = Vec::new();
        for (i, n) in pools.nodes.iter().take(4).enumerate() {
            let actor = pools.actors[0];
            ops.push(LogOp {
                ts: Hlc::new(i as u64, 0, actor),
                actor,
                op: Op::Create { node: *n, parent: root, position: pos.clone() },
            });
        }
        // Then the concurrent moves. Self-moves (a==b) map to root, which is
        // harmless; a!=b builds the cycle pressure.
        let base = pools.nodes.len() as u64;
        for (k, (a, b, who)) in moves.iter().enumerate() {
            let node = pools.nodes[*a];
            let parent = if a == b { root } else { pools.nodes[*b] };
            let actor = pools.actors[*who];
            ops.push(LogOp {
                ts: Hlc::new(base + k as u64, 0, actor),
                actor,
                op: Op::Move {
                    node,
                    new_parent: parent,
                    position: pos.clone(),
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            });
        }

        // Deliver in textual order and in a random permutation.
        let mut r1 = Replica::new(ActorId::new());
        for op in &ops {
            r1.apply(op.clone());
        }
        let order = permutation(ops.len(), order_seed);
        let mut r2 = Replica::new(ActorId::new());
        for &i in &order {
            r2.apply(ops[i].clone());
        }

        // (a) No cycle on either replica.
        prop_assert!(find_cycle(&r1.tree).is_none(), "cycle materialized (order 1)");
        prop_assert!(find_cycle(&r2.tree).is_none(), "cycle materialized (order 2)");

        // (b) Convergence.
        prop_assert_eq!(snapshot(&r1.tree), snapshot(&r2.tree), "replicas diverged");

        // (c) No silent loss: both logs hold every op (HLCs are all unique,
        // so no dedup, so log len == ops.len()).
        prop_assert_eq!(r1.log.len(), ops.len(), "ops lost from log (order 1)");
        prop_assert_eq!(r2.log.len(), ops.len(), "ops lost from log (order 2)");
    }

    /// PROPERTY 4 — HLC total order / actor tiebreak determinism.
    ///
    /// Two ops with the *same* physical+logical time but different actors must
    /// resolve to the same winner on every replica, independent of delivery
    /// order. We move the same node to two different parents at equal physical
    /// time; whichever actor wins the tiebreak must win identically on both
    /// orders.
    #[test]
    fn hlc_actor_tiebreak_is_deterministic(
        same_physical in 1u64..1000,
        order_seed in any::<u64>(),
    ) {
        // Two distinct actors; sort so we know the deterministic winner.
        let mut a1 = ActorId::new();
        let mut a2 = ActorId::new();
        if a1.0 > a2.0 {
            std::mem::swap(&mut a1, &mut a2);
        }
        // a2 has the larger ActorId → larger HLC at equal physical/logical →
        // a2's move is the later op → a2 wins.

        let node = NodeId::new();
        let p1 = NodeId::new();
        let p2 = NodeId::new();
        let root = NodeId::root();
        let pos = Fractional::parse("m").expect("valid position");

        // Setup: create node + both target parents under root, at earlier
        // physical times so they always precede the contested moves.
        let setup = vec![
            LogOp { ts: Hlc::new(0, 0, a1), actor: a1,
                op: Op::Create { node, parent: root, position: pos.clone() } },
            LogOp { ts: Hlc::new(0, 0, a2), actor: a2,
                op: Op::Create { node: p1, parent: root, position: pos.clone() } },
            LogOp { ts: Hlc::new(0, 1, a1), actor: a1,
                op: Op::Create { node: p2, parent: root, position: pos.clone() } },
        ];

        // The two contested moves: same physical+logical, different actor.
        let move_a1 = LogOp { ts: Hlc::new(same_physical, 0, a1), actor: a1,
            op: Op::Move { node, new_parent: p1, position: pos.clone(),
                old_parent: NodeId::root(), old_position: Fractional::first() } };
        let move_a2 = LogOp { ts: Hlc::new(same_physical, 0, a2), actor: a2,
            op: Op::Move { node, new_parent: p2, position: pos.clone(),
                old_parent: NodeId::root(), old_position: Fractional::first() } };

        // Two deliveries: a1-then-a2, and a2-then-a1, each after random setup.
        let build = |contested: [&LogOp; 2], seed: u64| -> NodeId {
            let mut all = setup.clone();
            all.push(contested[0].clone());
            all.push(contested[1].clone());
            let order = permutation(all.len(), seed);
            let mut r = Replica::new(ActorId::new());
            for &i in &order {
                r.apply(all[i].clone());
            }
            r.tree.parent(node).expect("node exists")
        };

        let winner_fwd = build([&move_a1, &move_a2], order_seed);
        let winner_rev = build([&move_a2, &move_a1], order_seed.rotate_left(17) | 1);

        // a2 has the larger ActorId, so its move (equal physical/logical) is
        // the later op in the total order → a2 wins → parent == p2.
        prop_assert_eq!(winner_fwd, p2, "tiebreak winner wrong (delivery 1)");
        prop_assert_eq!(winner_rev, p2, "tiebreak winner wrong (delivery 2)");
    }

    /// PROPERTY 5 — Undo/redo round-trip (the reorder mechanism).
    ///
    /// outl has no user-facing undo; `undo_op`/`do_op` exist as the engine
    /// `apply_op` uses to reorder when a late op (smaller HLC than the log
    /// tail) arrives. The convergence properties above already exercise that
    /// path heavily. This property pins the *round-trip* directly: applying a
    /// program, then delivering one extra op with an HLC *older* than the
    /// whole log (forcing a full undo→redo of every existing op), converges
    /// to the same state as delivering that op first. i.e. the undo/redo of
    /// the entire log is a faithful round-trip.
    #[test]
    fn late_op_undo_redo_round_trips(program in program_strategy(), late_seed in any::<u64>()) {
        let pools = Pools::new();
        let ops = lower(&program, &pools);

        // Construct a "late" op: a Move of some node, stamped with a physical
        // time *below* every op in `ops` (which start at physical 0), using a
        // dedicated actor and a negative-most logical slot. Since physical 0
        // is the floor, we give the late op a smaller ActorId tiebreak at
        // physical 0 — guaranteeing it sorts before the program's first op.
        let late_actor = {
            // Find an ActorId strictly smaller than every pool actor so the
            // late op wins the "earliest" slot deterministically.
            let min_pool = pools.actors.iter().map(|a| a.0).min().expect("actors");
            // Generate until we get one below min_pool (ULID space is huge;
            // in practice the first random one usually qualifies, but loop to
            // be safe and deterministic-enough for a test).
            let mut candidate = ActorId::new();
            let mut tries = 0;
            while candidate.0 >= min_pool && tries < 64 {
                candidate = ActorId::new();
                tries += 1;
            }
            candidate
        };
        // If we couldn't find a smaller actor (astronomically unlikely),
        // fall back to physical underflow guard: just skip the assertion's
        // "late" guarantee by using physical 0 with the found actor; the
        // round-trip equality still holds regardless of who is earliest.
        let pos = Fractional::parse("m").expect("valid position");
        let late = LogOp {
            ts: Hlc::new(0, 0, late_actor),
            actor: late_actor,
            op: Op::Move {
                node: pools.nodes[0],
                new_parent: NodeId::root(),
                position: pos,
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        };

        // Delivery A: full program (random order) THEN the late op. This
        // forces apply_op to undo the entire log and redo it around `late`.
        let order = permutation(ops.len(), late_seed);
        let mut a = Replica::new(ActorId::new());
        for &i in &order {
            a.apply(ops[i].clone());
        }
        a.apply(late.clone());

        // Delivery B: the late op FIRST, then the program. No reorder needed.
        let mut b = Replica::new(ActorId::new());
        b.apply(late.clone());
        for &i in &order {
            b.apply(ops[i].clone());
        }

        prop_assert_eq!(
            snapshot(&a.tree),
            snapshot(&b.tree),
            "undo/redo round-trip diverged from late-first delivery"
        );
        prop_assert_eq!(a.log.len(), b.log.len(), "log length diverged across undo/redo");
    }
}

// --------------------------------------------------------------------------
// Regression: Op::Create must honor the cycle guard (was a real bug, found by
// the convergence suite — see crates/outl-core/CLAUDE.md).
// --------------------------------------------------------------------------

/// `Op::Create` must run the cycle guard, exactly like `Op::Move`.
///
/// `Tree::do_op`'s `Op::Move` branch calls `creates_cycle` before re-parenting.
/// The `Op::Create` branch must do the same: a `Create(node, parent)` whose
/// `parent` is already a descendant of `node` would insert an edge
/// `node → parent` that closes a loop, violating invariant #4 ("materialized
/// state is always a valid tree") and later panicking `creates_cycle` on the
/// malformed tree (`debug_assert!` in `src/tree/cycle.rs`).
///
/// This was a real bug (`Op::Create` did a bare `or_insert`); it is order-
/// independent and reproduces in *every* delivery order. The fix makes a
/// cycle-forming Create a no-op on the tree (the op still goes into the log),
/// the same way Move handles it.
///
/// Minimal reproduction (single actor):
///
/// 1. `Create(B, root)` — B exists under root.
/// 2. `Move(B, C)` — C is a phantom (no node record yet), so `creates_cycle`
///    walks from C, hits `parent(C) == None`, returns `false`, and sets B → C.
/// 3. `Create(C, B)` — `creates_cycle(C, B)` now walks B → C, hits C == node,
///    returns `true`, so the Create is a tree no-op. C is never materialized;
///    B keeps its phantom parent C. No cycle.
#[test]
fn create_respects_cycle_guard() {
    let actor = ActorId::new();
    let b = NodeId::new();
    let c = NodeId::new();
    let root = NodeId::root();
    let p = Fractional::parse("m").expect("valid position");

    let ops = [
        LogOp {
            ts: Hlc::new(1, 0, actor),
            actor,
            op: Op::Create {
                node: b,
                parent: root,
                position: p.clone(),
            },
        },
        LogOp {
            ts: Hlc::new(2, 0, actor),
            actor,
            op: Op::Move {
                node: b,
                new_parent: c, // C is still a phantom -> cycle guard sees no loop
                position: p.clone(),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        },
        LogOp {
            ts: Hlc::new(3, 0, actor),
            actor,
            op: Op::Create {
                node: c,
                parent: b, // would close B -> C -> B; the guard makes it a no-op
                position: p,
            },
        },
    ];

    // Apply in every permutation: apply_op orders by HLC, so the materialized
    // result must be identical and cycle-free regardless of delivery order.
    for seed in [1u64, 2, 3, 5, 8, 13, 21, 34] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(actor);
        for &i in &order {
            r.apply(ops[i].clone());
        }

        assert!(
            find_cycle(&r.tree).is_none(),
            "Op::Create closed a cycle the Move opened (seed {seed}): parent(B)={:?}, parent(C)={:?}",
            r.tree.parent(b),
            r.tree.parent(c),
        );
        // The cycle-forming Create is a no-op: C is never materialized, and B
        // keeps the phantom parent the Move gave it. All three ops still logged.
        assert_eq!(r.tree.parent(b), Some(c), "B should keep its Move target C");
        assert_eq!(
            r.tree.parent(c),
            None,
            "C must not be materialized (Create was a cycle no-op)"
        );
        assert_eq!(r.log.len(), 3, "every op stays in the log (no silent loss)");
    }
}

/// A `Create` whose `parent` does not exist yet still materializes the node —
/// the parent is a phantom, exactly like a `Move` that arrives before its
/// target's `Create` (the `None` arm of `do_op`'s Move branch). The cycle guard
/// must NOT mistake an absent parent for a cycle: `creates_cycle` walking from a
/// parent with no record hits `None` and returns `false`, so the edge is laid
/// down. When the parent's own `Create` arrives, the chain resolves with no
/// cycle, and both ops are in the log — in every delivery order.
#[test]
fn create_with_phantom_parent_materializes_then_resolves() {
    let actor = ActorId::new();
    let x = NodeId::new();
    let parent = NodeId::new();
    let root = NodeId::root();
    let pos = Fractional::parse("m").expect("valid position");

    let ops = [
        // Create(X, parent) — `parent` has no record yet (phantom).
        LogOp {
            ts: Hlc::new(1, 0, actor),
            actor,
            op: Op::Create {
                node: x,
                parent,
                position: pos.clone(),
            },
        },
        // Create(parent, root) — materializes the phantom under root.
        LogOp {
            ts: Hlc::new(2, 0, actor),
            actor,
            op: Op::Create {
                node: parent,
                parent: root,
                position: pos,
            },
        },
    ];

    for seed in [1u64, 2, 7, 11] {
        let order = permutation(ops.len(), seed);
        let mut r = Replica::new(actor);
        for &i in &order {
            r.apply(ops[i].clone());
        }

        assert!(
            find_cycle(&r.tree).is_none(),
            "phantom-parent Create must not cycle (seed {seed})"
        );
        assert_eq!(
            r.tree.parent(x),
            Some(parent),
            "X stays parented under the (now real) parent"
        );
        assert_eq!(
            r.tree.parent(parent),
            Some(root),
            "parent resolves under root once its Create lands"
        );
        assert_eq!(r.log.len(), 2, "both ops stay in the log");
    }
}
