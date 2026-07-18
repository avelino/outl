//! Refutation test for the assumed silent-loss in RFC #137 #2.
//!
//! The claim under test: "under PerPage, a peer's ops are silently lost
//! because `reload_per_page` only reads the local actor's shard."
//!
//! That claim looks at `reload_per_page` in isolation. The real workspace
//! topology under PerPage is a **Global catch-all `self.storage`** (opened
//! with `JsonlStorage::open`, scope Global — it scans every
//! `ops-<actor>.jsonl`, including peers) **plus** local per-page shards.
//! `all_ops_combined` merges both. Sync writes every peer's ops into the
//! Global `ops-<peer>.jsonl` (there is no PageScope awareness in
//! outl-sync-iroh), so peer ops live exactly where the Global storage reads.
//!
//! This test reproduces that split — local block in the shard, peer block in
//! the Global log — and asserts the combined boot sees both. If a peer op
//! were dropped, `parent(b_block)` would be `None`.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::{JsonlStorage, PageScope, Storage};
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
fn per_page_boot_sees_peer_ops_from_global_log() {
    let tmp = TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();

    let a = ActorId::new();
    let b = ActorId::new();
    let ga = HlcGenerator::new(a);
    let gb = HlcGenerator::new(b);

    // Both peers agree on the same deterministic page root for "home".
    let root = NodeId::from_slug("home");
    let a_block = NodeId::new();
    let b_block = NodeId::new();

    // Peer B's ops arrive via sync into the Global log `ops-<b>.jsonl`:
    // B created the page root and a block under it.
    {
        let mut peer_global = JsonlStorage::open(ops_dir.clone(), b).unwrap();
        peer_global
            .append_op(&create(&gb, b, root, NodeId::root()))
            .unwrap();
        peer_global
            .append_op(&create(&gb, b, b_block, root))
            .unwrap();
    }

    // Local actor A authors a block under the SAME root into its per-page
    // shard `ops/<a>/home.jsonl`.
    {
        let mut shard = JsonlStorage::open_with_scope_cap(
            ops_dir.clone(),
            a,
            PageScope::PerPage("home".into()),
            0,
        )
        .unwrap();
        shard.append_op(&create(&ga, a, a_block, root)).unwrap();
    }

    // Boot as actor A: Global catch-all + the registered home shard.
    let global = JsonlStorage::open(ops_dir.clone(), a).unwrap();
    let mut ws =
        Workspace::open_with_storage(a, Box::new(global), Some(tmp.path().to_path_buf())).unwrap();
    ws.set_snapshot_policy(false, 0);
    let shard =
        JsonlStorage::open_with_scope_cap(ops_dir.clone(), a, PageScope::PerPage("home".into()), 0)
            .unwrap();
    ws.register_page_storage("home", Box::new(shard));
    ws.register_page_root(root, "home");
    ws.reboot_with_all_storages().unwrap();

    // The whole point: both blocks are present under the root. The peer's
    // block came from the Global log the PerPage boot path *does* read via
    // `self.storage` — nothing is silently lost.
    assert_eq!(
        ws.tree().parent(root),
        Some(NodeId::root()),
        "page root materialized"
    );
    assert_eq!(
        ws.tree().parent(a_block),
        Some(root),
        "local shard op present"
    );
    assert_eq!(
        ws.tree().parent(b_block),
        Some(root),
        "peer Global op present — NOT silently lost under PerPage"
    );
}
