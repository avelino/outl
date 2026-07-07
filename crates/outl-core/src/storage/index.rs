//! Per-actor offset index sidecar (`ops-<actor>.idx`).
//!
//! Maps each op's HLC to its byte offset inside the matching
//! `ops-<actor>.jsonl`. The index is what lets `JsonlStorage` read a
//! single cold op from a mmapped file without buffering the whole log
//! into RAM (RFC #137, Phase A — LRU + mmap).
//!
//! ## On-disk format
//!
//! JSONL, one entry per line, mirroring the `.jsonl` it indexes:
//!
//! ```jsonc
//! {"ts": {…Hlc…}, "offset": 1234}
//! ```
//!
//! JSONL (not bincode) on purpose:
//!
//! - Same recovery path as the op log itself — `StreamDeserializer`
//!   catches glued writes.
//! - Append-only, so `append_op`'s index update is one `writeln!` +
//!   `fsync`, same cost shape as the op log append.
//! - Greppable and diffable, which matters the first time a real
//!   workspace hits an index bug.
//!
//! The index is a **cache**, not source of truth. Any time the `.idx`
//! is missing, truncated, or disagrees with the `.jsonl` line count,
//! we rebuild it from scratch by streaming the `.jsonl` once. That
//! rebuild is the same cost as the legacy full-load, so a missing
//! index never makes boot slower than today.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::hlc::Hlc;
use crate::op::LogOp;
use crate::storage::StorageError;

/// One indexed entry: the HLC of an op + its byte offset inside the
/// `.jsonl` for that op's actor.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct IndexEntry {
    ts: Hlc,
    offset: u64,
}

/// In-memory offset index for a single actor's `.jsonl`.
///
/// Keyed by HLC (total order), so range queries ("every op after this
/// cutoff") are a `BTreeMap::range` away.
#[derive(Default, Debug)]
pub struct OffsetIndex {
    entries: BTreeMap<Hlc, u64>,
}

impl OffsetIndex {
    /// Build an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `ts` lives at `offset` in the underlying `.jsonl`.
    /// Inserting the same `(ts)` twice silently keeps the latest
    /// offset — the op log dedups by HLC on apply anyway.
    pub fn insert(&mut self, ts: Hlc, offset: u64) {
        self.entries.insert(ts, offset);
    }

    /// Look up the byte offset of the op identified by `ts`.
    pub fn get(&self, ts: &Hlc) -> Option<u64> {
        self.entries.get(ts).copied()
    }

    /// Offsets whose HLC sorts strictly after `cutoff`, in HLC order.
    pub fn after(&self, cutoff: Hlc) -> impl Iterator<Item = (&Hlc, &u64)> {
        self.entries.range((
            std::ops::Bound::Excluded(cutoff),
            std::ops::Bound::Unbounded,
        ))
    }

    /// Number of indexed ops.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds zero ops.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Load the index from `path`. Returns:
    ///
    /// - `Ok(Some(index))` when the file exists and parses cleanly.
    /// - `Ok(None)` when the file doesn't exist or is empty — caller
    ///   should rebuild.
    /// - `Err(_)` on a real I/O error.
    ///
    /// A truncated or otherwise malformed file logs a warning and
    /// returns `Ok(None)` so the caller can rebuild from the `.jsonl`.
    /// We never propagate parse failures as hard errors — the index is
    /// a cache.
    pub fn load(path: &Path) -> Result<Option<Self>, StorageError> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(StorageError::Backend(format!(
                    "open index {}: {e}",
                    path.display()
                )))
            }
        };
        let mut index = Self::new();
        let mut recovered = 0usize;
        for (lineno, line) in BufReader::new(file).lines().enumerate() {
            let raw = match line {
                Ok(l) if !l.is_empty() => l,
                Ok(_) => continue,
                Err(e) => {
                    warn!("index io error {}:{}: {e}", path.display(), lineno + 1);
                    return Ok(None);
                }
            };
            // Same glued-op recovery as the op log: stream every
            // concatenated JSON value off the line. Two index entries
            // glued together by an interleaved append should not lose
            // either side.
            let stream = serde_json::Deserializer::from_str(&raw).into_iter::<IndexEntry>();
            let mut saw_any = false;
            for item in stream {
                match item {
                    Ok(entry) => {
                        index.insert(entry.ts, entry.offset);
                        recovered += 1;
                        saw_any = true;
                    }
                    Err(e) => {
                        warn!(
                            "index parse {}:{}: {e} — rebuilding from .jsonl",
                            path.display(),
                            lineno + 1
                        );
                        return Ok(None);
                    }
                }
            }
            if !saw_any {
                warn!(
                    "index empty line {}:{} — rebuilding",
                    path.display(),
                    lineno + 1
                );
                return Ok(None);
            }
        }
        if recovered == 0 {
            return Ok(None);
        }
        debug!(
            "index loaded from {} ({} entries)",
            path.display(),
            recovered
        );
        Ok(Some(index))
    }

    /// Persist the full index to `path` atomically (tmp + rename).
    ///
    /// Used by `JsonlStorage` on shutdown and on explicit flush. The
    /// per-append hot path writes one line via [`Self::append_to`]
    /// instead, so this full save is for checkpointing only.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let tmp = path.with_extension("idx.tmp");
        let mut file = File::create(&tmp)
            .map_err(|e| StorageError::Backend(format!("create {}: {e}", tmp.display())))?;
        for (ts, offset) in &self.entries {
            let line = serde_json::to_string(&IndexEntry {
                ts: *ts,
                offset: *offset,
            })
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
            writeln!(file, "{line}")
                .map_err(|e| StorageError::Backend(format!("write {}: {e}", tmp.display())))?;
        }
        file.sync_all()
            .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", tmp.display())))?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(|e| {
            StorageError::Backend(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(())
    }

    /// Append a single entry to `path`. Cheap, durable, append-only —
    /// this is the hot path called from `JsonlStorage::append_op`.
    pub fn append_to(path: &Path, ts: Hlc, offset: u64) -> Result<(), StorageError> {
        let line = serde_json::to_string(&IndexEntry { ts, offset })
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| StorageError::Backend(format!("open index {}: {e}", path.display())))?;
        writeln!(file, "{line}")
            .map_err(|e| StorageError::Backend(format!("write index {}: {e}", path.display())))?;
        // No fsync here — the caller has just fsynced the .jsonl and
        // will eventually fsync the .idx via `save` on shutdown, or
        // rebuild on the next boot if the .idx is lost. The index is a
        // cache; we don't pay double fsync per op for it.
        Ok(())
    }

    /// Rebuild the index by streaming the `.jsonl` once and recording
    /// the byte offset of each parsed op.
    ///
    /// Slow (O(jsonl size)) but correct. Used on first boot, after
    /// corruption, and by `reload()`. Same cost as the legacy full
    /// load, so a missing index never makes us slower than today.
    pub fn rebuild_from_jsonl(jsonl_path: &Path) -> Result<Self, StorageError> {
        let file = File::open(jsonl_path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", jsonl_path.display())))?;
        let mut reader = BufReader::new(file);
        let mut index = Self::new();
        let mut offset: u64 = 0;
        let mut buf = String::new();
        loop {
            let start = offset;
            buf.clear();
            let n = reader.read_line(&mut buf).map_err(|e| {
                StorageError::Backend(format!("read {}: {e}", jsonl_path.display()))
            })?;
            if n == 0 {
                break;
            }
            let trimmed = buf.trim();
            if !trimmed.is_empty() {
                // Reuse the same glued-op recovery the JsonlStorage
                // reload path uses. An entry per recovered op, all
                // pointing at the same line offset.
                let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<LogOp>();
                for op in stream.flatten() {
                    index.insert(op.ts, start);
                }
            }
            offset += n as u64;
        }
        Ok(index)
    }
}

/// Lock-protected multi-actor index, keyed by actor id.
///
/// `JsonlStorage` keeps one `OffsetIndex` per `ops-<actor>.jsonl`. This
/// wrapper is the cheap lookup: `index_for(actor)` borrows under a read
/// lock; mutation goes through a write lock.
#[derive(Default)]
pub struct ActorIndex {
    inner: RwLock<std::collections::HashMap<crate::id::ActorId, OffsetIndex>>,
}

impl ActorIndex {
    /// Build an empty multi-actor index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh `(ts, offset)` for `actor`.
    pub fn insert(&self, actor: crate::id::ActorId, ts: Hlc, offset: u64) {
        self.inner
            .write()
            .entry(actor)
            .or_default()
            .insert(ts, offset);
    }

    /// Replace the entire index for one actor (used by `reload()`).
    pub fn replace(&self, actor: crate::id::ActorId, index: OffsetIndex) {
        self.inner.write().insert(actor, index);
    }

    /// Snapshot of the offset for `(actor, ts)`, if known.
    pub fn get(&self, actor: crate::id::ActorId, ts: Hlc) -> Option<u64> {
        self.inner.read().get(&actor).and_then(|i| i.get(&ts))
    }

    /// Total entries across every actor.
    pub fn total_len(&self) -> usize {
        self.inner.read().values().map(|i| i.len()).sum()
    }

    /// Path of the `.idx` sidecar for a given actor inside `ops_dir`.
    /// Public so `JsonlStorage` can compute the same path the index
    /// layer would.
    pub fn sidecar_path(ops_dir: &Path, actor: crate::id::ActorId) -> PathBuf {
        ops_dir.join(format!("ops-{actor}.idx"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::HlcGenerator;
    use crate::id::ActorId;
    use crate::op::Op;
    use tempfile::TempDir;

    fn logop(g: &HlcGenerator, op: Op) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    #[test]
    fn index_roundtrips_through_save_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("ops-test.idx");
        let mut index = OffsetIndex::new();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        for offset in [0u64, 128, 256, 1024] {
            let ts = g.next();
            index.insert(ts, offset);
        }
        index.save(&path).unwrap();
        let loaded = OffsetIndex::load(&path)
            .unwrap()
            .expect("index should load");
        assert_eq!(loaded.len(), 4);
        for ts in loaded.entries.keys() {
            assert_eq!(loaded.get(ts), index.get(ts));
        }
    }

    #[test]
    fn missing_index_loads_as_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.idx");
        assert!(OffsetIndex::load(&path).unwrap().is_none());
    }

    #[test]
    fn corrupt_index_rebuilds_as_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("corrupt.idx");
        std::fs::write(&path, b"not valid json at all").unwrap();
        assert!(OffsetIndex::load(&path).unwrap().is_none());
    }

    #[test]
    fn rebuild_from_jsonl_matches_offsets() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let jsonl = tmp.path().join(format!("ops-{actor}.jsonl"));

        let mk = || {
            logop(
                &g,
                Op::Create {
                    node: crate::id::NodeId::new(),
                    parent: crate::id::NodeId::root(),
                    position: crate::fractional::Fractional::first(),
                },
            )
        };
        let a = mk();
        let b = mk();
        let c = mk();

        // Write each op on its own line, tracking offsets.
        let line_a = serde_json::to_string(&a).unwrap();
        let line_b = serde_json::to_string(&b).unwrap();
        let line_c = serde_json::to_string(&c).unwrap();
        let body = format!("{line_a}\n{line_b}\n{line_c}\n");
        std::fs::write(&jsonl, &body).unwrap();

        let off_a = 0u64;
        let off_b = (line_a.len() + 1) as u64; // +1 for \n
        let off_c = off_b + (line_b.len() + 1) as u64;

        let index = OffsetIndex::rebuild_from_jsonl(&jsonl).unwrap();
        assert_eq!(index.len(), 3);
        assert_eq!(index.get(&a.ts), Some(off_a));
        assert_eq!(index.get(&b.ts), Some(off_b));
        assert_eq!(index.get(&c.ts), Some(off_c));
    }

    #[test]
    fn after_range_excludes_cutoff() {
        let mut index = OffsetIndex::new();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let t0 = g.next();
        let t1 = g.next();
        let t2 = g.next();
        index.insert(t0, 0);
        index.insert(t1, 10);
        index.insert(t2, 20);

        let after_t1: Vec<Hlc> = index.after(t1).map(|(ts, _)| *ts).collect();
        assert_eq!(after_t1, vec![t2]);
    }

    #[test]
    fn actor_index_routes_by_actor() {
        let ai = ActorIndex::new();
        let a = ActorId::new();
        let b = ActorId::new();
        let ga = HlcGenerator::new(a);
        let gb = HlcGenerator::new(b);
        let ta = ga.next();
        let tb = gb.next();

        ai.insert(a, ta, 100);
        ai.insert(b, tb, 200);

        assert_eq!(ai.get(a, ta), Some(100));
        assert_eq!(ai.get(b, tb), Some(200));
        assert_eq!(ai.get(a, tb), None); // wrong actor
        assert_eq!(ai.total_len(), 2);
    }
}
