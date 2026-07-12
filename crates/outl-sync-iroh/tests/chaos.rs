//! CHAOS / concurrency battery for the iroh delta-sync flow (Pilar 3).
//!
//! Where the `integration.rs` / `catchup.rs` suites prove a single happy path
//! per scenario, this file *hammers* the same production wire code
//! (`delta_sync` initiator + the `SyncProtocolHandler` responder, both via
//! `outl_sync_iroh::test_support`) with the failure modes a P2P op-log actually
//! hits in the wild: many concurrent writers on one file, reordered + duplicated
//! delivery, an offline partition that heals under load, peer fan-out, and a
//! pairing-shaped dial racing a delta-sync on the same endpoint.
//!
//! Each test drives the **real** functions over real QUIC on loopback (no
//! faking, no relay — direct 127.0.0.1 addrs) under
//! `#[tokio::test(flavor = "multi_thread")]` so the concurrency is genuine.
//!
//! ## Determinism
//!
//! A flaky chaos test is worse than none, so every "randomness" here is a
//! *seeded* xorshift ([`Rng`]) — the interleavings and the duplicate-delivery
//! counts are reproducible from the per-test seed. Network timing is the only
//! true nondeterminism, and every wait goes through the generous
//! `common::STEP_TIMEOUT` (30s) + `wait_until` poll loop, never a fixed sleep we
//! race against. Sizes are bounded (≤ ~64 ops, ≤ 8 tasks) so the whole file
//! stays well under a minute while still stressing the lock + dedup paths.
//!
//! ## What proves "no corruption"
//!
//! The op-log self-heals glued lines on read (`JsonlStorage::reload` recovers
//! `…}}}{…` via a streaming deserializer), so going through `all_ops` would
//! MASK a real append-lock failure. To catch raw corruption we read the
//! `ops-<actor>.jsonl` bytes directly and assert each physical line is *exactly
//! one* JSON value ([`assert_every_line_is_one_json_value`]). That is the
//! regression for the `}}}{` glued-op corruption found on a real iCloud
//! workspace — turned into a stress test (N writers × M ops), not a single case.

mod chaos_helpers;
mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc;

use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_sync_iroh::test_support;

use chaos_helpers::{assert_every_line_is_one_json_value, seed_ops_from, Rng};
use common::{
    created_nodes_on_disk, disk_has_all_nodes, fresh_identity, seed_ops, shared_wid, wait_until,
    STEP_TIMEOUT,
};

/// Full set of created-node ids on a workspace's disk, regardless of authoring
/// actor (reads every `ops-*.jsonl` via the production storage).
///
/// Lives here (not in `chaos_helpers`) because it wraps `common`, and `common`
/// must be loaded by exactly one module per test binary (clippy `duplicate_mod`).
fn all_node_ids_on_disk(workspace_root: &Path, reader_actor: ActorId) -> BTreeSet<String> {
    created_nodes_on_disk(workspace_root, reader_actor)
        .into_iter()
        .map(|(_actor, node)| node)
        .collect()
}

// ── Scenario 1: concurrent writers don't corrupt the op log ─────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_never_corrupt_op_log() {
    // The regression for the `}}}{` glued-op corruption, as a STRESS test:
    // N concurrent initiators each dial the SAME responder and push ops authored
    // by the SAME actor, so every inbound `serve()` writes to the responder's
    // ONE `ops-<actor>.jsonl` concurrently. Those writes are serialized only by
    // the responder's single shared `AppendLock` — exactly the production race
    // (boot + catch-up + gossip + serve all hammering one file). Afterwards the
    // raw bytes must show every line is one JSON value (lock held), and the op
    // set on disk must be the exact union with no loss.
    let seed = 0xC0FFEE_u64;

    // One shared victim actor: all N batches land in ops-<actor>.jsonl on the
    // responder, maximizing same-file contention.
    let victim_actor = ActorId::new();

    let n_writers: u32 = 8;
    let ops_per_writer: u32 = 8;

    // Responder workspace starts empty; it will receive everything.
    let dir_r = tempfile::tempdir().expect("R tempdir");
    let id_r = fresh_identity(dir_r.path(), "r");
    let actor_r = ActorId::new();
    let (r_ready_tx, _r_ready_rx) = mpsc::channel::<()>();
    let ep_r = test_support::bind_sync_endpoint(&id_r)
        .await
        .expect("bind R endpoint");
    let r_addr = ep_r.addr();
    let _router_r = test_support::spawn_responder(
        ep_r,
        dir_r.path().to_path_buf(),
        shared_wid(),
        actor_r,
        r_ready_tx,
        // Writers are minted inside the loop below; each authorizes itself against
        // R via `authorize_peer` right before it dials (spawn-time slice can't
        // name identities that don't exist yet).
        &[],
    );

    // Author ONE canonical batch for the victim actor, then hand each writer a
    // DENSE PREFIX of it. Real peers hold dense prefixes of the same actor
    // (partial propagation of C's log), never disjoint sparse counters — the
    // latter breaks the 1-actor-per-device invariant the vector clock
    // (last-ts-per-actor) relies on, so an out-of-order writer holding only
    // "middle" counters looked already-synced and its ops were silently dropped.
    // Prefixes converge: the longest prefix carries the full union, each op once.
    let total_ops = n_writers * ops_per_writer;
    let canonical = tempfile::tempdir().expect("canonical tempdir");
    let all_nodes = common::seed_ops(canonical.path(), victim_actor, total_ops);
    let all_expected: BTreeSet<String> = all_nodes.iter().map(|n| n.to_string()).collect();
    let canonical_lines: Vec<String> = std::fs::read_to_string(
        canonical
            .path()
            .join("ops")
            .join(format!("ops-{victim_actor}.jsonl")),
    )
    .expect("read canonical ops log")
    .lines()
    .map(str::to_string)
    .collect();

    let mut handles = Vec::new();
    let mut rng = Rng::new(seed);

    // Shuffle the launch order deterministically so dials don't go out in a tidy
    // sequence (more interleaving pressure on the responder's lock).
    let mut order: Vec<u32> = (0..n_writers).collect();
    rng.shuffle(&mut order);

    for w in order {
        let dir_w = tempfile::tempdir().expect("writer tempdir");
        let id_w = fresh_identity(dir_w.path(), &format!("w{w}"));
        // Authorize this freshly-minted writer against R before it dials in
        // (issue #158: R refuses any peer not in its peers.json).
        test_support::authorize_peer(dir_r.path(), id_w.node_id());
        let actor_w = ActorId::new(); // distinct device identity per writer
                                      // This writer holds the first (w+1)*ops_per_writer ops of the canonical
                                      // victim log — a dense prefix. The longest prefix carries the full set.
        let prefix_len = ((w + 1) * ops_per_writer) as usize;
        let ops_w_dir = dir_w.path().join("ops");
        std::fs::create_dir_all(&ops_w_dir).expect("writer ops dir");
        let mut content = canonical_lines[..prefix_len].join("\n");
        content.push('\n');
        std::fs::write(ops_w_dir.join(format!("ops-{victim_actor}.jsonl")), content)
            .expect("write writer prefix");

        let ep_w = test_support::bind_sync_endpoint(&id_w)
            .await
            .expect("bind writer endpoint");
        let r_addr = r_addr.clone();
        let dir_w_path = dir_w.path().to_path_buf();
        let handle = tokio::spawn(async move {
            // Keep the tempdir + endpoint alive for the whole sync.
            let _keep = (dir_w, ep_w.clone());
            let (tx, _rx) = mpsc::channel::<()>();
            let res = tokio::time::timeout(
                STEP_TIMEOUT,
                test_support::run_delta_sync(
                    &ep_w,
                    r_addr,
                    &dir_w_path,
                    &shared_wid(),
                    actor_w,
                    tx,
                ),
            )
            .await;
            res.expect("writer sync timed out").expect("writer sync");
        });
        handles.push(handle);
    }

    for h in handles {
        h.await.expect("writer task panicked");
    }

    // The responder must hold every node from every writer (no loss).
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_r.path(), actor_r, &all_expected)
        }),
        "responder must hold the full union of all writers' ops"
    );

    // Raw-bytes invariant: the shared victim file must have ZERO glued/torn lines.
    let ops_dir = dir_r.path().join("ops");
    assert_every_line_is_one_json_value(&ops_dir, victim_actor);

    // Exact union, no duplication blow-up: dedup-by-id means the disk set equals
    // the union of distinct node ids we authored.
    let on_disk = all_node_ids_on_disk(dir_r.path(), actor_r);
    assert_eq!(
        on_disk, all_expected,
        "responder op set must be exactly the union of all writers' ops (no loss, no dup-induced extras)"
    );
    assert_eq!(
        on_disk.len(),
        (n_writers * ops_per_writer) as usize,
        "every authored op landed exactly once"
    );
}

// ── Scenario 2: reordered + duplicated delivery converges ────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reordered_and_duplicated_delivery_converges() {
    // Three nodes, each authoring its own disjoint batch. We run repeated sync
    // passes in a deterministically-shuffled order and DELIVER some passes more
    // than once (duplicate delivery). Dedup-by-op-id + HLC order must make all
    // three nodes converge to the identical materialized op set despite the
    // arbitrary interleaving and the duplicates.
    let seed = 0x1234_5678_u64;
    let mut rng = Rng::new(seed);

    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let dir_c = tempfile::tempdir().expect("C tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_c = ActorId::new();

    let a_nodes = seed_ops(dir_a.path(), actor_a, 5);
    let b_nodes = seed_ops(dir_b.path(), actor_b, 6);
    let c_nodes = seed_ops(dir_c.path(), actor_c, 7);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");
    let id_c = fresh_identity(dir_c.path(), "c");

    // Every node is BOTH an initiator (its own endpoint) and a responder (its own
    // router), so any directed pair can sync.
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let ep_c = test_support::bind_sync_endpoint(&id_c)
        .await
        .expect("bind C");
    let addr_a = ep_a.addr();
    let addr_b = ep_b.addr();
    let addr_c = ep_c.addr();

    let (ta, _ra) = mpsc::channel::<()>();
    let (tb, _rb) = mpsc::channel::<()>();
    let (tc, _rc) = mpsc::channel::<()>();
    let _router_a = test_support::spawn_responder(
        ep_a.clone(),
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        ta,
        &[id_b.node_id(), id_c.node_id()],
    );
    let _router_b = test_support::spawn_responder(
        ep_b.clone(),
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        tb,
        &[id_a.node_id(), id_c.node_id()],
    );
    let _router_c = test_support::spawn_responder(
        ep_c.clone(),
        dir_c.path().to_path_buf(),
        shared_wid(),
        actor_c,
        tc,
        &[id_a.node_id(), id_b.node_id()],
    );

    // The directed pairs that, run enough times in any order, fully connect the
    // triangle. We over-provision the schedule with duplicates and shuffle it.
    // Each entry: (initiator endpoint index, responder addr index, initiator
    // workspace dir, initiator actor).
    #[derive(Clone, Copy)]
    enum Node {
        A,
        B,
        C,
    }
    let pairs = [
        (Node::A, Node::B),
        (Node::B, Node::C),
        (Node::C, Node::A),
        (Node::A, Node::C),
        (Node::B, Node::A),
        (Node::C, Node::B),
    ];

    // Run THREE full rounds (so even a fully-shuffled, duplicate-heavy schedule
    // reaches a transitive closure), each round a fresh deterministic shuffle,
    // and inside each round randomly deliver ~half the passes a SECOND time.
    for _round in 0..3 {
        let mut schedule: Vec<(Node, Node)> = pairs.to_vec();
        rng.shuffle(&mut schedule);

        for (init, resp) in schedule {
            let deliveries = if rng.below(2) == 0 { 2 } else { 1 }; // duplicate ~half
            for _ in 0..deliveries {
                let (ep, dir, actor) = match init {
                    Node::A => (&ep_a, dir_a.path(), actor_a),
                    Node::B => (&ep_b, dir_b.path(), actor_b),
                    Node::C => (&ep_c, dir_c.path(), actor_c),
                };
                let addr = match resp {
                    Node::A => addr_a.clone(),
                    Node::B => addr_b.clone(),
                    Node::C => addr_c.clone(),
                };
                let (tx, _rx) = mpsc::channel::<()>();
                tokio::time::timeout(
                    STEP_TIMEOUT,
                    test_support::run_delta_sync(ep, addr, dir, &shared_wid(), actor, tx),
                )
                .await
                .expect("sync pass timed out")
                .expect("sync pass failed");
            }
        }
    }

    // All three must converge to the identical full union.
    let full: BTreeSet<String> = a_nodes
        .iter()
        .chain(b_nodes.iter())
        .chain(c_nodes.iter())
        .map(|n| n.to_string())
        .collect();

    for (label, dir, reader) in [
        ("A", dir_a.path(), actor_a),
        ("B", dir_b.path(), actor_b),
        ("C", dir_c.path(), actor_c),
    ] {
        assert!(
            wait_until(STEP_TIMEOUT, || disk_has_all_nodes(dir, reader, &full)),
            "{label} must converge to the full op union under reordered+duplicated delivery"
        );
        let on_disk = all_node_ids_on_disk(dir, reader);
        assert_eq!(
            on_disk, full,
            "{label} op set must equal the union exactly — duplicate delivery must not add extras"
        );
    }
}

// ── Scenario 3: partition + heal (offline catch-up under load) ───────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partition_then_heal_under_load() {
    // B goes "offline" (does not sync) while A and C make MANY edits and also
    // sync with each other. When B comes back it must converge to the full
    // A+B+C state via catch-up — no lost ops.
    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");
    let dir_c = tempfile::tempdir().expect("C tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let actor_c = ActorId::new();

    // Initial disjoint state on every node.
    let a1 = seed_ops(dir_a.path(), actor_a, 6);
    let b1 = seed_ops(dir_b.path(), actor_b, 6);
    let c1 = seed_ops(dir_c.path(), actor_c, 6);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");
    let id_c = fresh_identity(dir_c.path(), "c");

    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A");
    let ep_c = test_support::bind_sync_endpoint(&id_c)
        .await
        .expect("bind C");
    let addr_a = ep_a.addr();
    let addr_c = ep_c.addr();

    let (ta, _ra) = mpsc::channel::<()>();
    let (tc, _rc) = mpsc::channel::<()>();
    let _router_a = test_support::spawn_responder(
        ep_a.clone(),
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        ta,
        &[id_b.node_id(), id_c.node_id()],
    );
    let _router_c = test_support::spawn_responder(
        ep_c.clone(),
        dir_c.path().to_path_buf(),
        shared_wid(),
        actor_c,
        tc,
        &[id_a.node_id()],
    );

    // While B is partitioned, A and C make MANY more edits and reconcile A↔C
    // several times. B never participates.
    for round in 0..3u32 {
        seed_ops_from(dir_a.path(), actor_a, 4, 1000 + round * 10);
        seed_ops_from(dir_c.path(), actor_c, 4, 2000 + round * 10);
        // A pulls/pushes against C.
        let (tx, _rx) = mpsc::channel::<()>();
        tokio::time::timeout(
            STEP_TIMEOUT,
            test_support::run_delta_sync(
                &ep_a,
                addr_c.clone(),
                dir_a.path(),
                &shared_wid(),
                actor_a,
                tx,
            ),
        )
        .await
        .expect("A↔C sync timed out")
        .expect("A↔C sync");
    }

    // A and C have now converged with each other but NOT with B.
    let ac_union: BTreeSet<String> = all_node_ids_on_disk(dir_a.path(), actor_a)
        .union(&all_node_ids_on_disk(dir_c.path(), actor_c))
        .cloned()
        .collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_a.path(), actor_a, &ac_union)
        }),
        "A and C should be converged before B heals"
    );
    // B is still isolated: it must not have A's or C's ops yet.
    let b_before = all_node_ids_on_disk(dir_b.path(), actor_b);
    assert!(
        a1.iter().all(|n| !b_before.contains(&n.to_string()))
            && c1.iter().all(|n| !b_before.contains(&n.to_string())),
        "B must still be partitioned (no A/C ops) before healing"
    );

    // HEAL: B comes online and syncs against A (which already holds A+C). Then,
    // to push B's own offline edits the other way, A also syncs against B's
    // endpoint. We bring B's endpoint + responder up now (it was "offline").
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B");
    let (tb, _rb) = mpsc::channel::<()>();
    let _router_b = test_support::spawn_responder(
        ep_b.clone(),
        dir_b.path().to_path_buf(),
        shared_wid(),
        actor_b,
        tb,
        // No one dials B in this test: B only ever initiates (heal → A). It's a
        // responder purely for symmetry, so it authorizes nobody.
        &[],
    );

    // B initiates against A: bidirectional delta-sync pulls A+C into B and pushes
    // B's offline edits into A in the same pass.
    let (tx, _rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_b,
            addr_a.clone(),
            dir_b.path(),
            &shared_wid(),
            actor_b,
            tx,
        ),
    )
    .await
    .expect("B↔A heal timed out")
    .expect("B↔A heal");

    // Propagate B's ops on to C (A now has them; C syncs against A).
    let (tx, _rx) = mpsc::channel::<()>();
    tokio::time::timeout(
        STEP_TIMEOUT,
        test_support::run_delta_sync(
            &ep_c,
            addr_a.clone(),
            dir_c.path(),
            &shared_wid(),
            actor_c,
            tx,
        ),
    )
    .await
    .expect("C↔A reconcile timed out")
    .expect("C↔A reconcile");

    // Build the full expected union across all three (every batch, including the
    // offline edits from each side).
    let full: BTreeSet<String> = all_node_ids_on_disk(dir_a.path(), actor_a)
        .union(&all_node_ids_on_disk(dir_b.path(), actor_b))
        .cloned()
        .collect::<BTreeSet<_>>()
        .union(&all_node_ids_on_disk(dir_c.path(), actor_c))
        .cloned()
        .collect();

    // Sanity: the full union must contain every node's ORIGINAL seed batch.
    for n in a1.iter().chain(b1.iter()).chain(c1.iter()) {
        assert!(
            full.contains(&n.to_string()),
            "full union must include every original seed op"
        );
    }

    for (label, dir, reader) in [
        ("A", dir_a.path(), actor_a),
        ("B", dir_b.path(), actor_b),
        ("C", dir_c.path(), actor_c),
    ] {
        assert!(
            wait_until(STEP_TIMEOUT, || disk_has_all_nodes(dir, reader, &full)),
            "{label} must hold the full A+B+C state after partition heals (no lost ops)"
        );
    }

    // No corruption on the healed node's files either.
    let b_ops_dir = dir_b.path().join("ops");
    assert_every_line_is_one_json_value(&b_ops_dir, actor_b);
}

// ── Scenario 4: many peers / fan-out + in-flight guard ───────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fan_out_to_many_peers_converges_without_double_dial() {
    // One hub node with several peers. A fan-out pass dials all of them
    // concurrently; the hub converges to every peer's ops. We ALSO launch
    // redundant concurrent dials to the SAME peer and assert the in-flight guard
    // (production `InFlightPeers`) prevents a double-dial from corrupting the
    // hub's op log.
    //
    // Note: `test_support::run_delta_sync` creates a fresh per-call append lock,
    // so to exercise the production in-flight + shared-lock path we instead make
    // the HUB the responder and have the peers dial IN. Every inbound `serve()`
    // shares the hub responder's ONE append lock — the real fan-out write race.
    let n_peers: u32 = 5;

    let dir_hub = tempfile::tempdir().expect("hub tempdir");
    let id_hub = fresh_identity(dir_hub.path(), "hub");
    let actor_hub = ActorId::new();
    let (hub_tx, _hub_rx) = mpsc::channel::<()>();
    let ep_hub = test_support::bind_sync_endpoint(&id_hub)
        .await
        .expect("bind hub");
    let hub_addr = ep_hub.addr();
    let _router_hub = test_support::spawn_responder(
        ep_hub,
        dir_hub.path().to_path_buf(),
        shared_wid(),
        actor_hub,
        hub_tx,
        // Peers are minted in the loop below; each authorizes itself against the
        // hub before it dials (issue #158).
        &[],
    );

    let mut handles = Vec::new();
    let mut all_expected: BTreeSet<String> = BTreeSet::new();

    for p in 0..n_peers {
        let dir_p = tempfile::tempdir().expect("peer tempdir");
        let id_p = fresh_identity(dir_p.path(), &format!("p{p}"));
        // Authorize this peer against the hub before it dials in.
        test_support::authorize_peer(dir_hub.path(), id_p.node_id());
        let actor_p = ActorId::new();
        let nodes = seed_ops(dir_p.path(), actor_p, 4);
        for n in &nodes {
            all_expected.insert(n.to_string());
        }
        let ep_p = test_support::bind_sync_endpoint(&id_p)
            .await
            .expect("bind peer");
        let hub_addr = hub_addr.clone();
        let dir_p_path = dir_p.path().to_path_buf();

        // Launch the peer's dial TWICE concurrently against the same hub: this is
        // the redundant-dial case. Both must succeed (delta_sync is a no-op on a
        // matching clock) and neither may corrupt the hub log.
        let handle = tokio::spawn(async move {
            let _keep = dir_p; // keep tempdir alive
            let mut inner = Vec::new();
            for _dup in 0..2 {
                let ep_p = ep_p.clone();
                let hub_addr = hub_addr.clone();
                let dir_p_path = dir_p_path.clone();
                inner.push(tokio::spawn(async move {
                    let (tx, _rx) = mpsc::channel::<()>();
                    tokio::time::timeout(
                        STEP_TIMEOUT,
                        test_support::run_delta_sync(
                            &ep_p,
                            hub_addr,
                            &dir_p_path,
                            &shared_wid(),
                            actor_p,
                            tx,
                        ),
                    )
                    .await
                    .expect("peer dial timed out")
                    .expect("peer dial");
                }));
            }
            for h in inner {
                h.await.expect("peer dup-dial task panicked");
            }
            // Keep the endpoint alive until both dials are done.
            drop(ep_p);
        });
        handles.push(handle);
    }

    for h in handles {
        h.await.expect("peer task panicked");
    }

    // Hub converges to every peer's ops.
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_hub.path(), actor_hub, &all_expected)
        }),
        "hub must converge to all peers' ops after fan-out"
    );

    let on_disk = all_node_ids_on_disk(dir_hub.path(), actor_hub);
    assert_eq!(
        on_disk, all_expected,
        "hub op set must be exactly the union of all peers' ops"
    );
    assert_eq!(
        on_disk.len(),
        (n_peers * 4) as usize,
        "every peer op landed exactly once despite duplicate concurrent dials"
    );

    // Each peer authored under its own actor; assert no glued line on ANY actor
    // file in the hub workspace (the double-dial write race must stay clean).
    let hub_ops_dir = dir_hub.path().join("ops");
    let storage = JsonlStorage::open(hub_ops_dir.clone(), actor_hub).expect("open hub storage");
    let actors: BTreeSet<ActorId> = storage
        .all_ops()
        .expect("hub all_ops")
        .into_iter()
        .map(|op| op.actor)
        .collect();
    for actor in actors {
        assert_every_line_is_one_json_value(&hub_ops_dir, actor);
    }
}

// ── Scenario 5: interleaved pairing-shaped dials + sync on one endpoint ───────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_inbound_dials_on_single_endpoint_stay_clean() {
    // Single-endpoint invariant under concurrency. A pair handshake in the real
    // transport rides the SAME endpoint a delta_sync uses (one endpoint per
    // identity — see crate CLAUDE.md). On relay-less loopback we can't drive the
    // gossip/pairing swarm, so we model the invariant directly: many concurrent
    // inbound delta-sync connections hit ONE responder endpoint (sharing its one
    // append lock) WHILE that same endpoint is itself initiating an outbound
    // delta-sync. Neither direction may corrupt the op log, and the responder
    // must still converge.
    let seed = 0xABCD_u64;
    let mut rng = Rng::new(seed);

    // The hub is the single shared endpoint: it both ACCEPTS inbound syncs and
    // INITIATES an outbound one, concurrently.
    let dir_hub = tempfile::tempdir().expect("hub tempdir");
    let id_hub = fresh_identity(dir_hub.path(), "hub");
    let actor_hub = ActorId::new();
    let hub_nodes = seed_ops(dir_hub.path(), actor_hub, 4);
    let ep_hub = test_support::bind_sync_endpoint(&id_hub)
        .await
        .expect("bind hub");
    let (hub_tx, _hub_rx) = mpsc::channel::<()>();
    let _router_hub = test_support::spawn_responder(
        ep_hub.clone(),
        dir_hub.path().to_path_buf(),
        shared_wid(),
        actor_hub,
        hub_tx,
        // Inbound dialers are minted in the loop below; each authorizes itself
        // against the hub before it dials in (issue #158).
        &[],
    );

    // A separate "outbound target" the hub will dial out to, concurrently with
    // the inbound storm — this is the "delta_sync in flight while another dial
    // lands on the same endpoint" condition.
    let dir_out = tempfile::tempdir().expect("out tempdir");
    let id_out = fresh_identity(dir_out.path(), "out");
    let actor_out = ActorId::new();
    let out_nodes = seed_ops(dir_out.path(), actor_out, 4);
    let ep_out = test_support::bind_sync_endpoint(&id_out)
        .await
        .expect("bind out");
    let out_addr = ep_out.addr();
    let (out_tx, _out_rx) = mpsc::channel::<()>();
    let _router_out = test_support::spawn_responder(
        ep_out,
        dir_out.path().to_path_buf(),
        shared_wid(),
        actor_out,
        out_tx,
        // The hub dials OUT to this responder concurrently with its inbound storm.
        &[id_hub.node_id()],
    );

    // Several inbound dialers, each with its own disjoint batch, all targeting
    // the hub. Launch order shuffled deterministically.
    let n_inbound: u32 = 6;
    let mut inbound_order: Vec<u32> = (0..n_inbound).collect();
    rng.shuffle(&mut inbound_order);

    let mut handles = Vec::new();
    let mut inbound_expected: BTreeSet<String> = BTreeSet::new();
    let hub_addr = ep_hub.addr();

    for w in inbound_order {
        let dir_w = tempfile::tempdir().expect("inbound tempdir");
        let id_w = fresh_identity(dir_w.path(), &format!("in{w}"));
        // Authorize this inbound dialer against the hub before it dials in.
        test_support::authorize_peer(dir_hub.path(), id_w.node_id());
        let actor_w = ActorId::new();
        let nodes = seed_ops(dir_w.path(), actor_w, 3);
        for n in &nodes {
            inbound_expected.insert(n.to_string());
        }
        let ep_w = test_support::bind_sync_endpoint(&id_w)
            .await
            .expect("bind inbound");
        let hub_addr = hub_addr.clone();
        let dir_w_path = dir_w.path().to_path_buf();
        handles.push(tokio::spawn(async move {
            let _keep = (dir_w, ep_w.clone());
            let (tx, _rx) = mpsc::channel::<()>();
            tokio::time::timeout(
                STEP_TIMEOUT,
                test_support::run_delta_sync(
                    &ep_w,
                    hub_addr,
                    &dir_w_path,
                    &shared_wid(),
                    actor_w,
                    tx,
                ),
            )
            .await
            .expect("inbound dial timed out")
            .expect("inbound dial");
        }));
    }

    // CONCURRENTLY: the hub initiates an OUTBOUND delta-sync on its own endpoint
    // while the inbound storm is landing.
    let ep_hub_out = ep_hub.clone();
    let dir_hub_path = dir_hub.path().to_path_buf();
    let outbound = tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel::<()>();
        tokio::time::timeout(
            STEP_TIMEOUT,
            test_support::run_delta_sync(
                &ep_hub_out,
                out_addr,
                &dir_hub_path,
                &shared_wid(),
                actor_hub,
                tx,
            ),
        )
        .await
        .expect("hub outbound timed out")
        .expect("hub outbound");
    });

    for h in handles {
        h.await.expect("inbound task panicked");
    }
    outbound.await.expect("outbound task panicked");

    // The hub must hold its own ops + every inbound batch + the outbound peer's
    // ops (pulled during the concurrent outbound dial).
    let mut expected = inbound_expected.clone();
    for n in hub_nodes.iter().chain(out_nodes.iter()) {
        expected.insert(n.to_string());
    }
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_hub.path(), actor_hub, &expected)
        }),
        "hub must converge across concurrent inbound + outbound dials on one endpoint"
    );

    // No glued/torn line on any actor file in the hub workspace.
    let hub_ops_dir = dir_hub.path().join("ops");
    let storage = JsonlStorage::open(hub_ops_dir.clone(), actor_hub).expect("open hub storage");
    let actors: BTreeSet<ActorId> = storage
        .all_ops()
        .expect("hub all_ops")
        .into_iter()
        .map(|op| op.actor)
        .collect();
    for actor in actors {
        assert_every_line_is_one_json_value(&hub_ops_dir, actor);
    }
}
