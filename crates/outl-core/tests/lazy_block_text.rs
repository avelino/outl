//! Regression for #179: full-replay boot defers block-text materialization.
//!
//! Boot used to rebuild every block's Yrs `Doc` and materialize its string
//! up front — O(all blocks), a major freeze on large snapshotless vaults
//! (a 66k-block mobile install-clean). Now the full-replay path leaves the
//! tree fully materialized but rebuilds each block's string lazily on the
//! first `block_text` read.
//!
//! The load-bearing property: a lazily-materialized block reads back
//! byte-identical to an eager (snapshot) boot of the exact same op log.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
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

/// A block not touched since a full-replay (lazy) boot reads back
/// byte-identical to the same block after an eager snapshot boot of the
/// same op log.
#[test]
fn lazy_full_replay_block_text_matches_eager_snapshot_boot() {
    let tmp = TempDir::new().unwrap();
    let ops_dir = tmp.path().join(".outl").join("ops");
    let snapshots_dir = tmp.path().join(".outl").join("snapshots");
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let root = NodeId::root();

    // Build a workspace with many distinct blocks, then snapshot it so we
    // have an eager reference boot to compare against.
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(outl_core::storage::JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    let mut ids = Vec::new();
    let mut expected = Vec::new();
    for i in 0..256 {
        let n = NodeId::new();
        ids.push(n);
        ws.apply(logop(
            &g,
            Op::Create {
                node: n,
                parent: root,
                position: Fractional::first(),
            },
        ))
        .unwrap();
        // Mix of multi-byte content and an intentionally empty edit so the
        // lazy path is exercised against non-trivial and empty strings.
        let text = if i % 17 == 0 {
            String::new()
        } else {
            format!("block {i} — café ☃ {i}")
        };
        let update = ws.build_text_replace_update(n, &text);
        ws.apply(logop(
            &g,
            Op::Edit {
                node: n,
                text_op: update,
            },
        ))
        .unwrap();
        expected.push(text);
    }
    // Create-only node: never edited, must read back as `None` on both paths.
    let bare = NodeId::new();
    ws.apply(logop(
        &g,
        Op::Create {
            node: bare,
            parent: root,
            position: Fractional::first(),
        },
    ))
    .unwrap();

    ws.save_snapshot().unwrap();
    drop(ws);

    // Eager boot: snapshot present → hydrates the full text map up front.
    let eager = Workspace::open_with_storage(
        actor,
        Box::new(outl_core::storage::JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Lazy boot: remove the snapshot so boot full-replays and defers text.
    std::fs::remove_dir_all(&snapshots_dir).unwrap();
    let lazy = Workspace::open_with_storage(
        actor,
        Box::new(outl_core::storage::JsonlStorage::open(ops_dir, actor).unwrap()),
        Some(tmp.path().to_path_buf()),
    )
    .unwrap();

    // Tree structure is fully materialized on both paths.
    assert_eq!(lazy.tree().node_count(), eager.tree().node_count());

    for (n, want) in ids.iter().zip(expected.iter()) {
        // Lazy read produces the exact written text...
        assert_eq!(
            lazy.block_text(*n).as_deref(),
            Some(want.as_str()),
            "lazy text wrong for {n:?}"
        );
        // ...byte-identical to the eager (snapshot) boot.
        assert_eq!(
            lazy.block_text(*n),
            eager.block_text(*n),
            "lazy vs eager mismatch for {n:?}"
        );
    }

    // A never-edited block has no text on either path (no phantom "").
    assert_eq!(lazy.block_text(bare), None);
    assert_eq!(eager.block_text(bare), None);
}
