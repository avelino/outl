//! Phase 2 — peer-snapshot adoption must NOT lose local input.
//!
//! A freshly paired device with no snapshot of its own adopts a peer's
//! `snap-<peer>.bin` (via `read_best_from_disk`) to skip a full replay of a
//! huge op log. The load-bearing guarantee, and the exact thing the user
//! asked to protect: this device's OWN ops — written to `ops-<self>.jsonl`,
//! sitting above the peer snapshot's per-actor cutoff — are replayed on top
//! of the adopted tree and are never lost.

use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn create(g: &HlcGenerator, actor: ActorId, node: NodeId, parent: NodeId) -> LogOp {
    LogOp {
        ts: g.next(),
        actor,
        op: Op::Create {
            node,
            parent,
            position: Fractional::first(),
        },
    }
}

#[test]
fn adopting_peer_snapshot_preserves_local_ops() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let ops_dir = root.join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();

    let peer = ActorId::new(); // "desktop"
    let me = ActorId::new(); // "mobile" — this device
    let gp = HlcGenerator::new(peer);
    let gm = HlcGenerator::new(me);

    let page = NodeId::from_slug("home");
    let peer_block = NodeId::new();
    let my_block = NodeId::new();

    // Peer builds the page + a block, then writes its snapshot
    // (`snap-<peer>.bin` under `<root>/.outl/snapshots/`).
    {
        let storage = JsonlStorage::open(ops_dir.clone(), peer).unwrap();
        let mut ws =
            Workspace::open_with_storage(peer, Box::new(storage), Some(root.clone())).unwrap();
        ws.apply(create(&gp, peer, page, NodeId::root())).unwrap();
        ws.apply(create(&gp, peer, peer_block, page)).unwrap();
        ws.save_snapshot().unwrap();
    }

    // This device authors a local block straight into ITS OWN ops file —
    // the peer snapshot never saw it (the note the user typed on the phone).
    {
        let mut storage = JsonlStorage::open(ops_dir.clone(), me).unwrap();
        storage.append_op(&create(&gm, me, my_block, page)).unwrap();
    }

    // This device boots with NO snapshot of its own → `read_best_from_disk`
    // adopts `snap-<peer>.bin`, then the per-actor delta replays this
    // device's ops on top.
    let storage = JsonlStorage::open(ops_dir.clone(), me).unwrap();
    let ws = Workspace::open_with_storage(me, Box::new(storage), Some(root.clone())).unwrap();

    assert_eq!(
        ws.tree().parent(page),
        Some(NodeId::root()),
        "page root came from the adopted peer snapshot"
    );
    assert_eq!(
        ws.tree().parent(peer_block),
        Some(page),
        "peer's block came from the adopted snapshot"
    );
    assert_eq!(
        ws.tree().parent(my_block),
        Some(page),
        "LOCAL block preserved via the per-actor delta replay — NOT lost"
    );
}

/// The convergence guard: adopting a peer snapshot whose per-actor cutoff
/// excludes a local `Move` that sorts BELOW a `Move` folded into the body
/// must still materialize the SAME tree a full replay would — the exact
/// divergence the crdt-invariant-checker caught.
///
/// Op set (a, b under root):
///   D = Move(b under a), ts=100, mobile
///   Y = Move(a under b), ts=200, desktop
/// Full replay: D applies (root→a→b), Y is a cycle no-op → `root→a→b`
/// (parent(a)=root, parent(b)=a). A snapshot folding in Y (body: root→b→a)
/// with D in the delta would, WITHOUT the guard, keep root→b→a — divergent.
/// The guard sees D.ts(100) ≤ max body HLC(200), bails to full replay, and
/// the tree matches.
#[test]
fn peer_snapshot_cycle_reorder_matches_full_replay() {
    let tmp = TempDir::new().unwrap();
    let root_dir = tmp.path().to_path_buf();
    let ops_dir = root_dir.join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();

    let setup = ActorId::new();
    let desk = ActorId::new();
    let mob = ActorId::new();
    let a = NodeId::new();
    let b = NodeId::new();

    let mv = |ts: u64, actor: ActorId, node: NodeId, new_parent: NodeId| LogOp {
        ts: Hlc::new(ts, 0, actor),
        actor,
        op: Op::Move {
            node,
            new_parent,
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
    };
    let create = |ts: u64, actor: ActorId, node: NodeId| LogOp {
        ts: Hlc::new(ts, 0, actor),
        actor,
        op: Op::Create {
            node,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };

    // Shared history + the desktop's high-HLC Move go to disk.
    {
        let mut s = JsonlStorage::open(ops_dir.clone(), setup).unwrap();
        s.append_op(&create(10, setup, a)).unwrap();
        s.append_op(&create(20, setup, b)).unwrap();
    }
    {
        let mut s = JsonlStorage::open(ops_dir.clone(), desk).unwrap();
        s.append_op(&mv(200, desk, a, b)).unwrap(); // Y: a under b
    }

    // Desktop writes its snapshot BEFORE the mobile op exists — so the
    // cutoff covers {setup, desk} but not mob, and the body folds in Y.
    {
        let storage = JsonlStorage::open(ops_dir.clone(), desk).unwrap();
        let mut ws =
            Workspace::open_with_storage(desk, Box::new(storage), Some(root_dir.clone())).unwrap();
        ws.save_snapshot().unwrap();
    }

    // Mobile's low-HLC Move lands after the snapshot.
    {
        let mut s = JsonlStorage::open(ops_dir.clone(), mob).unwrap();
        s.append_op(&mv(100, mob, b, a)).unwrap(); // D: b under a
    }

    // Mobile boots: adopts snap-desk, the guard sees D below the body's max
    // HLC → full replay. Tree must match the full-replay result.
    let storage = JsonlStorage::open(ops_dir.clone(), mob).unwrap();
    let ws = Workspace::open_with_storage(mob, Box::new(storage), Some(root_dir.clone())).unwrap();

    assert_eq!(
        ws.tree().parent(a),
        Some(NodeId::root()),
        "a stays under root (full-replay result), not under b (divergent snapshot result)"
    );
    assert_eq!(
        ws.tree().parent(b),
        Some(a),
        "b under a (full-replay result)"
    );
}
