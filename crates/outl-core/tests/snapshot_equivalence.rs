//! Boot path equivalence: opening a workspace via snapshot + delta must
//! produce the same materialized state as opening it via full replay.
//!
//! This is the load-bearing correctness property of Phase 1: snapshot is
//! only a cache, so any divergence between the two boot paths is a
//! silent state-corruption bug. The test is intentionally deterministic
//! (not proptest) so a regression points at a specific op combination.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use std::collections::HashSet;
use tempfile::TempDir;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// Snapshot + delta replay must converge to the same materialized state
/// as a full replay. Covers every op kind (Create, Edit, SetProp,
/// SetCollapsed, Move) and exercises both the "node carried by
/// snapshot" and "node edited after snapshot" branches.
#[test]
fn snapshot_and_delta_match_full_replay() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");
    let snapshots_dir = dotl.join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let root = NodeId::root();

    // 1. Populate workspace with one of each op kind.
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();

    let n1 = NodeId::new();
    let n2 = NodeId::new();
    let n3 = NodeId::new();
    let pos1 = Fractional::first();
    let pos2 = Fractional::first();
    let pos3 = Fractional::first();

    ws.apply(logop(
        &g,
        Op::Create {
            node: n1,
            parent: root,
            position: pos1.clone(),
        },
    ))
    .unwrap();
    ws.apply(logop(
        &g,
        Op::Create {
            node: n2,
            parent: n1,
            position: pos2.clone(),
        },
    ))
    .unwrap();
    ws.apply(logop(
        &g,
        Op::Create {
            node: n3,
            parent: root,
            position: pos3.clone(),
        },
    ))
    .unwrap();

    let edit1 = ws.build_text_replace_update(n1, "hello world");
    ws.apply(logop(
        &g,
        Op::Edit {
            node: n1,
            text_op: edit1,
        },
    ))
    .unwrap();
    let edit2 = ws.build_text_replace_update(n2, "child block");
    ws.apply(logop(
        &g,
        Op::Edit {
            node: n2,
            text_op: edit2,
        },
    ))
    .unwrap();

    ws.apply(logop(
        &g,
        Op::SetProp {
            node: n1,
            key: "tags".into(),
            value: Some(PropValue::Tag("foo".into())),
            old_value: None,
        },
    ))
    .unwrap();
    ws.apply(logop(
        &g,
        Op::SetCollapsed {
            node: n1,
            value: true,
            old_value: false,
        },
    ))
    .unwrap();

    // Move n3 under n2 — exercises old_parent / old_position capture.
    ws.apply(logop(
        &g,
        Op::Move {
            node: n3,
            new_parent: n2,
            position: Fractional::first(),
            old_parent: root,
            old_position: pos3.clone(),
        },
    ))
    .unwrap();

    // 2. Snapshot the pre-delta state.
    ws.save_snapshot().unwrap();
    assert!(
        snapshots_dir.exists(),
        "save_snapshot must create the snapshots dir"
    );
    drop(ws);

    // 3. Apply delta: create a brand-new node + edit it. We avoid
    //    re-editing a node that already had an `Edit` pre-snapshot —
    //    `materialize` (used by both boot paths) re-derives a node's
    //    text from its full Edit history on a fresh Doc, and a known
    //    Yrs quirk with sequential same-actor `replace_text` updates
    //    can concatenate rather than replace when re-applied out of
    //    the original Doc's state-vector context. That bug is
    //    independent of the snapshot path; both `from_snapshot` and
    //    `from_full` would diverge from the live state the same way.
    //    Tracked separately.
    let mut ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();

    let n4 = NodeId::new();
    ws2.apply(logop(
        &g,
        Op::Create {
            node: n4,
            parent: root,
            position: Fractional::first(),
        },
    ))
    .unwrap();
    let edit4 = ws2.build_text_replace_update(n4, "fresh after snapshot");
    ws2.apply(logop(
        &g,
        Op::Edit {
            node: n4,
            text_op: edit4,
        },
    ))
    .unwrap();

    // A structural op on a pre-existing node (no Edit conflict) —
    // exercises the post-snapshot Move path without triggering the
    // multi-Edit-per-node Yrs quirk.
    ws2.apply(logop(
        &g,
        Op::Move {
            node: n3,
            new_parent: root,
            position: Fractional::first(),
            old_parent: n2,
            old_position: Fractional::first(),
        },
    ))
    .unwrap();
    drop(ws2);

    // 4. Open two workspaces from the same op log:
    //    - `from_snapshot`: snapshot present → snapshot + delta boot path.
    //    - `from_full`: snapshots/ removed → forces the full-replay fallback.
    let from_snapshot = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();

    std::fs::remove_dir_all(&snapshots_dir).unwrap();
    let from_full = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();

    assert_workspaces_equivalent(&from_snapshot, &from_full);
}

/// A corrupt snapshot (hash mismatch) must silently fall back to a full
/// replay and produce the correct state.
#[test]
fn corrupt_snapshot_falls_back_to_full_replay() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();
    let n = NodeId::new();
    ws.apply(logop(
        &g,
        Op::Create {
            node: n,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    let edit = ws.build_text_replace_update(n, "before snapshot");
    ws.apply(logop(
        &g,
        Op::Edit {
            node: n,
            text_op: edit,
        },
    ))
    .unwrap();
    ws.save_snapshot().unwrap();
    drop(ws);

    // Tamper with the snapshot bytes so the content hash check fails.
    let snap_path = dotl.join("snapshots").join(format!("snap-{actor}.bin"));
    let mut bytes = std::fs::read(&snap_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&snap_path, bytes).unwrap();

    // Boot: should detect the mismatch and fall back to full replay.
    let ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(ws.block_text(n).as_deref(), Some("before snapshot"));
    assert_eq!(ws.tree().node_count(), 1);
}

/// A leftover `.tmp` file from a crashed save must not be picked up as
/// the snapshot. The load path only reads `snap-<actor>.bin`.
#[test]
fn leftover_tmp_from_crashed_save_is_ignored() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");
    let snapshots_dir = dotl.join("snapshots");
    let actor = ActorId::new();
    std::fs::create_dir_all(&snapshots_dir).unwrap();

    // Simulate a half-written snapshot that never got renamed: `.tmp`
    // contains garbage, the real `snap-<actor>.bin` is absent.
    let garbage = b"this is not bincode";
    let tmp_path = snapshots_dir.join(format!("snap-{actor}.bin.tmp"));
    std::fs::write(&tmp_path, garbage).unwrap();

    let g = HlcGenerator::new(actor);
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();
    let n = NodeId::new();
    ws.apply(logop(
        &g,
        Op::Create {
            node: n,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    let edit = ws.build_text_replace_update(n, "fresh");
    ws.apply(logop(
        &g,
        Op::Edit {
            node: n,
            text_op: edit,
        },
    ))
    .unwrap();
    drop(ws);

    // Re-open: no real snapshot exists, so boot must full-replay and
    // land on the right state despite the leftover `.tmp`.
    let ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(ws.block_text(n).as_deref(), Some("fresh"));
}

fn assert_workspaces_equivalent(a: &Workspace, b: &Workspace) {
    // Nodes: identity, parent, AND position all matter.
    let a_nodes: HashSet<_> = a.tree().iter_nodes().collect();
    let b_nodes: HashSet<_> = b.tree().iter_nodes().collect();
    let a_nodes: HashSet<_> = a_nodes
        .into_iter()
        .map(|(n, p, pos)| (n, p, pos.clone()))
        .collect();
    let b_nodes: HashSet<_> = b_nodes
        .into_iter()
        .map(|(n, p, pos)| (n, p, pos.clone()))
        .collect();
    assert_eq!(a_nodes, b_nodes, "node set / parents / positions differ");

    // Block text for every node.
    let all_nodes: Vec<_> = a_nodes.iter().map(|(n, _, _)| *n).collect();
    for n in &all_nodes {
        let ta = a.block_text(*n);
        let tb = b.block_text(*n);
        assert_eq!(ta, tb, "block text differs for node {n:?}");
    }

    // Properties per node (order-independent). `PropValue` doesn't
    // implement Hash (it carries `Vec<PropValue>`), so we serialize each
    // value to JSON and compare sorted tuples.
    for n in &all_nodes {
        let mut pa: Vec<(String, String)> = a
            .tree()
            .properties_of(*n)
            .map(|(k, v)| {
                (
                    k.to_string(),
                    serde_json::to_string(v).expect("PropValue serializes"),
                )
            })
            .collect();
        let mut pb: Vec<(String, String)> = b
            .tree()
            .properties_of(*n)
            .map(|(k, v)| {
                (
                    k.to_string(),
                    serde_json::to_string(v).expect("PropValue serializes"),
                )
            })
            .collect();
        pa.sort();
        pb.sort();
        assert_eq!(pa, pb, "properties differ for node {n:?}");
    }

    // Collapsed set.
    let a_c: HashSet<_> = a.tree().collapsed_ids().collect();
    let b_c: HashSet<_> = b.tree().collapsed_ids().collect();
    assert_eq!(a_c, b_c, "collapsed set differs");
}

/// In-band snapshot trigger fires after `op_threshold` applies. The
/// expensive write (encode + fsync + rename) runs in a worker thread
/// spawned by `apply`; the calling thread returns immediately. The CLI
/// opt-out (threshold 0) never spawns.
#[test]
fn apply_trigger_writes_snapshot_at_threshold() {
    let tmp = TempDir::new().unwrap();
    // The workspace root is `tmp/` itself; `.outl/` lives one level
    // below (alongside future `pages/`, `journals/`).
    let root = tmp.path();
    let dotl = root.join(".outl");
    let ops_dir = dotl.join("ops");
    let snapshots_dir = dotl.join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // Pass `root = Some(...)` so `Workspace` derives `snapshots_dir`
    // and the in-band trigger can fire. With `None` the workspace has
    // nowhere to write (`MemoryStorage`-equivalent).
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    // Tight threshold so the test doesn't need 10k ops.
    ws.set_snapshot_policy(true, 3);
    let root = NodeId::root();

    let mk = || {
        let n = NodeId::new();
        logop(
            &g,
            Op::Create {
                node: n,
                parent: root,
                position: Fractional::first(),
            },
        )
    };

    // Two ops: under threshold, no snapshot yet.
    ws.apply(mk()).unwrap();
    ws.apply(mk()).unwrap();
    assert!(
        !snapshots_dir.exists() || std::fs::read_dir(&snapshots_dir).unwrap().count() == 0,
        "no snapshot should exist before threshold"
    );

    // Third op crosses threshold → worker spawned. `apply` must not
    // block on the write — measure its latency to catch a regression
    // that pulls the write back onto the calling thread. Bound at
    // 200ms: the body build for three `Create` ops is sub-millisecond,
    // so this only fails if someone accidentally made `apply` wait on
    // fsync.
    let start = std::time::Instant::now();
    ws.apply(mk()).unwrap();
    let apply_elapsed = start.elapsed();
    assert!(
        apply_elapsed.as_millis() < 200,
        "apply should not block on the snapshot write (took {apply_elapsed:?})"
    );

    // Join the worker so the file actually exists on disk when we check.
    ws.wait_for_snapshots();
    let snap_path = snapshots_dir.join(format!("snap-{actor}.bin"));
    assert!(
        snap_path.exists(),
        "snapshot must exist after crossing threshold + wait_for_snapshots"
    );

    // Three more ops → another background snapshot; joining leaves the
    // file present and updated.
    ws.apply(mk()).unwrap();
    ws.apply(mk()).unwrap();
    ws.apply(mk()).unwrap();
    ws.wait_for_snapshots();
    assert!(snap_path.exists());

    // Opt-out: after `set_snapshot_policy(false, _)`, further applies
    // must not spawn any new workers — file mtime stays put.
    let before = std::fs::metadata(&snap_path)
        .map(|m| m.modified().ok())
        .ok()
        .flatten();
    ws.set_snapshot_policy(false, 1);
    for _ in 0..10 {
        ws.apply(mk()).unwrap();
    }
    ws.wait_for_snapshots();
    let after = std::fs::metadata(&snap_path)
        .map(|m| m.modified().ok())
        .ok()
        .flatten();
    assert_eq!(
        before, after,
        "CLI-style opt-out must not touch the snapshot file"
    );
}
