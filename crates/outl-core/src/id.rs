//! Stable identifiers for nodes and actors.
//!
//! Both are ULIDs (128-bit, lexicographically sortable). `NodeId` identifies
//! a block or page; `ActorId` identifies a device that produces ops.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identifier for a node in the outline tree.
///
/// Nodes are blocks and pages (a page is the root of its block tree).
/// Identifiers are ULIDs and must be globally unique across all replicas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub ulid::Ulid);

impl NodeId {
    /// Generate a fresh `NodeId` using the current time and a random tail.
    pub fn new() -> Self {
        Self(ulid::Ulid::new())
    }

    /// Returns the canonical [`NodeId`] for the workspace root.
    ///
    /// All real nodes descend (transitively) from this root.
    pub const fn root() -> Self {
        Self(ulid::Ulid(0))
    }

    /// Returns the canonical [`NodeId`] used to represent "deleted" nodes.
    ///
    /// We never physically remove a node from the op log; instead, deletion
    /// is implemented as a `Move` to `TRASH_ROOT`. See `docs/crdt.md`.
    pub const fn trash() -> Self {
        // Distinct sentinel value from ROOT (1u128).
        Self(ulid::Ulid(1))
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifier of a device (or process) producing ops.
///
/// ActorIds are ULIDs persisted per-workspace in `.outl/config.toml`.
/// They serve as the final tiebreak in HLC ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ActorId(pub ulid::Ulid);

impl ActorId {
    /// Generate a fresh `ActorId`.
    pub fn new() -> Self {
        Self(ulid::Ulid::new())
    }
}

impl Default for ActorId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
