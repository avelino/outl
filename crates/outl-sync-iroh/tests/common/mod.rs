//! Shared helpers for the `outl-sync-iroh` loopback integration tests.
//!
//! Both `integration.rs` (pairing + delta-sync + mesh) and `catchup.rs`
//! (catch-up loop + post-adoption re-dial) `mod common;` this file so the seed /
//! read / wait helpers live in exactly one place. Keeping them here is what let
//! `integration.rs` drop back under the file-size guard's stop threshold when the
//! catch-up suite grew.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::LogOp;
use outl_core::WorkspaceId;
use outl_sync_iroh::IrohIdentity;

/// Generous ceiling for any single network step on loopback.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// A fixed workspace id shared by both sides of a sync test.
///
/// The serve side validates `SyncRequest.workspace_id` against the local id and
/// rejects a mismatch, so two nodes that should sync as one workspace must
/// present the SAME id — even though their local temp-dir paths differ. Tests
/// that want two nodes to reconcile call this on both sides.
pub fn shared_wid() -> WorkspaceId {
    WorkspaceId::from_raw("TESTWORKSPACE0000000000000000")
}

/// Build a fresh in-memory identity backed by a temp file path that doesn't
/// exist yet (so `load_or_generate` generates + persists a new keypair).
pub fn fresh_identity(dir: &Path, name: &str) -> Arc<IrohIdentity> {
    let path = dir.join(format!("{name}-identity.key"));
    Arc::new(IrohIdentity::load_or_generate(&path).expect("generate identity"))
}

/// Write `count` real `Op::Create` ops authored by `actor` into
/// `<workspace>/ops/ops-<actor>.jsonl` via the production `JsonlStorage`.
///
/// Timestamps are anchored at the current wall clock so they comfortably pass
/// the receiver's "24h in the future" sanity gate, and stay monotonic per op.
/// Appends (does not truncate), so calling it twice on the same workspace grows
/// the log — used by the post-adoption re-dial test to add a second batch.
pub fn seed_ops(workspace_root: &Path, actor: ActorId, count: u32) -> Vec<NodeId> {
    let ops_dir = workspace_root.join("ops");
    let mut storage = JsonlStorage::open(ops_dir, actor).expect("open storage");
    let base_ms = now_ms();
    let mut nodes = Vec::with_capacity(count as usize);
    for i in 0..count {
        let node = NodeId::new();
        nodes.push(node);
        let op = LogOp {
            ts: Hlc::new(base_ms + i as u64, i, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        };
        storage.append_op(&op).expect("append op");
    }
    nodes
}

/// Current wall-clock time in milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_millis() as u64
}

/// Read every actor present in `<workspace>/ops/` and return the full set of
/// `(actor, node)` pairs materialized from the `Op::Create` ops on disk.
///
/// Going through `JsonlStorage::all_ops` proves the bytes round-tripped through
/// real serialization, exactly like a workspace reload would see them.
pub fn created_nodes_on_disk(
    workspace_root: &Path,
    reader_actor: ActorId,
) -> BTreeSet<(String, String)> {
    let ops_dir = workspace_root.join("ops");
    let storage = JsonlStorage::open(ops_dir, reader_actor).expect("open storage for read");
    storage
        .all_ops()
        .expect("load all ops")
        .into_iter()
        .filter_map(|log| match log.op {
            Op::Create { node, .. } => Some((log.actor.to_string(), node.to_string())),
            _ => None,
        })
        .collect()
}

/// True once every node id in `expected` appears in `<workspace>/ops/`.
pub fn disk_has_all_nodes(
    workspace_root: &Path,
    reader_actor: ActorId,
    expected: &BTreeSet<String>,
) -> bool {
    let present: BTreeSet<String> = created_nodes_on_disk(workspace_root, reader_actor)
        .into_iter()
        .map(|(_actor, node)| node)
        .collect();
    expected.iter().all(|n| present.contains(n))
}

/// Poll `cond` until it's true or `timeout` elapses. Sleeps between polls so the
/// responder's async ingest has time to flush to disk.
pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
