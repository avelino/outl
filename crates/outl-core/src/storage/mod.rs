//! Storage abstraction.
//!
//! The op log is the source of truth; storage is how we persist it. The
//! `Storage` trait is the single boundary between `outl-core` and any
//! particular persistence layer. See `docs/storage.md`.

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use std::collections::HashMap;
use thiserror::Error;

pub mod index;
pub mod jsonl;
pub mod memory;
pub mod node_index;

pub use index::{ActorIndex, OffsetIndex};
pub use jsonl::JsonlStorage;
pub use memory::MemoryStorage;
pub use node_index::{ActorNodeIndex, NodeIndex};

/// Which page a storage backend is responsible for.
///
/// `Global` is the legacy single-file-per-actor layout every workspace
/// shipped with before RFC #137 Phase B. `PerPage(slug)` is the
/// sharded layout — one `ops/<actor>/<slug>.jsonl` per (actor, page)
/// pair — that lets boot and sync be proportional to the active page
/// rather than the whole workspace.
///
/// Layouts on disk:
///
/// - `Global` → `ops/ops-<actor>.jsonl` (+ `.idx`, `.nodes.idx` sidecars)
/// - `PerPage(slug)` → `ops/<actor>/<slug>.jsonl` (+ sidecars)
///
/// `Global` stays the default for back-compat. New workspaces opt into
/// `PerPage` via `outl init --scope=per-page`; existing ones migrate
/// via `outl migrate-to-per-page-ops`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PageScope {
    /// Single op log per actor — every page shares one file.
    #[default]
    Global,
    /// Op log scoped to one page. The slug is the page's URL-safe name
    /// (same one used for the `.md` filename under `pages/`).
    PerPage(String),
}

impl PageScope {
    /// `true` for the legacy single-file layout.
    pub fn is_global(&self) -> bool {
        matches!(self, PageScope::Global)
    }
}

/// Errors a `Storage` implementation may produce.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Underlying I/O or backend error.
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Serialization failure for op or snapshot data.
    #[error("serialization error: {0}")]
    Serialize(String),

    /// Integrity check failed (e.g. SQLite `integrity_check`).
    #[error("integrity error: {0}")]
    Integrity(String),

    /// Op log corruption: an op that should be present is missing.
    #[error("missing op: {0}")]
    MissingOp(String),
}

/// An opaque on-disk snapshot of materialized workspace state.
///
/// Storage treats `bytes` as a black box — it does not know (or need to
/// know) the layout. `Workspace` is the single owner of the format: it
/// serializes the materialized tree + block text via bincode and hands
/// the buffer to `Storage` for persistence. See `snapshot.rs` for the
/// typed shape (`SnapshotBody`) and the boot contract.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    /// Serialized snapshot body; format owned by the caller of
    /// `save_snapshot`, not by `Storage`.
    pub bytes: Vec<u8>,
}

/// Storage backend trait.
///
/// Implementations: [`JsonlStorage`] (one file per actor, the only
/// persistent backend), [`MemoryStorage`] (test double, no disk),
/// future `ChronDbStorage` (issue #1).
pub trait Storage: Send + Sync {
    /// Append a new op. Must be durable before returning `Ok`.
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError>;

    /// Return all ops with HLC strictly greater than `ts`, in HLC order.
    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops touching the given node.
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops created by the given actor.
    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError>;

    /// Return the most recent HLC seen per actor (used for sync vector clocks).
    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError>;

    /// Return all ops in HLC order.
    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError>;

    /// Persist a snapshot for faster startup.
    fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError>;

    /// Load the most recent snapshot, if any.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError>;

    /// HLC cutoff of the snapshot currently on disk, if any.
    ///
    /// Used by `Workspace` to decide whether the snapshot is worth
    /// loading on boot and how many ops to replay after it
    /// (`ops_since(cutoff)`). Default `None` covers in-memory backends
    /// and any future backend that has no snapshot yet.
    fn snapshot_cutoff(&self) -> Result<Option<Hlc>, StorageError> {
        Ok(None)
    }

    /// Shrink (or grow) the in-memory op cache to hold at most `cap`
    /// ops. `cap = 0` means "unbounded" — keep every op resident (the
    /// legacy default). Default no-op; [`JsonlStorage`] implements it
    /// for real.
    ///
    /// Called by `Workspace` after boot completes (see
    /// `Workspace::apply_lru_cap`). Boot needs every op in RAM to
    /// re-materialise Yrs `Doc`s via `ops_for_node`; once that's done,
    /// the long-running client can shed the cold history.
    ///
    /// Implementations must be idempotent and safe to call from any
    /// point in the lifecycle.
    fn resize_cache(&mut self, _cap: usize) {}
}
