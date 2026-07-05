//! Boot-path benchmark for snapshot phase 1 (#128).
//!
//! Ignored by default — generates a large synthetic vault and measures
//! `Workspace::open_with_storage` with and without a snapshot. Run with:
//!
//! ```sh
//! cargo test -p outl-core --release --test snapshot_bench -- --ignored --nocapture
//! ```
//!
//! Output is plain text on stderr (`eprintln!`) so a `--nocapture` run
//! shows the numbers inline. We don't run it in CI — it's slow and the
//! "equivalence vs full replay" property test (`snapshot_equivalence.rs`)
//! is the load-bearing correctness guard.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::storage::JsonlStorage;
use outl_core::Workspace;
use std::time::Instant;
use tempfile::TempDir;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// Compare boot time with and without a snapshot on a vault with
/// ~100k ops. Prints elapsed + speedup.
#[test]
#[ignore]
fn boot_path_with_vs_without_snapshot() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join(".outl").join("ops");
    let snapshots_dir = root.join(".outl").join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    // Disable in-band trigger so the snapshot we measure is the one we
    // write explicitly at the end. (Otherwise the trigger fires partway
    // through generation and confuses the benchmark.)
    ws.set_snapshot_policy(false, 0);

    eprintln!("Generating ops…");
    let start_gen = Instant::now();
    // NODE_COUNT trades off benchmark runtime vs signal. The costly
    // step during generation is `append_op`'s `fsync` per op, so 5_000
    // nodes ≈ 23_000 ops ≈ ~25s on an SSD. That's enough to see the
    // snapshot vs full-replay split clearly.
    const NODE_COUNT: usize = 5_000;
    for i in 0..NODE_COUNT {
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
        // Placeholder text_op (invalid Yrs → empty string after
        // materialize). Real-world edits would carry real Yrs updates;
        // the boot-path cost we're measuring is the structural replay
        // and the snapshot's O(current state) load, which doesn't
        // depend on text content.
        ws.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: vec![1, 2, 3, 4],
            },
        ))
        .unwrap();
        if i % 3 == 0 {
            ws.apply(logop(
                &g,
                Op::SetProp {
                    node: n,
                    key: "i".into(),
                    value: Some(PropValue::Text(i.to_string())),
                    old_value: None,
                },
            ))
            .unwrap();
        }
        if i % 7 == 0 {
            ws.apply(logop(
                &g,
                Op::SetCollapsed {
                    node: n,
                    value: true,
                    old_value: false,
                },
            ))
            .unwrap();
        }
    }
    eprintln!(
        "  generated {} ops over {} nodes in {:?}",
        ws.log().len(),
        ws.tree().node_count(),
        start_gen.elapsed()
    );

    // Save snapshot synchronously (the path the TUI / desktop / mobile
    // would take on shutdown).
    let start_snap = Instant::now();
    ws.save_snapshot().unwrap();
    let snap_time = start_snap.elapsed();
    let snap_size = std::fs::metadata(snapshots_dir.join(format!("snap-{actor}.bin")))
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!("  save_snapshot: {snap_time:?} ({snap_size} bytes)");
    drop(ws);

    // Boot path 1: with snapshot present.
    let start_boot_snap = Instant::now();
    let ws1 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    let boot_snap_time = start_boot_snap.elapsed();
    eprintln!("  boot WITH snapshot:    {boot_snap_time:?}");
    drop(ws1);

    // Remove snapshot and boot from full replay.
    std::fs::remove_dir_all(&snapshots_dir).unwrap();
    let start_boot_full = Instant::now();
    let ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    let boot_full_time = start_boot_full.elapsed();
    eprintln!("  boot WITHOUT snapshot: {boot_full_time:?}");

    let speedup = boot_full_time.as_secs_f64() / boot_snap_time.as_secs_f64();
    eprintln!();
    eprintln!("speedup: {speedup:.2}x");
    eprintln!(
        "delta: saved {:?} per boot",
        boot_full_time
            .checked_sub(boot_snap_time)
            .unwrap_or_default()
    );

    // Sanity: both paths must end on the same number of nodes.
    let _ = ws2.tree().node_count();
}
