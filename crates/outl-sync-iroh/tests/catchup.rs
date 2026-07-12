//! Catch-up loop integration tests over real QUIC on loopback.
//!
//! Split out of `integration.rs` (file-size guard) so the catch-up suite — the
//! periodic re-sync of peers paired/changed after boot — has a focused home:
//!
//! 1. `catch_up_syncs_peer_paired_after_boot` — a device paired AFTER the
//!    transport booted gets its full op-log history pulled by the periodic loop,
//!    not just new gossip ops.
//! 2. `catch_up_redials_after_workspace_id_change` — when the joiner adopts the
//!    host's workspace id at runtime, the loop clears its per-session `synced`
//!    dedup and re-dials, so edits made after the single immediate post-pair sync
//!    still converge without a restart (resume-sync item 2).
//! 3. `catch_up_resyncs_peer_after_interval` — the maintenance re-sync: a peer
//!    synced cleanly is re-dialed once its success goes stale, so a later edit on
//!    the other device propagates with NO gossip and NO external signal. This is
//!    the safety net that makes convergence independent of the real-time path
//!    (the "edit on one device never reached the other" report).
//!
//! Shared seed/read/wait helpers live in `common/mod.rs`.

mod common;

use std::collections::BTreeSet;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use outl_core::id::ActorId;
use outl_core::WorkspaceId;
use outl_sync_iroh::test_support;

use common::{
    created_nodes_on_disk, disk_has_all_nodes, fresh_identity, seed_ops, shared_wid, wait_until,
    STEP_TIMEOUT,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_syncs_peer_paired_after_boot() {
    // Regression for the reported bug: B's transport is already running, then a
    // NEW device (A) is paired AFTER boot (its PeerEntry shows up in B's peer
    // list only later). The boot-time connect never saw A, so without the
    // periodic catch-up loop B would only ever get A's *new* gossip ops and
    // never A's existing op-log history. The catch-up loop must pull all of it.
    use std::sync::Mutex;

    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // A already has N ops in its log; B starts completely empty.
    let n: u32 = 5;
    let a_nodes = seed_ops(dir_a.path(), actor_a, n);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    // A is the responder (online, waiting), exactly like an already-paired
    // device the new device will reach.
    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let a_addr = ep_a.addr();
    let _router_a = test_support::spawn_responder(
        ep_a,
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        a_ready_tx,
        &[id_b.node_id()],
    );

    // B brings its catch-up loop up FIRST, with an empty peer list. The shared
    // slot is `None` at boot and is filled with A's addr only *after* the loop
    // is already running — that's the "paired after boot" condition.
    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();

    let peer_slot: Arc<Mutex<Option<iroh::EndpointAddr>>> = Arc::new(Mutex::new(None));
    let resolver_slot = peer_slot.clone();
    let resolver = move || {
        resolver_slot
            .lock()
            .expect("peer slot poisoned")
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    };

    let b_workspace = dir_b.path().to_path_buf();
    let catchup = tokio::spawn(async move {
        // Short tick so the test doesn't wait the production 8s.
        test_support::run_catch_up_loop(
            ep_b,
            Duration::from_millis(200),
            // No maintenance re-sync during the test: this test asserts the
            // first catch-up of a peer paired after boot, not the periodic
            // re-pull. A long interval keeps it to a single dial.
            Duration::from_secs(3600),
            resolver,
            b_workspace,
            shared_wid(),
            actor_b,
            b_ready_tx,
            None,
        )
        .await;
    });

    // Let the loop spin a couple of empty ticks (peer not known yet).
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        created_nodes_on_disk(dir_b.path(), actor_b).is_empty(),
        "B should still be empty before A is paired"
    );

    // "Pair" A now: publish its loopback addr into the slot the resolver reads.
    *peer_slot.lock().expect("peer slot poisoned") = Some(a_addr);

    // Within a couple of catch-up ticks, B must hold all N of A's ops.
    let a_node_strs: BTreeSet<String> = a_nodes.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &a_node_strs)
        }),
        "B should catch up A's full history after pairing post-boot"
    );

    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);
    assert_eq!(
        b_after.len(),
        n as usize,
        "B holds exactly A's N ops after catch-up"
    );

    catchup.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_redials_after_workspace_id_change() {
    // Regression for the "sync stops after pairing adoption" bug (resume-sync
    // item 2): once the catch-up loop syncs a peer cleanly, it marks the peer in
    // its per-session `synced` set and stops re-dialing — trusting gossip for
    // live updates. When the joiner ADOPTS the host's workspace id at runtime,
    // that single immediate post-pair sync is the only one that ever runs; later
    // edits on the host never reach the joiner. The fix wires the id-change
    // signal into the catch-up loop so it clears `synced` and re-dials every
    // peer. This test proves: sync once, host gains MORE ops, fire the id-change
    // signal, and the joiner pulls the new ops WITHOUT a restart.
    use std::sync::Mutex;

    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // A (host/responder) starts with a first batch; B starts empty.
    let first: u32 = 3;
    let a_first = seed_ops(dir_a.path(), actor_a, first);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let a_addr = ep_a.addr();
    // Both sides present the SAME workspace id: the id-change signal in this test
    // models adoption finishing (the catch-up dedup reset), not a mismatch — the
    // responder must accept throughout so we isolate the re-dial behaviour.
    let _router_a = test_support::spawn_responder(
        ep_a,
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        a_ready_tx,
        &[id_b.node_id()],
    );

    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();

    // A is known to the resolver from the start (paired), so the loop syncs the
    // first batch on its first tick and marks A `synced`.
    let peer_slot: Arc<Mutex<Option<iroh::EndpointAddr>>> = Arc::new(Mutex::new(Some(a_addr)));
    let resolver_slot = peer_slot.clone();
    let resolver = move || {
        resolver_slot
            .lock()
            .expect("peer slot poisoned")
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    };

    // The broadcast channel the production transport wires from pairing adoption
    // into the catch-up loop. The test holds the sender to fire the signal.
    let (wid_tx, wid_rx) = tokio::sync::broadcast::channel::<WorkspaceId>(8);

    let b_workspace = dir_b.path().to_path_buf();
    let catchup = tokio::spawn(async move {
        test_support::run_catch_up_loop(
            ep_b,
            Duration::from_millis(200),
            // Long interval so the re-dial under test comes from the workspace-id
            // change clearing the dedup, NOT from a maintenance re-sync — keeps
            // the assertion isolated to the adoption path.
            Duration::from_secs(3600),
            resolver,
            b_workspace,
            shared_wid(),
            actor_b,
            b_ready_tx,
            Some(wid_rx),
        )
        .await;
    });

    // Step 1: B catches up A's first batch and marks A synced.
    let first_strs: BTreeSet<String> = a_first.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &first_strs)
        }),
        "B should sync A's first batch on the first catch-up tick"
    );

    // Step 2: A gains MORE ops. The catch-up loop has A in `synced`, so it will
    // NOT re-dial on its own — these ops stay stranded until a signal arrives.
    let second: u32 = 4;
    let a_second = seed_ops(dir_a.path(), actor_a, second);
    let second_strs: BTreeSet<String> = a_second.iter().map(|n| n.to_string()).collect();

    // Give the loop several ticks to (not) pull the new ops without a signal.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert!(
        !disk_has_all_nodes(dir_b.path(), actor_b, &second_strs),
        "without an id-change signal the loop must NOT have re-dialed the synced peer"
    );

    // Step 3: fire the workspace-id-change signal (adoption completed). The loop
    // clears `synced` and re-dials A under the new id, pulling the second batch.
    wid_tx
        .send(shared_wid())
        .expect("at least one catch-up receiver is live");

    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &second_strs)
        }),
        "after the id-change signal the loop must re-dial and pull A's new ops"
    );

    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);
    assert_eq!(
        b_after.len(),
        (first + second) as usize,
        "B holds both of A's batches after the post-adoption re-dial"
    );

    catchup.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catch_up_resyncs_peer_after_interval() {
    // Regression for the "edit on one device never reached the other" report.
    // After the first clean sync the loop used to mark the peer `synced` for the
    // whole session and never re-dial — so a later edit only ever propagated if
    // the gossip announce happened to cross (it didn't: flaky cross-network iroh,
    // and the GUI clients weren't even calling `announce_local_ops`). The fix is
    // the maintenance re-sync: a peer goes stale after `resync_after` and is
    // re-dialed. This test proves the new ops land with NO gossip and NO
    // id-change signal — only the periodic re-pull.
    use std::sync::Mutex;

    let dir_a = tempfile::tempdir().expect("A tempdir");
    let dir_b = tempfile::tempdir().expect("B tempdir");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // A (responder) starts with a first batch; B starts empty.
    let first: u32 = 3;
    let a_first = seed_ops(dir_a.path(), actor_a, first);

    let id_a = fresh_identity(dir_a.path(), "a");
    let id_b = fresh_identity(dir_b.path(), "b");

    let (a_ready_tx, _a_ready_rx) = mpsc::channel::<()>();
    let ep_a = test_support::bind_sync_endpoint(&id_a)
        .await
        .expect("bind A endpoint");
    let a_addr = ep_a.addr();
    let _router_a = test_support::spawn_responder(
        ep_a,
        dir_a.path().to_path_buf(),
        shared_wid(),
        actor_a,
        a_ready_tx,
        &[id_b.node_id()],
    );

    let ep_b = test_support::bind_sync_endpoint(&id_b)
        .await
        .expect("bind B endpoint");
    let (b_ready_tx, _b_ready_rx) = mpsc::channel::<()>();

    // A is known from the start, so the loop syncs the first batch on tick one
    // and stamps A's last-synced time.
    let peer_slot: Arc<Mutex<Option<iroh::EndpointAddr>>> = Arc::new(Mutex::new(Some(a_addr)));
    let resolver_slot = peer_slot.clone();
    let resolver = move || {
        resolver_slot
            .lock()
            .expect("peer slot poisoned")
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    };

    let b_workspace = dir_b.path().to_path_buf();
    let catchup = tokio::spawn(async move {
        // Short tick AND short re-sync window: a peer goes stale ~400ms after its
        // last clean sync, so the maintenance re-dial fires within the test.
        test_support::run_catch_up_loop(
            ep_b,
            Duration::from_millis(150),
            Duration::from_millis(400),
            resolver,
            b_workspace,
            shared_wid(),
            actor_b,
            b_ready_tx,
            None,
        )
        .await;
    });

    // Step 1: B catches up A's first batch and stamps A as freshly synced.
    let first_strs: BTreeSet<String> = a_first.iter().map(|n| n.to_string()).collect();
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &first_strs)
        }),
        "B should sync A's first batch on the first catch-up tick"
    );

    // Step 2: A gains MORE ops. No gossip, no id-change signal exists in this
    // harness — the ONLY path to B is the maintenance re-sync once A goes stale.
    let second: u32 = 4;
    let a_second = seed_ops(dir_a.path(), actor_a, second);
    let second_strs: BTreeSet<String> = a_second.iter().map(|n| n.to_string()).collect();

    // Step 3: within a few stale-and-re-dial cycles, the new ops must land.
    assert!(
        wait_until(STEP_TIMEOUT, || {
            disk_has_all_nodes(dir_b.path(), actor_b, &second_strs)
        }),
        "the maintenance re-sync must re-dial the stale peer and pull A's new ops \
         with no gossip and no id-change signal"
    );

    let b_after = created_nodes_on_disk(dir_b.path(), actor_b);
    assert_eq!(
        b_after.len(),
        (first + second) as usize,
        "B holds both batches purely via the periodic maintenance re-sync"
    );

    catchup.abort();
}
