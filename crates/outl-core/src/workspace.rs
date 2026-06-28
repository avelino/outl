//! Workspace — the top-level container.
//!
//! A `Workspace` owns the materialized tree, the in-memory op log, the
//! storage backend, and the per-block Yrs `Doc`s that hold block text.
//! CLI and TUI consume the workspace; they don't reach into `Tree` or
//! `OpLog` directly.
//!
//! This module exposes a deliberately minimal API:
//!
//! - [`Workspace::open_in_memory`] / [`Workspace::open_with_storage`]
//! - [`Workspace::apply`] — accept an op, route to tree + storage + Yrs
//! - read-only accessors for the tree, log, and block text
//!
//! Higher-level methods (page CRUD, journal CRUD, block edit shortcuts)
//! live in `outl-actions` and `outl-cli`, not here.

use crate::content::ContentStore;
use crate::id::{ActorId, NodeId};
use crate::log::OpLog;
use crate::op::{LogOp, Op};
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;
use std::collections::HashMap;
use std::path::PathBuf;

/// Errors a workspace may surface to its caller.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Underlying storage failure.
    #[error(transparent)]
    Storage(#[from] StorageError),
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
        let storage = crate::storage::MemoryStorage::new();
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
        // Pass 1: structural. Apply every op to the tree (`Edit` is a
        // no-op there) and the log. Text is materialized in pass 2 so the
        // open-time memory peak stays at a single live `Doc` instead of
        // one per block — that peak is what jetsam was killing on iOS.
        let ops = ws.storage.all_ops()?;
        for op in ops {
            ws.tree.apply_op(&mut ws.log, op);
        }

        // Pass 2: text. Group `Edit` ops by node (indices only, no byte
        // copies), then rebuild one `Doc` at a time, materialize its
        // string, and drop it before moving on.
        let mut edits_by_node: HashMap<NodeId, Vec<usize>> = HashMap::new();
        for (i, logged) in ws.log.iter().enumerate() {
            if let Op::Edit { node, .. } = &logged.op {
                edits_by_node.entry(*node).or_default().push(i);
            }
        }
        for (node, indices) in edits_by_node {
            ws.content.materialize(
                node,
                indices.iter().filter_map(|&i| match ws.log.get(i) {
                    Some(LogOp {
                        op: Op::Edit { text_op, .. },
                        ..
                    }) => Some(text_op.as_slice()),
                    _ => None,
                }),
            );
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
            // Merge the update into the block's text. The Doc is rebuilt
            // from the log here, before this op is appended below, so the
            // merge sees the prior state.
            self.content.merge_update(*node, &self.log, text_op);
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

    /// Number of live Yrs `Doc`s currently resident in the content cache.
    ///
    /// Test-only window into the bound that keeps large vaults under the
    /// iOS memory limit (issue #108).
    #[cfg(test)]
    fn live_doc_count(&self) -> usize {
        self.content.live_doc_count()
    }

    /// Build a Yrs `update_v1` payload that, once wrapped in
    /// `Op::Edit { node, text_op }` and pushed through [`Self::apply`],
    /// rewrites the block's text to `new_text` exactly.
    ///
    /// **Side effect:** the workspace's own content `Doc` for `node`
    /// is mutated in-place to match `new_text`. The returned update
    /// encodes exactly that mutation, captured via the Doc's state
    /// vector. Yrs is idempotent on `apply_update`, so the subsequent
    /// `Workspace::apply(LogOp::Edit { … })` is a no-op on the local
    /// Doc but still appends to the log and propagates to peers.
    ///
    /// Returns an empty `Vec` when the requested change is a no-op.
    pub fn build_text_replace_update(&mut self, node: NodeId, new_text: &str) -> Vec<u8> {
        let current = self.block_text(node).unwrap_or_default();
        if current == new_text {
            return Vec::new();
        }
        self.content.replace_text(node, &self.log, new_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::DOC_CACHE_CAP;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use yrs::{Doc, Text, Transact};

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

        // Use a shared directory: open, write, close, reopen.
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let storage1 = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
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

        let storage2 = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
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

    /// Edit one block, then create + edit a fresh one. Reopening rebuilds
    /// the text of both from the log even though neither Doc was kept
    /// resident across the close.
    #[test]
    fn reopen_rebuilds_text_without_resident_docs() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let storage = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

        let mut ids = Vec::new();
        for i in 0..5 {
            let n = NodeId::new();
            ids.push(n);
            ws.apply(make_op(
                &g,
                Op::Create {
                    node: n,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            ))
            .unwrap();
            let update = ws.build_text_replace_update(n, &format!("block {i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: update,
                },
            ))
            .unwrap();
        }
        drop(ws);

        let storage = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
        let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();
        for (i, n) in ids.iter().enumerate() {
            assert_eq!(
                ws2.block_text(*n).as_deref(),
                Some(format!("block {i}").as_str())
            );
        }
        // Pass 2 materializes strings and drops every Doc, so nothing is
        // resident right after open — the whole point of issue #108.
        assert_eq!(ws2.live_doc_count(), 0);
    }

    /// The live-Doc cache never grows past its cap, no matter how many
    /// distinct blocks get edited in a session.
    #[test]
    fn doc_cache_is_bounded() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();

        let over = DOC_CACHE_CAP + 50;
        for i in 0..over {
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
            let update = ws.build_text_replace_update(n, &format!("b{i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: update,
                },
            ))
            .unwrap();
            assert!(ws.live_doc_count() <= DOC_CACHE_CAP);
        }
        assert_eq!(ws.live_doc_count(), DOC_CACHE_CAP);
    }

    /// A block evicted from the cache is rebuilt from the log on the next
    /// edit, preserving its text instead of losing history.
    #[test]
    fn evicted_block_rebuilds_from_log() {
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();

        let first = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: first,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(first, "hello");
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: first,
                text_op: update,
            },
        ))
        .unwrap();

        // Edit enough other blocks to evict `first` from the cache.
        for i in 0..DOC_CACHE_CAP + 10 {
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
            let u = ws.build_text_replace_update(n, &format!("x{i}"));
            ws.apply(make_op(
                &g,
                Op::Edit {
                    node: n,
                    text_op: u,
                },
            ))
            .unwrap();
        }
        assert!(!ws.content.is_cached(first));

        // Rebuild on demand: appending to the evicted block keeps "hello".
        let update = ws.build_text_replace_update(first, "hello world");
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: first,
                text_op: update,
            },
        ))
        .unwrap();
        assert_eq!(ws.block_text(first).as_deref(), Some("hello world"));
    }
}
