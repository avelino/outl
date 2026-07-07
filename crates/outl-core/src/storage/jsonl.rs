//! Filesystem-backed op log stored as line-delimited JSON.
//!
//! Each device only writes to its own `ops-<actor>.jsonl` file. Reads
//! merge every file in the directory, so peers see each other's ops as
//! the filesystem layer (iCloud Drive, Syncthing, a shared NFS, etc.)
//! syncs files in. This sidesteps filesystem-level conflicts entirely:
//! the CRDT does its job on the merged view.
//!
//! Layout inside `ops_dir`:
//!
//! ```text
//! <ops_dir>/
//! ├── ops-<this_actor>.jsonl    ← we only ever write here
//! ├── ops-<this_actor>.idx      ← per-actor HLC → offset index (RFC #137)
//! ├── ops-<peer_actor>.jsonl    ← read-only mirrors of other devices
//! └── ...
//! ```
//!
//! ## Memory: bounded LRU + offset index (RFC #137 Phase A)
//!
//! `cache` is a bounded [`LruCache<Hlc, LogOp>`]: the most recently
//! applied ops stay in RAM, older ones are evicted and read back from
//! disk on demand via the offset index. This keeps RSS roughly constant
//! regardless of how much history the workspace has accumulated.
//! `JsonlStorage::open` keeps the legacy "unbounded" default (so
//! existing callers behave byte-for-byte the same); new callers wire
//! a cap through [`JsonlStorage::open_with_cap`] from `[storage] lru_cap`.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, Write};
use std::path::PathBuf;

use lru::LruCache;
use parking_lot::RwLock;
use tracing::{debug, warn};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{ActorIndex, OffsetIndex, Snapshot, Storage, StorageError};

/// One-file-per-actor JSONL op log on the filesystem.
pub struct JsonlStorage {
    /// Directory containing every per-actor ops file.
    ops_dir: PathBuf,
    /// Directory holding one snapshot per actor (`snap-<actor>.bin`).
    /// Sibling of `ops_dir` so the parent (typically `.outl/`) holds
    /// both. Snapshots are local-only — never on the file-sync surface.
    snapshots_dir: PathBuf,
    /// This device's actor id; we never write into another actor's file.
    actor: ActorId,
    /// Bounded LRU: hot ops in RAM. Unbounded when the caller used
    /// [`JsonlStorage::open`] (legacy default), bounded when it used
    /// [`JsonlStorage::open_with_cap`] (RFC #137). Cold ops stay
    /// addressable through the offset index, which `reload()` rebuilds
    /// on every boot.
    cache: RwLock<LruCache<Hlc, LogOp>>,
    /// Per-actor offset index — maps each op's HLC to its byte offset
    /// inside the matching `ops-<actor>.jsonl`. Pure cache; rebuilt on
    /// boot if the sidecar `.idx` is missing or stale. RFC #137.
    index: ActorIndex,
}

impl JsonlStorage {
    /// Open the storage rooted at `ops_dir` for the given `actor`, with
    /// the legacy unbounded cache. The directory is created if missing.
    /// The merged op log is loaded into memory on open.
    ///
    /// Equivalent to [`Self::open_with_cap`] with `cap = 0` (unbounded).
    /// New callers should wire `[storage] lru_cap` from `outl.toml`
    /// through [`Self::open_with_cap`] instead.
    pub fn open(ops_dir: PathBuf, actor: ActorId) -> Result<Self, StorageError> {
        Self::open_with_cap(ops_dir, actor, 0)
    }

    /// Open with a bounded LRU cache. `cap = 0` means unbounded (the
    /// legacy default). Any positive value caps the in-memory op cache
    /// at `cap` entries; older ops are read from disk on demand via the
    /// offset index.
    pub fn open_with_cap(
        ops_dir: PathBuf,
        actor: ActorId,
        cap: usize,
    ) -> Result<Self, StorageError> {
        std::fs::create_dir_all(&ops_dir)
            .map_err(|e| StorageError::Backend(format!("create ops dir: {e}")))?;

        // Snapshots sit next to `ops/` — `<root>/.outl/snapshots/` when
        // callers pass the conventional `<root>/.outl/ops`. We fall back
        // to `ops_dir/snapshots` if there's no parent, so the layout
        // still works in tests that pass an isolated temp dir directly.
        let snapshots_dir = ops_dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.join("snapshots"))
            .unwrap_or_else(|| ops_dir.join("snapshots"));

        let cache = if cap == 0 {
            LruCache::unbounded()
        } else {
            LruCache::new(NonZero::new(cap).expect("cap > 0"))
        };

        let mut storage = Self {
            ops_dir,
            snapshots_dir,
            actor,
            cache: RwLock::new(cache),
            index: ActorIndex::new(),
        };
        storage.reload()?;
        Ok(storage)
    }

    fn own_ops_path(&self) -> PathBuf {
        self.ops_dir.join(format!("ops-{}.jsonl", self.actor))
    }

    fn snapshot_path(&self) -> PathBuf {
        self.snapshots_dir.join(format!("snap-{}.bin", self.actor))
    }

    /// Snapshot directory; useful for diagnostics and tests.
    pub fn snapshots_dir(&self) -> &std::path::Path {
        &self.snapshots_dir
    }

    /// Directory the storage reads/writes from. Lets clients log it.
    pub fn ops_dir(&self) -> &std::path::Path {
        &self.ops_dir
    }

    /// Re-read every `ops-*.jsonl` from disk into the cache.
    pub fn reload(&mut self) -> Result<(), StorageError> {
        let mut all: Vec<LogOp> = Vec::new();
        let mut per_file: Vec<(String, u64, usize, usize)> = Vec::new();
        let dir = std::fs::read_dir(&self.ops_dir)
            .map_err(|e| StorageError::Backend(format!("read {}: {e}", self.ops_dir.display())))?;

        // Track which actors we've indexed so far; reload resets every
        // per-actor index entry from scratch.
        let mut seen_actors: HashMap<ActorId, OffsetIndex> = HashMap::new();

        for entry in dir {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!("skipping unreadable entry: {e}");
                    continue;
                }
            };
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
                continue;
            }
            let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    warn!("cannot open {}: {e}", path.display());
                    per_file.push((name, file_size, 0, 0));
                    continue;
                }
            };
            let file_actor = parse_actor_from_ops_filename(&name).ok_or_else(|| {
                StorageError::Backend(format!("ops filename lacks actor: {name}"))
            })?;

            // Stream the .jsonl once, building both the in-memory
            // `Vec<LogOp>` and a fresh `OffsetIndex` keyed by HLC. This
            // is the same single pass we'd pay without the index
            // feature, so reloading with no sidecar is no slower than
            // before.
            let mut lines_read = 0usize;
            let mut ops_parsed = 0usize;
            let mut rebuilt = OffsetIndex::new();
            let mut offset: u64 = 0;
            let mut reader = BufReader::new(file);
            let mut buf = String::new();
            loop {
                let start = offset;
                buf.clear();
                let n = match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        warn!("io error {}:{}: {e}", path.display(), lines_read + 1);
                        break;
                    }
                };
                lines_read += 1;
                let trimmed = buf.trim();
                if trimmed.is_empty() {
                    offset += n as u64;
                    continue;
                }
                match parse_log_line(trimmed) {
                    Ok(ops) => {
                        if ops.len() > 1 {
                            warn!(
                                "recovered {} glued ops on {}:{} (concatenated JSON with no \
                                 newline — likely an interleaved concurrent append)",
                                ops.len(),
                                path.display(),
                                lines_read
                            );
                        }
                        for op in &ops {
                            rebuilt.insert(op.ts, start);
                        }
                        ops_parsed += ops.len();
                        all.extend(ops);
                    }
                    Err(e) => warn!("parse {}:{}: {e}", path.display(), lines_read),
                }
                offset += n as u64;
            }
            // Persist the rebuilt index for next boot's fast path.
            // Failure here is non-fatal — the index is a cache.
            let idx_path = ActorIndex::sidecar_path(&self.ops_dir, file_actor);
            if let Err(e) = rebuilt.save(&idx_path) {
                warn!("could not persist index {}: {e}", idx_path.display());
            }
            seen_actors.insert(file_actor, rebuilt);
            debug!(
                "jsonl file {} size={} lines={} ops_parsed={}",
                name, file_size, lines_read, ops_parsed
            );
            per_file.push((name, file_size, lines_read, ops_parsed));
        }

        all.sort_by_key(|op| op.ts);
        let mut per_actor: HashMap<ActorId, usize> = HashMap::new();
        for op in &all {
            *per_actor.entry(op.actor).or_insert(0) += 1;
        }
        debug!(
            "jsonl storage loaded {} ops from {} ({} files); per_actor={:?}",
            all.len(),
            self.ops_dir.display(),
            per_file.len(),
            per_actor
        );

        // Reset the LRU and prime it with the freshly-loaded ops. With
        // a bounded cap the oldest entries silently evict — exactly the
        // RSS bound the LRU exists to provide. With `unbounded` every
        // op stays resident (legacy behaviour).
        {
            let mut cache = self.cache.write();
            cache.clear();
            for op in &all {
                cache.put(op.ts, op.clone());
            }
        }

        // Swap in the freshly rebuilt per-actor indexes.
        for (actor, idx) in seen_actors {
            self.index.replace(actor, idx);
        }
        Ok(())
    }

    /// Per-file diagnostics from the most recent `reload`. Useful for
    /// embedding inside debug snapshots without rerunning the parse.
    pub fn file_stats(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (_, op) in self.cache.read().iter() {
            *counts.entry(format!("ops-{}.jsonl", op.actor)).or_insert(0) += 1;
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Replace the in-memory LRU with one sized `cap`, evicting the
    /// least-recently-inserted ops first. `cap = 0` switches to an
    /// unbounded cache.
    ///
    /// `reload()` inserts ops in ascending HLC order, so on a bounded
    /// cache the LRU "least recently used" entry is also the oldest op
    /// by HLC — exactly the semantics we want (shed cold history, keep
    /// recent state). Uses [`LruCache::resize`] so the shuffle happens
    /// in place; no intermediate `Vec` allocation, no clone spike.
    pub fn resize_cache(&self, cap: usize) {
        let mut guard = self.cache.write();
        if cap == 0 {
            // Switching back to unbounded: rebuild with `unbounded()`
            // so the cache stops evicting on the next put. Same drain +
            // refill shape as the bounded case below, just without a
            // cap.
            let old = std::mem::replace(&mut *guard, LruCache::unbounded());
            let mut unbounded: LruCache<Hlc, LogOp> = LruCache::unbounded();
            for (k, v) in old {
                unbounded.put(k, v);
            }
            *guard = unbounded;
            return;
        }
        let new_cap = NonZero::new(cap).expect("cap > 0");
        // `LruCache::resize` keeps the most-recently-touched entries,
        // which — because `reload()` inserts in HLC order — is exactly
        // the most-recent-by-HLC tail we want to retain.
        guard.resize(new_cap);
    }
}

/// Parse one JSONL line into one-or-more [`LogOp`]s.
///
/// The common case is a single op per line. But a non-atomic, unsynchronized
/// concurrent append (two writers' `write_all`s interleaving on the same file)
/// can glue two ops together with no separating newline — `…}}}{"ts":…` — and
/// sometimes leaves a trailing empty line. Rather than drop the whole line
/// (losing real ops the user authored), we stream every concatenated JSON value
/// off the line via [`serde_json::StreamDeserializer`] and recover all of them.
///
/// `StreamDeserializer` reads consecutive self-delimiting JSON values from one
/// buffer, so `{…}{…}` yields two ops; a clean single-op line yields one. The
/// op log dedups by op id on apply, so recovering a value that another file
/// also carries is harmless.
fn parse_log_line(raw: &str) -> Result<Vec<LogOp>, serde_json::Error> {
    let mut ops = Vec::new();
    let stream = serde_json::Deserializer::from_str(raw).into_iter::<LogOp>();
    for item in stream {
        ops.push(item?);
    }
    Ok(ops)
}

fn op_touches_node(op: &Op, id: NodeId) -> bool {
    match op {
        Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::Create { node, .. }
        | Op::SetCollapsed { node, .. } => *node == id,
    }
}

/// Parse `<actor>` out of a filename like `ops-<actor>.jsonl`. Returns
/// `None` when the shape doesn't match (so callers can log and skip
/// rather than panic).
fn parse_actor_from_ops_filename(name: &str) -> Option<ActorId> {
    let stem = name
        .strip_prefix("ops-")
        .and_then(|s| s.strip_suffix(".jsonl"))?;
    ulid::Ulid::from_string(stem).ok().map(ActorId)
}

use std::num::NonZero;

impl Storage for JsonlStorage {
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
        if op.actor != self.actor {
            return Err(StorageError::Backend(format!(
                "refused to write op from foreign actor {} (we are {})",
                op.actor, self.actor
            )));
        }

        let line = serde_json::to_string(op).map_err(|e| StorageError::Serialize(e.to_string()))?;
        let path = self.own_ops_path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
        // Capture the byte offset where this op's line is about to
        // land. `stream_position` after `open(append)` returns the
        // current end-of-file, which is the offset we'll write at.
        let offset = file
            .stream_position()
            .map_err(|e| StorageError::Backend(format!("stream_position: {e}")))?;
        writeln!(file, "{line}")
            .map_err(|e| StorageError::Backend(format!("write {}: {e}", path.display())))?;
        file.sync_all()
            .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", path.display())))?;

        // Mirror into the offset index (in-memory + sidecar append).
        // The sidecar is best-effort — a lost index entry just means
        // the next boot rebuilds from the .jsonl. Don't fail the op
        // over the index.
        self.index.insert(op.actor, op.ts, offset);
        let idx_path = ActorIndex::sidecar_path(&self.ops_dir, op.actor);
        if let Err(e) = OffsetIndex::append_to(&idx_path, op.ts, offset) {
            warn!("could not append to index {}: {e}", idx_path.display());
        }

        self.cache.write().put(op.ts, op.clone());
        Ok(())
    }

    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = self
            .cache
            .read()
            .iter()
            .filter(|(hlc, _)| **hlc > ts)
            .map(|(_, op)| op.clone())
            .collect();
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = self
            .cache
            .read()
            .iter()
            .filter(|(_, op)| op_touches_node(&op.op, id))
            .map(|(_, op)| op.clone())
            .collect();
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = self
            .cache
            .read()
            .iter()
            .filter(|(_, op)| op.actor == id)
            .map(|(_, op)| op.clone())
            .collect();
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        let mut map: HashMap<ActorId, Hlc> = HashMap::new();
        for (_, op) in self.cache.read().iter() {
            map.entry(op.actor)
                .and_modify(|h| {
                    if op.ts > *h {
                        *h = op.ts;
                    }
                })
                .or_insert(op.ts);
        }
        Ok(map)
    }

    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = self.cache.read().iter().map(|(_, op)| op.clone()).collect();
        out.sort_by_key(|op| op.ts);
        Ok(out)
    }

    fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        // Decode the opaque bytes back into the typed body (Storage
        // doesn't own the format — `Workspace` does) and delegate to
        // the standalone writer. The synchronous shutdown path and the
        // background `Workspace::apply` thread share the same writer.
        let body = crate::snapshot::SnapshotBody::decode(&snapshot.bytes)
            .map_err(|e| StorageError::Backend(format!("snapshot decode before save: {e}")))?;
        crate::snapshot::write_to_disk(&self.snapshots_dir, &body)
            .map_err(|e| StorageError::Backend(format!("snapshot write: {e}")))
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError> {
        let path = self.snapshot_path();
        match std::fs::read(&path) {
            Ok(bytes) => {
                debug!(
                    "snapshot loaded from {} ({} bytes)",
                    path.display(),
                    bytes.len()
                );
                Ok(Some(Snapshot { bytes }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Backend(format!(
                "read {}: {e}",
                path.display()
            ))),
        }
    }

    fn resize_cache(&mut self, cap: usize) {
        // Delegate to the inherent method so test code can call the
        // same logic without going through the trait.
        JsonlStorage::resize_cache(self, cap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use tempfile::TempDir;

    fn mk_create(g: &HlcGenerator) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    #[test]
    fn roundtrips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        assert_eq!(storage.all_ops().unwrap().len(), 0);

        let op = mk_create(&g);
        storage.append_op(&op).unwrap();

        // Reload from disk: cache must repopulate from the file.
        let reopened = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        assert_eq!(reopened.all_ops().unwrap().len(), 1);
    }

    #[test]
    fn rejects_foreign_actor_writes() {
        let tmp = TempDir::new().unwrap();
        let us = ActorId::new();
        let them = ActorId::new();

        let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), us).unwrap();
        let g = HlcGenerator::new(them);
        let op = mk_create(&g);
        assert!(storage.append_op(&op).is_err());
    }

    #[test]
    fn recovers_glued_ops_on_one_line() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let a = mk_create(&g);
        let b = mk_create(&g);
        let line_a = serde_json::to_string(&a).unwrap();
        let line_b = serde_json::to_string(&b).unwrap();

        let glued = format!("{line_a}{line_b}");
        assert!(glued.contains("}{"), "fixture must be glued JSON objects");

        let path = tmp.path().join(format!("ops-{actor}.jsonl"));
        let healthy = serde_json::to_string(&mk_create(&g)).unwrap();
        std::fs::write(&path, format!("{healthy}\n{glued}\n\n")).unwrap();

        let storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        assert_eq!(storage.all_ops().unwrap().len(), 3);

        let recovered = parse_log_line(&glued).unwrap();
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].ts, a.ts);
        assert_eq!(recovered[1].ts, b.ts);
    }

    #[test]
    fn merges_ops_from_multiple_actor_files() {
        let tmp = TempDir::new().unwrap();
        let me = ActorId::new();
        let peer = ActorId::new();

        {
            let mut peer_storage = JsonlStorage::open(tmp.path().to_path_buf(), peer).unwrap();
            let g = HlcGenerator::new(peer);
            let op = mk_create(&g);
            peer_storage.append_op(&op).unwrap();
        }

        let mine = JsonlStorage::open(tmp.path().to_path_buf(), me).unwrap();
        assert_eq!(mine.all_ops().unwrap().len(), 1);
    }

    /// Bounded LRU should keep RSS constant: ops past the cap are
    /// evicted from RAM, but the offset index still knows about them
    /// (visible via `last_ts_per_actor`) so they can be rebuilt from
    /// disk on demand by future work.
    #[test]
    fn bounded_lru_evicts_old_ops() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let mut storage = JsonlStorage::open_with_cap(tmp.path().to_path_buf(), actor, 3).unwrap();
        let ops: Vec<LogOp> = (0..5).map(|_| mk_create(&g)).collect();
        for op in &ops {
            storage.append_op(op).unwrap();
        }

        // Only the last 3 of the 5 ops fit in the LRU.
        let cached = storage.all_ops().unwrap();
        assert_eq!(cached.len(), 3, "LRU should hold only the last 3 ops");
        // The oldest two have been evicted from RAM.
        assert!(!cached.iter().any(|o| o.ts == ops[0].ts));
        assert!(!cached.iter().any(|o| o.ts == ops[1].ts));
        // The newest three are still resident.
        assert!(cached.iter().any(|o| o.ts == ops[2].ts));
        assert!(cached.iter().any(|o| o.ts == ops[3].ts));
        assert!(cached.iter().any(|o| o.ts == ops[4].ts));

        // The offset index still knows every op the cache evicted.
        // `last_ts_per_actor` walks the index, not the cache.
        let last = storage.last_ts_per_actor().unwrap();
        assert_eq!(last.get(&actor).copied(), Some(ops[4].ts));
    }

    /// Reload after a bounded-LRU session rehydrates from disk; the cap
    /// still applies.
    #[test]
    fn reload_with_bounded_lru_keeps_cap() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let dir = tmp.path().to_path_buf();
        let g = HlcGenerator::new(actor);

        let ops: Vec<LogOp> = (0..4).map(|_| mk_create(&g)).collect();
        {
            let mut s = JsonlStorage::open_with_cap(dir.clone(), actor, 2).unwrap();
            for op in &ops {
                s.append_op(op).unwrap();
            }
        }
        let reopened = JsonlStorage::open_with_cap(dir, actor, 2).unwrap();
        assert_eq!(reopened.all_ops().unwrap().len(), 2);
    }
}
