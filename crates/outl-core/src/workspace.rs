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
use crate::op::{op_node, LogOp, Op};
use crate::snapshot::{self, SnapshotBody};
use crate::storage::{Storage, StorageError};
use crate::tree::Tree;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::thread::JoinHandle;
use tracing::{debug, warn};

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
    /// Pluggable storage backend (Global scope — the legacy
    /// single-file-per-actor layout). Per-page shards live in
    /// `page_storages`.
    storage: Box<dyn Storage>,
    /// Per-page storage backends (Phase B of RFC #137). Keyed by page
    /// slug. Empty for workspaces that haven't migrated to per-page
    /// shards. When non-empty, `apply` routes each op to the storage
    /// that owns the op's node.
    page_storages: HashMap<String, Box<dyn Storage>>,
    /// `NodeId → slug` map for page roots. Populated by the client
    /// (which reads sidecars via `outl-md`) via
    /// [`Self::register_page_root`]. `apply` walks the parent chain
    /// from an op's node up to a page root, then uses this map to
    /// find the slug and route to the right `page_storages` entry.
    page_root_to_slug: HashMap<NodeId, String>,
    /// `<root>/.outl/snapshots` when `root` is set, `None` for
    /// in-memory workspaces. Background snapshot writes go straight
    /// here — they don't go through `storage` (the snapshot is a local
    /// cache, not part of the source-of-truth op log).
    snapshots_dir: Option<PathBuf>,
    /// Background snapshot writers still in flight. `apply` drains
    /// finished handles on every trigger (cheap, non-blocking) so the
    /// list stays bounded; `wait_for_snapshots` joins the rest.
    snapshot_workers: Vec<JoinHandle<()>>,
    /// Number of ops applied since the last successful snapshot write.
    /// `apply` increments this and spawns a snapshot worker once it
    /// crosses `snapshot_threshold`. Reset to `0` on every spawn and
    /// on policy change.
    ops_since_snapshot: u32,
    /// Trigger threshold for background snapshot writes inside `apply`.
    /// `0` disables the in-band trigger (the CLI sets this — it's
    /// ephemeral and shouldn't churn the snapshots dir).
    snapshot_threshold: u32,
    /// Whether `self.log` carries every op ever persisted (`true` after
    /// full replay) or only the delta posted after a snapshot cutoff
    /// (`false` after snapshot boot). When `false`, `Doc` rebuilds that
    /// need the full `Edit` history for a node load it from storage via
    /// [`Self::ensure_doc_for_edit`] — the in-memory log alone would
    /// miss pre-snapshot edits and produce a wrong Doc state (#129).
    log_complete: bool,
}

impl Workspace {
    /// Open an in-memory workspace. Useful for tests.
    pub fn open_in_memory(actor: ActorId) -> Result<Self, WorkspaceError> {
        let storage = crate::storage::MemoryStorage::new();
        Self::open_with_storage(actor, Box::new(storage), None)
    }

    /// Open a workspace backed by a given storage implementation.
    ///
    /// Tries to boot from a snapshot first (O(current state) + the ops
    /// posted since the snapshot); on any failure — missing snapshot,
    /// hash mismatch, future schema — falls back to a full replay of
    /// the op log. Snapshot is purely a boot cache; the op log stays
    /// the single source of truth.
    ///
    /// In both paths, block text is materialized so the workspace is
    /// ready to read from the moment this returns.
    pub fn open_with_storage(
        actor: ActorId,
        storage: Box<dyn Storage>,
        root: Option<PathBuf>,
    ) -> Result<Self, WorkspaceError> {
        let snapshots_dir = root.as_ref().map(|r| r.join(".outl").join("snapshots"));

        let mut ws = Self {
            root,
            actor,
            tree: Tree::new(),
            log: OpLog::new(),
            content: ContentStore::default(),
            storage,
            page_storages: HashMap::new(),
            page_root_to_slug: HashMap::new(),
            snapshots_dir,
            snapshot_workers: Vec::new(),
            ops_since_snapshot: 0,
            snapshot_threshold: Workspace::DEFAULT_SNAPSHOT_THRESHOLD,
            log_complete: true,
        };

        let booted_from_snapshot = match ws.boot_from_snapshot() {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                warn!("snapshot boot failed, falling back to full replay: {e}");
                false
            }
        };

        if booted_from_snapshot {
            // The in-memory log only has delta ops; `Doc` rebuilds that
            // need the full `Edit` history load it from storage (#129).
            ws.log_complete = false;
        } else {
            ws.boot_from_full_replay()?;
        }

        Ok(ws)
    }

    /// Default `apply`-count between in-band snapshot writes. Clients
    /// override with [`Self::set_snapshot_policy`] from `[snapshot]`
    /// in `outl.toml`. The CLI forces `0` to opt out.
    const DEFAULT_SNAPSHOT_THRESHOLD: u32 = 10_000;

    /// Try to hydrate the workspace from a snapshot + the ops posted
    /// since its cutoff. Returns `Ok(false)` when there's nothing to
    /// load (so the caller falls through to [`Self::boot_from_full_replay`]).
    fn boot_from_snapshot(&mut self) -> Result<bool, WorkspaceError> {
        let snap = match self.storage.load_snapshot()? {
            Some(s) => s,
            None => return Ok(false),
        };
        let body = match SnapshotBody::decode(&snap.bytes) {
            Ok(b) => b,
            Err(e) => {
                warn!("snapshot unusable, falling back to full replay: {e}");
                return Ok(false);
            }
        };

        // Hydrate the materialized tree + block text directly from the
        // snapshot body. No `Edit` ops are replayed for these nodes;
        // `block_text` already carries their settled string. Convert
        // BTreeMap → HashMap to match the runtime representation
        // (ContentStore's hot path is a HashMap lookup).
        self.tree = Tree::from_parts(
            body.nodes.into_iter().collect(),
            body.properties.into_iter().collect(),
            body.collapsed.into_iter().collect(),
        );
        self.content = ContentStore::from_text_map(body.block_text.into_iter().collect());

        // Replay only the ops the snapshot hasn't seen yet. `Edit` ops
        // in this delta will be re-materialized in the loop below; the
        // rest of the snapshot's text is still authoritative.
        let delta = self.ops_since_combined(body.hlc_cutoff)?;
        let mut edited_in_delta: HashSet<NodeId> = HashSet::new();
        for op in delta {
            if let Op::Edit { node, .. } = &op.op {
                edited_in_delta.insert(*node);
            }
            self.tree.apply_op(&mut self.log, op);
        }

        // Re-materialize only the nodes whose text changed after the
        // snapshot. The in-memory log only has delta ops (pre-snapshot
        // ops are in storage), so we load the FULL `Edit` history for
        // each edited node from storage. Replaying everything through a
        // fresh `Doc` — pre-snapshot edits included — is what makes the
        // resulting text match the full-replay path (#129).
        let rematerialized_count = edited_in_delta.len();
        for node in &edited_in_delta {
            let mut ops = match self.ops_for_node_combined(*node) {
                Ok(o) => o,
                Err(e) => {
                    warn!("skipping re-materialize for {node}: {e}");
                    continue;
                }
            };
            // `ops_for_node`'s order is backend-dependent (JsonlStorage's
            // in-memory cache can be unsorted after appends). Sort by HLC so
            // the Doc is rebuilt in the same order as the full-replay path.
            ops.sort_by_key(|l| l.ts);
            let updates: Vec<&[u8]> = ops
                .iter()
                .filter_map(|l| match &l.op {
                    Op::Edit {
                        node: n, text_op, ..
                    } if n == node => Some(text_op.as_slice()),
                    _ => None,
                })
                .collect();
            if !updates.is_empty() {
                self.content.materialize(*node, updates.into_iter());
            }
        }

        debug!(
            "booted from snapshot (cutoff {}, {} nodes, {} delta ops, {rematerialized_count} \
             nodes re-materialized)",
            body.hlc_cutoff.physical_ms,
            self.tree.node_count(),
            self.log.len(),
        );

        Ok(true)
    }

    /// Boot by replaying every op in storage. The historical path; used
    /// as the snapshot-miss fallback and by `open_in_memory`.
    fn boot_from_full_replay(&mut self) -> Result<(), WorkspaceError> {
        // Pass 1: structural. Apply every op to the tree (`Edit` is a
        // no-op there) and the log. Text is materialized in pass 2 so the
        // open-time memory peak stays at a single live `Doc` instead of
        // one per block — that peak is what jetsam was killing on iOS.
        let ops = self.all_ops_combined()?;
        for op in ops {
            self.tree.apply_op(&mut self.log, op);
        }

        // Pass 2: text. Group `Edit` ops by node (indices only, no byte
        // copies), then rebuild one `Doc` at a time, materialize its
        // string, and drop it before moving on.
        let mut edits_by_node: HashMap<NodeId, Vec<usize>> = HashMap::new();
        for (i, logged) in self.log.iter().enumerate() {
            if let Op::Edit { node, .. } = &logged.op {
                edits_by_node.entry(*node).or_default().push(i);
            }
        }
        for (node, indices) in edits_by_node {
            self.content.materialize(
                node,
                indices.iter().filter_map(|&i| match self.log.get(i) {
                    Some(LogOp {
                        op: Op::Edit { text_op, .. },
                        ..
                    }) => Some(text_op.as_slice()),
                    _ => None,
                }),
            );
        }

        // Seed the snapshot counter so a long-lived workspace opened
        // without a snapshot on disk doesn't have to wait a full
        // `threshold` worth of new ops before producing one. If the log
        // already crosses the threshold, the next `apply` snapshots.
        self.ops_since_snapshot = (self.log.len() as u32).min(self.snapshot_threshold);

        Ok(())
    }

    /// Ensure the Yrs `Doc` for `node` is in the content cache with the
    /// FULL `Edit` history applied.
    ///
    /// After snapshot boot, `self.log` only carries delta ops (the
    /// pre-snapshot ops live in storage). Rebuilding a `Doc` from the
    /// incomplete log would miss those edits, producing a wrong state
    /// vector and, in turn, updates that concatenate instead of replace
    /// on full replay (#129). When `log_complete` is `false`, we load
    /// the node's entire `Edit` history from storage before the
    /// `ContentStore`'s internal `ensure_doc` kicks in.
    fn ensure_doc_for_edit(&mut self, node: NodeId) {
        if self.content.is_cached(node) {
            return;
        }
        if self.log_complete {
            return; // ContentStore::ensure_doc rebuilds from the full log.
        }
        let mut ops = match self.ops_for_node_combined(node) {
            Ok(o) => o,
            Err(e) => {
                warn!("could not load ops for {node} from storage: {e}");
                return;
            }
        };
        // Sort by HLC before replay: `ops_for_node`'s return order is
        // backend-dependent, and rebuilding the Doc in the same order as the
        // full-replay path keeps the state vector deterministic across
        // backends and sessions.
        ops.sort_by_key(|l| l.ts);
        let updates: Vec<&[u8]> = ops
            .iter()
            .filter_map(|l| match &l.op {
                Op::Edit {
                    node: n, text_op, ..
                } if *n == node => Some(text_op.as_slice()),
                _ => None,
            })
            .collect();
        self.content.cache_doc(node, updates.into_iter());
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
            self.ensure_doc_for_edit(*node);
            self.content.merge_update(*node, &self.log, text_op);
        }
        self.tree.apply_op(&mut self.log, op.clone());

        // Route to the right storage. If the op's node belongs to a
        // registered page, write to that page's shard; otherwise write
        // to the global storage (legacy behaviour).
        let slug = op_node(&op.op).and_then(|node| self.slug_for_node(node));
        match slug {
            Some(ref slug) if self.page_storages.contains_key(slug) => {
                if let Some(s) = self.page_storages.get_mut(slug) {
                    s.append_op(&op)?;
                }
            }
            _ => {
                self.storage.append_op(&op)?;
            }
        }

        // Background snapshot trigger. `snapshot_threshold = 0` is the
        // opt-out (CLI). When the threshold is crossed we build the
        // `SnapshotBody` inline (cheap-ish: 3 HashMap clones + text map
        // clone + one sha256; the *write* is what's expensive) and hand
        // it off to a worker thread so the user never blocks on fsync.
        // Failure inside the worker is logged and discarded — snapshot
        // is a cache, not source of truth.
        if self.snapshot_threshold > 0 && self.snapshots_dir.is_some() {
            self.ops_since_snapshot = self.ops_since_snapshot.saturating_add(1);
            if self.ops_since_snapshot >= self.snapshot_threshold {
                // Drain finished workers (non-blocking) so the handle
                // list doesn't grow unbounded over a long session.
                self.snapshot_workers.retain(|h| !h.is_finished());
                self.spawn_background_snapshot();
                self.ops_since_snapshot = 0;
            }
        }
        Ok(())
    }

    /// Build a `SnapshotBody` from the current materialized state and
    /// hand it to a worker thread that runs [`snapshot::write_to_disk`].
    /// Non-blocking: the encode + fsync + rename happen off the calling
    /// thread. If the body can't be built (e.g. empty log) we no-op.
    fn spawn_background_snapshot(&mut self) {
        let cutoff = match self.log.last() {
            Some(l) => l.ts,
            None => return,
        };
        let snapshots_dir = match &self.snapshots_dir {
            Some(p) => p.clone(),
            None => return,
        };
        let (nodes, properties, collapsed) = self.tree.snapshot_parts();
        // Convert to BTreeMap/BTreeSet for canonical (order-stable)
        // serialization — see `snapshot::SnapshotBody` for why.
        let body = SnapshotBody::from_parts(
            self.actor,
            cutoff,
            nodes.iter().map(|(k, v)| (*k, v.clone())).collect(),
            properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            collapsed.iter().copied().collect(),
            self.content
                .text_map()
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
        );

        let handle = std::thread::Builder::new()
            .name(format!("outl-snapshot-{}", self.actor))
            .spawn(move || {
                if let Err(e) = snapshot::write_to_disk(&snapshots_dir, &body) {
                    warn!("background snapshot write failed (non-fatal): {e}");
                }
            })
            .expect("spawn snapshot worker");
        self.snapshot_workers.push(handle);
    }

    /// Block until every background snapshot worker finishes.
    ///
    /// Call on graceful shutdown so a long-lived client doesn't exit
    /// with a snapshot write still in flight (which would race the
    /// process exit and could leave a stale `.tmp` behind). Errors
    /// inside the workers are already logged; this just joins.
    pub fn wait_for_snapshots(&mut self) {
        let workers = std::mem::take(&mut self.snapshot_workers);
        for h in workers {
            if let Err(e) = h.join() {
                warn!("snapshot worker panicked: {e:?}");
            }
        }
    }

    /// Configure when the workspace writes snapshots to disk during
    /// `apply`. Pass `enabled = false` to opt out (the CLI does this
    /// — it's ephemeral and shouldn't churn the snapshots dir). The
    /// threshold is the number of `apply` calls between snapshot
    /// writes; values smaller than 1 are clamped to 1 so we don't
    /// snapshot on every single op.
    ///
    /// Reads `[snapshot]` from `outl.toml` via `outl-config` — clients
    /// wire it up after loading config:
    ///
    /// ```ignore
    /// ws.set_snapshot_policy(cfg.snapshot.enabled, cfg.snapshot.op_threshold);
    /// ```
    pub fn set_snapshot_policy(&mut self, enabled: bool, threshold: u32) {
        self.snapshot_threshold = if enabled { threshold.max(1) } else { 0 };
        self.ops_since_snapshot = 0;
    }

    /// Persist a snapshot of the materialized state under the current
    /// actor, stamped with the latest applied HLC as its cutoff.
    ///
    /// Synchronous — used by clients on graceful shutdown (when they're
    /// about to drop the workspace anyway and don't care that the write
    /// blocks). For the in-band path, [`Self::apply`] spawns a worker
    /// thread that calls the same writer without blocking the caller.
    ///
    /// A no-op (returns `Ok(())`) when the log is empty — there's
    /// nothing worth snapshotting and a zero-cutoff snapshot would
    /// force the next boot to re-replay everything anyway.
    pub fn save_snapshot(&mut self) -> Result<(), WorkspaceError> {
        let cutoff = match self.log.last() {
            Some(l) => l.ts,
            None => return Ok(()),
        };
        let (nodes, properties, collapsed) = self.tree.snapshot_parts();
        let body = SnapshotBody::from_parts(
            self.actor,
            cutoff,
            nodes.iter().map(|(k, v)| (*k, v.clone())).collect(),
            properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            collapsed.iter().copied().collect(),
            self.content
                .text_map()
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
        );
        let bytes = body.encode().map_err(|e| {
            WorkspaceError::Storage(StorageError::Backend(format!("snapshot encode: {e}")))
        })?;
        self.storage
            .save_snapshot(&crate::storage::Snapshot { bytes })?;
        debug!(
            "snapshot saved at cutoff {} ({} nodes, {} properties)",
            cutoff.physical_ms,
            nodes.len(),
            properties.len()
        );
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
        self.ensure_doc_for_edit(node);
        self.content.replace_text(node, &self.log, new_text)
    }

    /// Re-boot the workspace from all registered storages (Global +
    /// PerPage). Called by clients after registering per-page shards
    /// via [`Self::register_page_storage`] — the initial boot only
    /// sees the Global storage, so a re-boot is needed to load the
    /// per-page ops into the materialized tree. RFC #137 Phase B.
    pub fn reboot_with_all_storages(&mut self) -> Result<(), WorkspaceError> {
        self.tree = Tree::new();
        self.log = OpLog::new();
        self.content = ContentStore::default();
        self.log_complete = true;
        self.boot_from_full_replay()?;
        Ok(())
    }

    /// Whether any per-page storage shards have been registered.
    pub fn has_page_storages(&self) -> bool {
        !self.page_storages.is_empty()
    }

    /// Register a per-page storage backend. The client (CLI / TUI /
    /// desktop / mobile) calls this after opening the workspace if the
    /// workspace uses the per-page shard layout (RFC #137 Phase B).
    /// Ops whose node belongs to `slug` will be routed to this storage
    /// instead of the global one.
    pub fn register_page_storage(&mut self, slug: &str, storage: Box<dyn Storage>) {
        self.page_storages.insert(slug.to_string(), storage);
    }

    /// Register a page root → slug mapping. The client calls this for
    /// every page/journal it knows about (read from sidecars via
    /// `outl-md`). `apply` uses this to resolve which page an op
    /// belongs to by walking the parent chain up to a page root.
    pub fn register_page_root(&mut self, root_id: NodeId, slug: &str) {
        self.page_root_to_slug.insert(root_id, slug.to_string());
    }

    /// Walk `tree.parent(node)` up until we hit a registered page root.
    /// Returns the slug of that page, or `None` if the chain dead-ends
    /// at the workspace root without finding one.
    fn slug_for_node(&self, node: NodeId) -> Option<String> {
        let mut current = node;
        loop {
            if let Some(slug) = self.page_root_to_slug.get(&current) {
                return Some(slug.clone());
            }
            current = self.tree.parent(current)?;
        }
    }

    /// Merge ops from the global storage and every registered
    /// per-page storage, sorted by HLC. Used by boot and read-side
    /// accessors (`all_ops`, `ops_since`, `ops_for_node`, etc.).
    fn all_ops_combined(&self) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.all_ops()?;
        for s in self.page_storages.values() {
            all.extend(s.all_ops()?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge ops with HLC > `ts` from every storage.
    fn ops_since_combined(&self, ts: crate::hlc::Hlc) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.ops_since(ts)?;
        for s in self.page_storages.values() {
            all.extend(s.ops_since(ts)?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge ops for `node` from every storage.
    fn ops_for_node_combined(&self, node: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let mut all = self.storage.ops_for_node(node)?;
        for s in self.page_storages.values() {
            all.extend(s.ops_for_node(node)?);
        }
        all.sort_by_key(|op| op.ts);
        all.dedup_by_key(|op| op.ts);
        Ok(all)
    }

    /// Merge `last_ts_per_actor` from every storage.
    #[allow(dead_code)]
    fn last_ts_per_actor_combined(
        &self,
    ) -> Result<HashMap<ActorId, crate::hlc::Hlc>, StorageError> {
        let mut map = self.storage.last_ts_per_actor()?;
        for s in self.page_storages.values() {
            for (actor, ts) in s.last_ts_per_actor()? {
                map.entry(actor)
                    .and_modify(|existing| {
                        if ts > *existing {
                            *existing = ts;
                        }
                    })
                    .or_insert(ts);
            }
        }
        Ok(map)
    }

    /// Apply the LRU cap configured by the client, after boot has
    /// finished re-materialising Yrs `Doc`s.
    ///
    /// Boot needs every op in RAM so `ops_for_node` (used to rebuild
    /// `Doc`s for nodes edited after the snapshot cutoff) sees the full
    /// `Edit` history. Once boot is done, the long-running client can
    /// shed cold history: `cap = 50_000` keeps the most-recent ops
    /// resident; older ones come back from disk via the offset index.
    /// RFC #137 Phase A.
    ///
    /// `cap = 0` means "unbounded" — keep every op resident (legacy
    /// default). Idempotent and safe to call at any point in the
    /// lifecycle (clients may resize on config change).
    pub fn apply_lru_cap(&mut self, cap: usize) {
        self.storage.resize_cache(cap);
        for s in self.page_storages.values_mut() {
            s.resize_cache(cap);
        }
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
