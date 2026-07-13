//! Materialized-state snapshot.
//!
//! A snapshot is a projection of the workspace tree + block text at a
//! specific HLC cutoff. It is **not** source of truth — the op log is.
//! Its only job is to short-circuit the O(total history) replay on boot
//! (issue #109) by giving `Workspace::open_with_storage` a starting
//! point that costs O(current state) to load and O(delta) to bring
//! up-to-date via [`crate::storage::Storage::ops_since`].
//!
//! ## Layout
//!
//! [`SnapshotBody`] is bincode-serialized and written straight to
//! `<root>/.outl/snapshots/snap-<actor>.bin` by [`write_to_disk`], and
//! read back by [`read_from_disk`]. `Workspace` owns both the format and
//! the on-disk location — the snapshot is a local boot cache, never
//! routed through the storage backend (the op log). A single
//! `schema_version` lets us migrate later without guessing.
//!
//! ## Integrity
//!
//! `content_hash` is `sha256(body)` computed with the hash field zeroed.
//! `decode` recomputes and compares; a mismatch falls back to full
//! replay (see `Workspace::open_with_storage`). Snapshot is a cache —
//! never a source of truth — so a stale snapshot is silently ignored.
//!
//! ## Scope (Phase 1)
//!
//! This module handles **local boot** only. Sharing snapshots between
//! peers (Phase 2, via iroh) and compacting the op log (Phase 3, with
//! undo horizon) live elsewhere.
//!
//! [`Workspace`]: crate::workspace::Workspace

use crate::fractional::Fractional;
use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::property::PropValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Current snapshot wire format. Bumped on any breaking change to
/// [`SnapshotBody`]; `decode` rejects mismatched versions instead of
/// guessing at backward compatibility.
pub const SCHEMA_VERSION: u32 = 2;

/// Errors that can occur while encoding or decoding a snapshot.
///
/// None of these are fatal for the caller — the boot path treats every
/// variant as "snapshot unusable, fall back to full replay" — but they
/// are surfaced so the caller can log a targeted warning instead of
/// silently eating the I/O cost.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Snapshot was written by a future schema version we don't know.
    #[error("snapshot schema version mismatch: expected <= {max_supported}, got {found}")]
    SchemaMismatch {
        /// Highest schema version this binary understands.
        max_supported: u32,
        /// Schema version found in the snapshot buffer.
        found: u32,
    },
    /// `content_hash` didn't match the body — file is corrupt or was
    /// partially rewritten (e.g. `kill -9` mid-save).
    #[error("snapshot content hash mismatch — corrupt or stale")]
    HashMismatch,
    /// Failed to serialize the snapshot body via bincode.
    #[error("snapshot encode error: {0}")]
    Encode(String),
    /// Failed to deserialize the snapshot body via bincode.
    #[error("snapshot decode error: {0}")]
    Decode(String),
    /// Filesystem error while writing or reading the snapshot file.
    /// Surfaced separately from encode/decode so a caller can
    /// distinguish "format broken" from "disk full".
    #[error("snapshot I/O error: {0}")]
    Io(String),
}

/// Typed view over a snapshot's `bytes`.
///
/// Built from a `Workspace` via [`SnapshotBody::from_parts`], serialized
/// with bincode, and persisted by `Storage::save_snapshot`. On boot,
/// `Workspace` calls [`SnapshotBody::decode`] on the bytes returned by
/// `Storage::load_snapshot`; a [`SnapshotError`] triggers the full
/// replay fallback.
///
/// All maps are `BTreeMap`s (not `HashMap`s) on purpose: the
/// `content_hash` is computed over the bincode-serialized body, and
/// `BTreeMap`'s iteration order is determined by key order — not by
/// per-instance hash-table state — so two bodies with the same content
/// produce the same hash. (Rust's `HashMap` randomizes layout per
/// process and even two `HashMap`s with identical contents can iterate
/// in different orders after different insertion histories.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotBody {
    /// Bumped on any breaking change to this struct.
    pub schema_version: u32,
    /// Actor that produced this snapshot. Informational; any actor may
    /// load any actor's snapshot (the materialized tree is the union of
    /// every actor's ops up to the per-actor `cutoff`).
    pub actor: ActorId,
    /// Per-actor replay cutoff: the high-water-mark HLC of each actor
    /// whose ops the materialized state below already includes.
    ///
    /// On boot `Workspace` replays, for each actor `A`, only the ops with
    /// `hlc > cutoff[A]` — and **every** op of an actor absent from this
    /// map (that actor was entirely unseen when the snapshot was taken).
    ///
    /// This must be a per-actor vector clock, not a single global HLC: a
    /// single cutoff tracks only the high-water mark of the snapshotting
    /// actor, so a legitimately-low-HLC op from a *different* actor
    /// delivered after the snapshot (offline device, lagging clock) would
    /// fall below it and be silently dropped from the tree even though
    /// it's durably in storage (#156).
    pub cutoff: BTreeMap<ActorId, Hlc>,
    /// Tree nodes: `(node_id -> (parent, position))`. `ROOT` and
    /// `TRASH_ROOT` are implicit, never present as keys.
    pub nodes: BTreeMap<NodeId, (NodeId, Fractional)>,
    /// Property triples: `(node_id, key) -> value`.
    pub properties: BTreeMap<(NodeId, String), PropValue>,
    /// Nodes currently flagged collapsed. Absence = expanded (default).
    pub collapsed: BTreeSet<NodeId>,
    /// Materialized text of every block that has text. Blocks without
    /// text are simply absent; `ContentStore` treats missing keys as
    /// empty.
    pub block_text: BTreeMap<NodeId, String>,
    /// `sha256` over the body with this field zeroed. Computed in
    /// [`SnapshotBody::from_parts`] and verified in [`SnapshotBody::decode`].
    pub content_hash: [u8; 32],
}

impl SnapshotBody {
    /// Assemble a snapshot from the materialized pieces of a workspace.
    ///
    /// The caller (always `Workspace` today) is responsible for picking
    /// the per-actor `cutoff` — the high-water-mark HLC of each actor the
    /// materialized state already reflects. We compute the `content_hash`
    /// here so what's returned is ready to [`encode`](Self::encode) and
    /// persist.
    pub fn from_parts(
        actor: ActorId,
        cutoff: BTreeMap<ActorId, Hlc>,
        nodes: BTreeMap<NodeId, (NodeId, Fractional)>,
        properties: BTreeMap<(NodeId, String), PropValue>,
        collapsed: BTreeSet<NodeId>,
        block_text: BTreeMap<NodeId, String>,
    ) -> Self {
        let mut body = Self {
            schema_version: SCHEMA_VERSION,
            actor,
            cutoff,
            nodes,
            properties,
            collapsed,
            block_text,
            content_hash: [0u8; 32],
        };
        body.content_hash = compute_hash(&body);
        body
    }

    /// Serialize via bincode for persistence by [`write_to_disk`].
    pub fn encode(&self) -> Result<Vec<u8>, SnapshotError> {
        bincode::serialize(self).map_err(|e| SnapshotError::Encode(e.to_string()))
    }

    /// Deserialize and validate a snapshot buffer.
    ///
    /// Returns [`SnapshotError::HashMismatch`] on tampering or
    /// truncation, and [`SnapshotError::SchemaMismatch`] if the buffer
    /// came from a future version. Both are recoverable: the caller
    /// falls back to a full op-log replay.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let body: Self =
            bincode::deserialize(bytes).map_err(|e| SnapshotError::Decode(e.to_string()))?;
        if body.schema_version > SCHEMA_VERSION {
            return Err(SnapshotError::SchemaMismatch {
                max_supported: SCHEMA_VERSION,
                found: body.schema_version,
            });
        }
        let recomputed = compute_hash(&body);
        if recomputed != body.content_hash {
            return Err(SnapshotError::HashMismatch);
        }
        Ok(body)
    }
}

/// Hash the body with the `content_hash` field zeroed. Used both at
/// build time (to stamp the hash) and at load time (to verify it).
fn compute_hash(body: &SnapshotBody) -> [u8; 32] {
    let mut clone = body.clone();
    clone.content_hash = [0u8; 32];
    // Two bodies with identical content must hash identical regardless
    // of HashMap iteration order — serialize the canonical-form clone.
    // bincode's default encoding is already order-deterministic given a
    // fixed in-memory layout, so this is sufficient for the integrity
    // check. Cross-actor canonical comparison is not a goal of the hash.
    let bytes = bincode::serialize(&clone).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(out.as_slice());
    arr
}

/// Write `body` to `snapshots_dir/snap-<actor>.bin` atomically.
///
/// Encodes the body to bincode, writes to a sibling `.tmp`, `fsync`s,
/// and renames into place. A crash at any point leaves either nothing
/// (`.tmp` never created) or a stale `.tmp` (rename didn't happen) —
/// never a half-written `snap-*.bin` that `load` could mistake for a
/// valid snapshot.
///
/// This is a standalone function (not on `Storage`) on purpose: the
/// background-snapshot path in `Workspace::apply` calls it from a
/// worker thread that owns the body outright, with no borrow of the
/// storage backend. `Workspace::save_snapshot` delegates here for its
/// synchronous shutdown path, and `Workspace::spawn_background_snapshot`
/// for the in-band worker-thread path.
pub fn write_to_disk(snapshots_dir: &Path, body: &SnapshotBody) -> Result<(), SnapshotError> {
    let actor = body.actor;
    let bytes = body.encode()?;
    let final_path = snapshots_dir.join(format!("snap-{actor}.bin"));
    let tmp_path = final_path.with_extension("bin.tmp");

    std::fs::create_dir_all(snapshots_dir)
        .map_err(|e| SnapshotError::Io(format!("create {}: {e}", snapshots_dir.display())))?;
    let mut file = File::create(&tmp_path)
        .map_err(|e| SnapshotError::Io(format!("create {}: {e}", tmp_path.display())))?;
    file.write_all(&bytes)
        .map_err(|e| SnapshotError::Io(format!("write {}: {e}", tmp_path.display())))?;
    file.sync_all()
        .map_err(|e| SnapshotError::Io(format!("fsync {}: {e}", tmp_path.display())))?;
    drop(file);
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        SnapshotError::Io(format!(
            "rename {} -> {}: {e}",
            tmp_path.display(),
            final_path.display()
        ))
    })?;
    tracing::debug!(
        "snapshot written to {} ({} bytes)",
        final_path.display(),
        bytes.len()
    );
    Ok(())
}

/// Read and decode `snapshots_dir/snap-<actor>.bin`, if present.
///
/// Returns `Ok(None)` when no snapshot exists yet — first boot, or the
/// file was never written. A decode / hash / schema failure is surfaced
/// as `Err` so the caller can log a targeted warning; the boot path
/// treats *every* outcome other than `Ok(Some(_))` as "snapshot
/// unusable, fall back to full replay". Only the exact `snap-<actor>.bin`
/// is read, so a leftover `.tmp` from a crashed [`write_to_disk`] is
/// never mistaken for a valid snapshot.
///
/// Standalone (not on `Storage`) for the same reason as [`write_to_disk`]:
/// the snapshot is a local boot cache owned by `Workspace`, not part of
/// the source-of-truth op log, so it never routes through the storage
/// backend.
pub fn read_from_disk(
    snapshots_dir: &Path,
    actor: ActorId,
) -> Result<Option<SnapshotBody>, SnapshotError> {
    let path = snapshots_dir.join(format!("snap-{actor}.bin"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SnapshotError::Io(format!("read {}: {e}", path.display()))),
    };
    SnapshotBody::decode(&bytes).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_body() -> SnapshotBody {
        SnapshotBody::from_parts(
            ActorId::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeMap::new(),
        )
    }

    #[test]
    fn roundtrips_empty_body() {
        let body = empty_body();
        let bytes = body.encode().expect("encode");
        let decoded = SnapshotBody::decode(&bytes).expect("decode");
        assert_eq!(decoded.schema_version, SCHEMA_VERSION);
        assert_eq!(decoded.nodes, body.nodes);
        assert_eq!(decoded.content_hash, body.content_hash);
    }

    #[test]
    fn detects_tampered_hash() {
        let mut bytes = empty_body().encode().expect("encode");
        // Flip one byte near the end (hash field is last in serialization
        // order). Mismatch between stored and recomputed hash must trip
        // HashMismatch, not silently accept.
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = SnapshotBody::decode(&bytes).unwrap_err();
        assert!(matches!(err, SnapshotError::HashMismatch), "got {err:?}");
    }

    #[test]
    fn rejects_future_schema_version() {
        let mut body = empty_body();
        body.schema_version = SCHEMA_VERSION + 1;
        // Re-stamp the hash so the only thing wrong is the schema.
        body.content_hash = compute_hash(&body);
        let bytes = body.encode().expect("encode");
        let err = SnapshotBody::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, SnapshotError::SchemaMismatch { found, .. } if found == SCHEMA_VERSION + 1),
            "got {err:?}"
        );
    }
}
