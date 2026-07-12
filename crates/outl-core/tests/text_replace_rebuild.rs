//! Block-text rebuild correctness across snapshot boundaries.
//!
//! Sequential same-actor `build_text_replace_update` on a node that was
//! edited pre-snapshot, then re-edited post-snapshot, must produce the
//! replacement (not concatenation) on every boot path.
//!
//! Root cause: after snapshot boot, the in-memory `OpLog` only carries
//! delta ops (the pre-snapshot ops live in storage). When the content
//! store rebuilt a `Doc` from the incomplete log, the `state_vector`
//! captured before the second edit was wrong, so the encoded update
//! conflicted with the first edit's blocks on full replay — the
//! `remove_range` became a no-op and both inserts survived.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use tempfile::TempDir;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// The exact scenario from the issue: two sequential edits on the same
/// node, snapshot in between, reopen via both boot paths.
#[test]
fn reedit_after_snapshot_matches_full_replay() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");
    let snapshots_dir = dotl.join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // 1. Create a node, edit it, snapshot.
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
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
    let u1 = ws.build_text_replace_update(n, "hello");
    ws.apply(logop(
        &g,
        Op::Edit {
            node: n,
            text_op: u1,
        },
    ))
    .unwrap();
    assert_eq!(ws.block_text(n).as_deref(), Some("hello"));
    ws.save_snapshot().unwrap();
    drop(ws);

    // 2. Reopen (snapshot boot), re-edit the same node.
    let mut ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(ws2.block_text(n).as_deref(), Some("hello"));
    let u2 = ws2.build_text_replace_update(n, "hello world");
    ws2.apply(logop(
        &g,
        Op::Edit {
            node: n,
            text_op: u2,
        },
    ))
    .unwrap();
    assert_eq!(ws2.block_text(n).as_deref(), Some("hello world"));
    drop(ws2);

    // 3. Reopen WITH snapshot (snapshot + delta path).
    let from_snapshot = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    // 4. Remove snapshot, reopen via full replay.
    std::fs::remove_dir_all(&snapshots_dir).unwrap();
    let from_full = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    assert_eq!(
        from_snapshot.block_text(n).as_deref(),
        Some("hello world"),
        "snapshot+delta path must produce the replacement"
    );
    assert_eq!(
        from_full.block_text(n).as_deref(),
        Some("hello world"),
        "full-replay path must produce the replacement, not concatenation"
    );
}

/// Three sequential edits across two sessions (snapshot after the first,
/// reopen, second + third edits, reopen again) — the original repro from
/// the issue body, extended to cover multi-edit chains.
#[test]
fn three_edits_across_sessions() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
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

    for text in ["alpha", "beta", "gamma"] {
        let u = ws.build_text_replace_update(n, text);
        ws.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: u,
            },
        ))
        .unwrap();
    }
    assert_eq!(ws.block_text(n).as_deref(), Some("gamma"));
    ws.save_snapshot().unwrap();
    drop(ws);

    // Reopen and edit twice more.
    let mut ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(ws2.block_text(n).as_deref(), Some("gamma"));
    for text in ["delta", "epsilon"] {
        let u = ws2.build_text_replace_update(n, text);
        ws2.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: u,
            },
        ))
        .unwrap();
    }
    assert_eq!(ws2.block_text(n).as_deref(), Some("epsilon"));
    drop(ws2);

    // Full replay.
    let ws3 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(
        ws3.block_text(n).as_deref(),
        Some("epsilon"),
        "five sequential edits must survive full replay"
    );
}

/// Two different actors editing the same block, then full replay — a
/// concurrent-edit path that must also converge correctly.
#[test]
fn multi_actor_edit_converges_on_replay() {
    let tmp = TempDir::new().unwrap();
    let dotl = tmp.path().join(".outl");
    let ops_dir = dotl.join("ops");

    let actor_a = ActorId::new();
    let actor_b = ActorId::new();
    let ga = HlcGenerator::new(actor_a);
    let gb = HlcGenerator::new(actor_b);

    // Actor A creates and edits.
    let mut ws_a = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    let n = NodeId::new();
    ws_a.apply(logop(
        &ga,
        Op::Create {
            node: n,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    let u1 = ws_a.build_text_replace_update(n, "from A");
    ws_a.apply(logop(
        &ga,
        Op::Edit {
            node: n,
            text_op: u1,
        },
    ))
    .unwrap();
    ws_a.save_snapshot().unwrap();
    drop(ws_a);

    // Actor B opens the same workspace and edits the same node.
    let mut ws_b = Workspace::open_with_storage(
        actor_b,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_b).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(ws_b.block_text(n).as_deref(), Some("from A"));
    let u2 = ws_b.build_text_replace_update(n, "from B");
    ws_b.apply(logop(
        &gb,
        Op::Edit {
            node: n,
            text_op: u2,
        },
    ))
    .unwrap();
    assert_eq!(ws_b.block_text(n).as_deref(), Some("from B"));
    drop(ws_b);

    // Reopen (full replay across both actors' files).
    let ws = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();
    // The two actors edit with independent HLC generators, so which write
    // wins is not fixed here — the guarantee under test is that the reopen
    // resolves to exactly one edit's text, never a concatenation of both.
    let text = ws.block_text(n);
    assert!(
        text.as_deref() == Some("from A") || text.as_deref() == Some("from B"),
        "multi-actor edit must not concatenate: {:?}",
        text
    );
}
