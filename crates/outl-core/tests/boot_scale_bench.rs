//! Boot-path scale benchmark (#37 driver).
//!
//! Measures `Workspace::open_with_storage` time AND peak resident
//! memory at four op-log scales, with and without a Phase 1 snapshot.
//!
//! Run all rows (slow — generation is fsync-bound, ~4 ms/op):
//!
//! ```sh
//! cargo test -p outl-core --release --test boot_scale_bench -- --ignored --nocapture
//! ```
//!
//! Run one scale in isolation (preferred for partial data):
//!
//! ```sh
//! cargo test -p outl-core --release --test boot_scale_bench boot_scale_100k -- --ignored --nocapture
//! ```
//!
//! ## Why this exists
//!
//! Phase 1 snapshot (#128) speeds up boot but **does not reduce RSS** —
//! the resident `OpLog` still holds every `Op::Edit`'s `text_op` bytes,
//! and `JsonlStorage` mirrors them in its own cache. This bench
//! quantifies where the cost lands as the op log grows, so we can
//! decide whether per-page shards (#37) are urgent or can wait, and
//! pick the right layer to fix.
//!
//! Output is plain text on stderr so `--nocapture` shows the numbers
//! inline. Not run in CI — slow and the `snapshot_equivalence` property
//! test is the load-bearing correctness guard.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::JsonlStorage;
use outl_core::Workspace;
use std::time::Instant;
use tempfile::TempDir;

#[derive(Clone, Copy)]
struct Scale {
    label: &'static str,
    nodes: usize,
    edits_per_node: usize,
}

impl Scale {
    fn total_ops(&self) -> usize {
        // 1 Create + N Edits per node.
        self.nodes * (1 + self.edits_per_node)
    }
}

// 50k ≈ 1k pages × 50 ops (the "sync.md:520" soft ceiling).
// 100k/250k/500k trace the curve past it.
// 1M omitted — generation cost crosses ~10 min on an SSD and the
// curve is already legible at 500k. Add it back when shards land.
const SCALES: &[Scale] = &[
    Scale {
        label: "50k",
        nodes: 1_000,
        edits_per_node: 49,
    },
    Scale {
        label: "100k",
        nodes: 1_000,
        edits_per_node: 99,
    },
    Scale {
        label: "250k",
        nodes: 2_000,
        edits_per_node: 124,
    },
    Scale {
        label: "500k",
        nodes: 2_000,
        edits_per_node: 249,
    },
];

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// Peak resident set size of this process, in bytes.
///
/// `getrusage(RUSAGE_SELF).ru_maxrss` reports bytes on macOS and KB on
/// Linux — the cfg below normalises both to bytes. Returns 0 on any
/// syscall failure (the boot-time measurement is still meaningful).
fn peak_rss_bytes() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    if rc != 0 {
        return 0;
    }
    let raw = ru.ru_maxrss.max(0) as u64;
    #[cfg(target_os = "linux")]
    {
        raw * 1024
    }
    #[cfg(not(target_os = "linux"))]
    {
        raw
    }
}

/// Run one scale row: generate, snapshot, boot both ways, print the row.
///
/// Shared by the per-scale tests below so each can run in isolation —
/// generation is the slow step (~4 ms/op fsync-bound on an SSD), and a
/// single all-scales test used to time out before reaching the bigger
/// rows. Per-scale tests give partial data when the runner is killed.
fn run_scale(scale: Scale) {
    eprintln!();
    eprintln!(
        "boot_scale {} — {} ops, {} nodes. Phase 1 snapshot speeds boot but does NOT reduce RSS.",
        scale.label,
        scale.total_ops(),
        scale.nodes
    );
    eprintln!(
        "{:>6} | {:>9} | {:>9} | {:>10} | {:>10} | {:>10} | {:>10} | {:>9}",
        "scale", "ops", "gen", "boot_snap", "boot_full", "rss_snap", "rss_full", "snap_size"
    );

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
    // Disable in-band trigger so the snapshot we measure is the one
    // we write explicitly at the end (otherwise it fires partway
    // through generation and confuses the measurement).
    ws.set_snapshot_policy(false, 0);

    // Generation: 1 Create + N Edits per node. The `Edit` carries a
    // tiny placeholder update — the boot-path cost we're measuring
    // is structural replay + cache load, which doesn't depend on
    // real text content. fsync-per-op dominates wall time.
    let start_gen = Instant::now();
    for _ in 0..scale.nodes {
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
        for _ in 0..scale.edits_per_node {
            ws.apply(logop(
                &g,
                Op::Edit {
                    node: n,
                    text_op: vec![1, 2, 3, 4],
                },
            ))
            .unwrap();
        }
    }
    let gen_time = start_gen.elapsed();

    // Save snapshot synchronously (the path TUI / desktop / mobile
    // take on shutdown).
    ws.save_snapshot().unwrap();
    let snap_size = std::fs::metadata(snapshots_dir.join(format!("snap-{actor}.bin")))
        .map(|m| m.len())
        .unwrap_or(0);
    let node_count = ws.tree().node_count();
    drop(ws);

    // Boot path 1: with snapshot present.
    let start = Instant::now();
    let ws1 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    let boot_snap = start.elapsed();
    let rss_snap = peak_rss_bytes();
    let nodes_snap = ws1.tree().node_count();
    drop(ws1);

    // Boot path 2: snapshot removed, full replay fallback.
    let _ = std::fs::remove_dir_all(&snapshots_dir);
    let start = Instant::now();
    let ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    let boot_full = start.elapsed();
    let rss_full = peak_rss_bytes();
    let nodes_full = ws2.tree().node_count();
    drop(ws2);

    // Sanity: every path must land on the same node count, otherwise
    // the bench is comparing different states.
    assert_eq!(
        nodes_snap, nodes_full,
        "node count diverged between snapshot and full-replay boot at {}",
        scale.label
    );
    assert_eq!(
        nodes_snap, node_count,
        "node count drifted between generation and boot at {}",
        scale.label
    );

    eprintln!(
        "{:>6} | {:>9} | {:>6.1?} | {:>10.2?} | {:>10.2?} | {:>6} MB | {:>6} MB | {:>6} KB",
        scale.label,
        scale.total_ops(),
        gen_time,
        boot_snap,
        boot_full,
        rss_snap / 1024 / 1024,
        rss_full / 1024 / 1024,
        snap_size / 1024,
    );
    eprintln!(
        "  rss_snap = {} MB, rss_full = {} MB (peak RSS of this process after boot)",
        rss_snap / 1024 / 1024,
        rss_full / 1024 / 1024
    );
    eprintln!(
        "  speedup: {:.2}x (saved {:?} per boot)",
        boot_full.as_secs_f64() / boot_snap.as_secs_f64().max(0.000_001),
        boot_full.checked_sub(boot_snap).unwrap_or_default()
    );
}

#[test]
#[ignore]
fn boot_scale_50k() {
    run_scale(SCALES[0]);
}

#[test]
#[ignore]
fn boot_scale_100k() {
    run_scale(SCALES[1]);
}

#[test]
#[ignore]
fn boot_scale_250k() {
    run_scale(SCALES[2]);
}

#[test]
#[ignore]
fn boot_scale_500k() {
    run_scale(SCALES[3]);
}
