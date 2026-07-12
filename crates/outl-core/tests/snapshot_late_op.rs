//! Regression for #156 (Half 2): a late op from an actor the snapshot
//! never saw must survive a snapshot boot.
//!
//! The snapshot's replay cutoff used to be a single global `Hlc`, and the
//! delta replay was `ops_since(cutoff)` filtering `hlc > cutoff` strictly.
//! That silently dropped a legitimately-low-HLC op from a *different*
//! actor delivered after the snapshot: its HLC sits below the global
//! cutoff (which tracks the high-water mark of the actor that took the
//! snapshot), so it never replays and vanishes from the materialized tree
//! even though it's durably in storage.
//!
//! The fix makes the cutoff a per-actor vector clock: an op is replayed
//! when its HLC is above the cutoff **of its own actor** (or that actor
//! is absent from the snapshot entirely, in which case all of its ops
//! replay). This test builds the exact scenario with hand-rolled HLCs so
//! actor B's op is below actor A's high-water mark.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn hlc(physical_ms: u64, actor: ActorId) -> Hlc {
    Hlc {
        physical_ms,
        logical: 0,
        actor,
    }
}

#[test]
fn late_low_hlc_op_from_unseen_actor_survives_snapshot_boot() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join("ops");
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    let mut ws = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    // No in-band snapshots; we drive `save_snapshot` explicitly.
    ws.set_snapshot_policy(false, 0);

    // Actor A creates a node at a HIGH physical time.
    let n_a = NodeId::new();
    let ts_a = hlc(10_000, actor_a);
    ws.apply(LogOp {
        ts: ts_a,
        actor: actor_a,
        op: Op::Create {
            node: n_a,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    })
    .unwrap();

    // Snapshot now: its cutoff records A's high-water mark (physical 10_000).
    // At this point storage holds only A's op, so the snapshot's per-actor
    // cutoff is `{A: 10_000}` — actor B is entirely absent.
    ws.save_snapshot().unwrap();
    assert!(ws.tree().contains(n_a));
    drop(ws);

    // Actor B's op arrives AFTER the snapshot via sync: written straight
    // into `ops-B.jsonl` by a B-scoped storage (never through A's
    // `Workspace::apply`, which refuses foreign-actor ops). Its HLC is LOW
    // (physical 5 — B was offline / its clock is behind), sitting below A's
    // global high-water mark of 10_000.
    let n_b = NodeId::new();
    let ts_b = hlc(5, actor_b);
    {
        let mut storage_b = JsonlStorage::open(ops_dir.clone(), actor_b).unwrap();
        storage_b
            .append_op(&LogOp {
                ts: ts_b,
                actor: actor_b,
                op: Op::Create {
                    node: n_b,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .unwrap();
    }

    // Reboot A via the snapshot + delta path. It reads ops-A.jsonl AND
    // ops-B.jsonl; B's op must NOT be dropped.
    let ws2 = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();

    assert!(
        ws2.tree().contains(n_a),
        "actor A's snapshotted node must be present"
    );
    assert!(
        ws2.tree().contains(n_b),
        "actor B's late low-HLC op must survive snapshot boot (per-actor cutoff), not be dropped below A's global high-water mark"
    );
}
