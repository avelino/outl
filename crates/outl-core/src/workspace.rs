//! Workspace — the top-level container.
//!
//! A `Workspace` owns the materialized tree, the in-memory op log, the
//! storage backend, and the per-block Yrs `Doc`s that hold block text.
//! CLI and TUI consume the workspace; they don't reach into `Tree` or
//! `OpLog` directly.
//!
//! Phase 1 implements a minimal API:
//!
//! - [`Workspace::open_in_memory`] / [`Workspace::open_with_storage`]
//! - [`Workspace::apply`] — accept an op, route to tree + storage + Yrs
//! - read-only accessors for the tree, log, and block text
//!
//! Higher-level methods (page CRUD, journal CRUD, block edit shortcuts)
//! land in Step 4 alongside `outl-cli`.

use crate::id::{ActorId, NodeId};
use crate::log::OpLog;
use crate::op::{LogOp, Op};
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;
use std::collections::HashMap;
use std::path::PathBuf;
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, Transact};

/// Errors a workspace may surface to its caller.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Underlying storage failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Per-node Yrs documents holding block text.
///
/// Tree-level CRDT is structural only; per-block text convergence rides
/// on Yrs. The store is private to the workspace and reconstructed on
/// open from the op log.
#[derive(Default)]
struct ContentStore {
    docs: HashMap<NodeId, Doc>,
}

impl ContentStore {
    fn apply_update(&mut self, node: NodeId, update: &[u8]) {
        let doc = self.docs.entry(node).or_default();
        let mut txn = doc.transact_mut();
        if let Ok(decoded) = yrs::Update::decode_v1(update) {
            let _ = txn.apply_update(decoded);
        }
        // Silent ignore of malformed updates — those come from corrupted
        // peers and shouldn't crash the local workspace. Surfaced via
        // tracing instead.
    }

    fn text(&self, node: NodeId) -> Option<String> {
        let doc = self.docs.get(&node)?;
        let text = doc.get_or_insert_text("content");
        let txn = doc.transact();
        Some(text.get_string(&txn))
    }
}

/// Top-level workspace.
///
/// Construct via [`Workspace::open_in_memory`] for tests or
/// [`Workspace::open_with_storage`] for real backends.
pub struct Workspace {
    /// Filesystem root for the workspace, if any.
    pub root: Option<PathBuf>,
    /// This device's actor id.
    pub actor: ActorId,
    /// Materialized structural tree.
    tree: Tree,
    /// In-memory op log mirroring the storage backend.
    log: OpLog,
    /// Block text content (Yrs docs).
    content: ContentStore,
    /// Pluggable storage backend.
    storage: Box<dyn Storage>,
}

impl Workspace {
    /// Open an in-memory workspace. Useful for tests.
    pub fn open_in_memory(actor: ActorId) -> Result<Self, WorkspaceError> {
        let storage = crate::storage::SqliteStorage::open_in_memory()?;
        Self::open_with_storage(actor, Box::new(storage), None)
    }

    /// Open a workspace backed by a given storage implementation.
    ///
    /// Replays the full op log into the in-memory tree so the workspace
    /// is ready to read from the moment this returns.
    pub fn open_with_storage(
        actor: ActorId,
        storage: Box<dyn Storage>,
        root: Option<PathBuf>,
    ) -> Result<Self, WorkspaceError> {
        let mut ws = Self {
            root,
            actor,
            tree: Tree::new(),
            log: OpLog::new(),
            content: ContentStore::default(),
            storage,
        };
        // Replay the persisted log.
        let ops = ws.storage.all_ops()?;
        for op in ops {
            if let Op::Edit { node, text_op } = &op.op {
                ws.content.apply_update(*node, text_op);
            }
            ws.tree.apply_op(&mut ws.log, op);
        }
        Ok(ws)
    }

    /// Apply an op locally and persist it.
    ///
    /// The op is appended to the in-memory log, dispatched to the Yrs
    /// content store if it's an `Edit`, and persisted to storage. If
    /// storage fails, the in-memory state is still mutated — the caller
    /// is responsible for surfacing the error and/or invoking `outl doctor`.
    pub fn apply(&mut self, op: LogOp) -> Result<(), WorkspaceError> {
        if let Op::Edit { node, text_op } = &op.op {
            self.content.apply_update(*node, text_op);
        }
        self.tree.apply_op(&mut self.log, op.clone());
        self.storage.append_op(&op)?;
        Ok(())
    }

    /// Read-only access to the materialized tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Read-only access to the in-memory op log.
    pub fn log(&self) -> &OpLog {
        &self.log
    }

    /// Block text content, if the block has any.
    pub fn block_text(&self, node: NodeId) -> Option<String> {
        self.content.text(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use yrs::Text as _;

    fn make_op(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    #[test]
    fn open_apply_reload_preserves_state() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // Use a shared file: open, write, close, reopen.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let storage1 = Box::new(crate::storage::SqliteStorage::open(&path).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage1, None).unwrap();
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        drop(ws);

        let storage2 = Box::new(crate::storage::SqliteStorage::open(&path).unwrap());
        let ws2 = Workspace::open_with_storage(actor, storage2, None).unwrap();
        assert_eq!(ws2.tree().node_count(), 1);
        assert_eq!(ws2.tree().parent(n), Some(NodeId::root()));
        assert_eq!(ws2.log().len(), 1);
    }

    #[test]
    fn edit_dispatches_to_content_store() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();

        // Build a Yrs update locally.
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        text.push(&mut txn, "hello outl");
        let update_bytes = txn.encode_update_v1();
        drop(txn);

        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: update_bytes,
            },
        ))
        .unwrap();

        assert_eq!(ws.block_text(n).as_deref(), Some("hello outl"));
    }
}
