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

pub mod jsonl;
pub mod sqlite;

pub use jsonl::JsonlStorage;
pub use sqlite::SqliteStorage;

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

/// A snapshot of materialized state.
///
/// Step 2 defines the concrete shape; Step 1 ships a typed alias.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    /// Serialized bytes; format owned by `Storage` implementations.
    pub bytes: Vec<u8>,
}

/// Storage backend trait.
///
/// Implementations: `SqliteStorage` (phase 1), `ChronDbStorage` (issue #1),
/// in-memory test double (test-only).
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
}
