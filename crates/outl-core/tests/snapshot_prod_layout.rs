//! Regression for #156 (Half 1): under the PRODUCTION on-disk layout
//! (`ops_dir = <root>/ops`, **not** `<root>/.outl/ops`), a snapshot
//! written by the in-band background writer must be READ on the next
//! boot.
//!
//! Before the fix the workspace wrote snapshots to `<root>/.outl/snapshots`
//! but `JsonlStorage` derived its own read dir from `ops_dir.parent()` =
//! `<root>/snapshots`, so writer and reader never met and snapshot boot
//! was inert in production. Every pre-existing snapshot test used
//! `<root>/.outl/ops`, which makes the two dirs coincide — masking the
//! bug. This test pins the real layout the TUI / desktop / CLI pass
//! (`Paths::at` → `<root>/ops`).

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

/// Six blocks (create + edit each = 12 applies). A single background
/// snapshot is triggered by setting the threshold to exactly the apply
/// count, so the one snapshot on disk is deterministic and complete
/// (no racing workers). Then the op log is wiped: if boot still recovers
/// the blocks, it can only have read the snapshot — a full replay would
/// see zero ops.
#[test]
fn snapshot_written_in_prod_layout_is_read_on_boot() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // PRODUCTION layout: ops at <root>/ops (what Paths::at / the TUI /
    // tauri-shared pass), NOT <root>/.outl/ops.
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    // Threshold == total applies below → exactly one background snapshot,
    // fired by the final apply, capturing the full materialized state.
    ws.set_snapshot_policy(true, 12);

    let mut nodes = Vec::new();
    for i in 0..6 {
        let n = NodeId::new();
        nodes.push(n);
        ws.apply(logop(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let u = ws.build_text_replace_update(n, &format!("block {i}"));
        ws.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: u,
            },
        ))
        .unwrap();
    }
    ws.wait_for_snapshots();

    // The snapshot must land under <root>/.outl/snapshots — the local,
    // non-synced cache dir — regardless of where the op log lives.
    let snap = root
        .join(".outl")
        .join("snapshots")
        .join(format!("snap-{actor}.bin"));
    assert!(
        snap.exists(),
        "background writer must land the snapshot in <root>/.outl/snapshots under prod layout"
    );
    drop(ws);

    // Wipe the op log. A full replay now sees zero ops; only a snapshot
    // read can bring the six blocks back.
    std::fs::remove_dir_all(&ops_dir).unwrap();

    let ws2 = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    assert_eq!(
        ws2.tree().node_count(),
        6,
        "snapshot boot must recover the tree in prod layout (op log was wiped)"
    );
    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(
            ws2.block_text(*n).as_deref(),
            Some(format!("block {i}").as_str()),
            "block text must survive via the snapshot"
        );
    }
}
