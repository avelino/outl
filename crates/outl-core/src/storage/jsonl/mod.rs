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

mod append;
mod read;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::num::NonZero;
use std::path::PathBuf;

use lru::LruCache;
use parking_lot::RwLock;

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{ActorIndex, ActorNodeIndex, PageScope, Storage, StorageError};

/// One-file-per-actor JSONL op log on the filesystem.
///
/// The on-disk layout depends on the [`PageScope`] the storage was
/// opened with:
///
/// - `PageScope::Global` (legacy): `ops/ops-<actor>.jsonl`
/// - `PageScope::PerPage(slug)`: `ops/<actor>/<slug>.jsonl`
///
/// Sidecars (`.idx`, `.nodes.idx`) live next to the `.jsonl` in both
/// layouts. See `own_ops_path` for the exact routing.
pub struct JsonlStorage {
    /// Directory containing every per-actor ops file (Global) or every
    /// per-actor subdirectory (PerPage).
    ops_dir: PathBuf,
    /// This device's actor id; we never write into another actor's file.
    actor: ActorId,
    /// Scope this storage is responsible for. Determines path layout.
    scope: PageScope,
    /// Bounded LRU: hot ops in RAM. Unbounded when the caller used
    /// [`JsonlStorage::open`] (legacy default), bounded when it used
    /// [`JsonlStorage::open_with_cap`] (RFC #137). Cold ops are read
    /// back from the `.jsonl` via [`Self::read_op_at`].
    cache: RwLock<LruCache<Hlc, LogOp>>,
    /// Per-actor HLC offset index — `HLC → byte offset in .jsonl`.
    /// Pure cache; rebuilt on boot if the sidecar `.idx` is missing
    /// or stale. RFC #137.
    index: ActorIndex,
    /// Per-actor secondary index — `NodeId → Vec<(Hlc, offset)>`.
    /// Powers `ops_for_node` without scanning the whole file. Same
    /// cache semantics as `index`. RFC #137 Phase A.
    node_index: ActorNodeIndex,
}

impl JsonlStorage {
    /// Open the storage rooted at `ops_dir` for the given `actor`, with
    /// the legacy unbounded cache and `PageScope::Global`. The directory
    /// is created if missing. The merged op log is loaded into memory
    /// on open.
    ///
    /// Equivalent to [`Self::open_with_cap`] with `cap = 0` (unbounded).
    /// New callers should wire `[storage] lru_cap` from `outl.toml`
    /// through [`Self::open_with_cap`] instead.
    pub fn open(ops_dir: PathBuf, actor: ActorId) -> Result<Self, StorageError> {
        Self::open_with_cap(ops_dir, actor, 0)
    }

    /// Open with a bounded LRU cache under `PageScope::Global`. `cap = 0`
    /// means unbounded (the legacy default).
    pub fn open_with_cap(
        ops_dir: PathBuf,
        actor: ActorId,
        cap: usize,
    ) -> Result<Self, StorageError> {
        Self::open_with_scope_cap(ops_dir, actor, PageScope::Global, cap)
    }

    /// Open with explicit scope and LRU cap. This is the constructor
    /// Phase B (RFC #137) wires through `outl init --scope=per-page`
    /// and `outl migrate-to-per-page-ops`.
    ///
    /// Layout:
    /// - `PageScope::Global` → `ops/ops-<actor>.jsonl` (legacy)
    /// - `PageScope::PerPage(slug)` → `ops/<actor>/<slug>.jsonl`
    pub fn open_with_scope_cap(
        ops_dir: PathBuf,
        actor: ActorId,
        scope: PageScope,
        cap: usize,
    ) -> Result<Self, StorageError> {
        // Make sure the directory that will hold the .jsonl exists.
        // For Global that's `ops_dir` itself; for PerPage it's
        // `ops_dir/<actor>/`.
        let own_dir = match &scope {
            PageScope::Global => ops_dir.clone(),
            PageScope::PerPage(_) => ops_dir.join(actor.to_string()),
        };
        std::fs::create_dir_all(&own_dir)
            .map_err(|e| StorageError::Backend(format!("create ops dir: {e}")))?;

        let cache = if cap == 0 {
            LruCache::unbounded()
        } else {
            LruCache::new(NonZero::new(cap).expect("cap > 0"))
        };

        let mut storage = Self {
            ops_dir,
            actor,
            scope,
            cache: RwLock::new(cache),
            index: ActorIndex::new(),
            node_index: ActorNodeIndex::new(),
        };
        storage.reload()?;
        Ok(storage)
    }

    fn own_ops_path(&self) -> PathBuf {
        match &self.scope {
            PageScope::Global => self.ops_dir.join(format!("ops-{}.jsonl", self.actor)),
            PageScope::PerPage(slug) => self
                .ops_dir
                .join(format!("{actor}", actor = self.actor))
                .join(format!("{slug}.jsonl")),
        }
    }

    fn own_ops_dir(&self) -> PathBuf {
        match &self.scope {
            PageScope::Global => self.ops_dir.clone(),
            PageScope::PerPage(_) => self.ops_dir.join(self.actor.to_string()),
        }
    }

    /// Directory the storage reads/writes from. Lets clients log it.
    pub fn ops_dir(&self) -> &std::path::Path {
        &self.ops_dir
    }

    /// Scope this storage was opened with.
    pub fn scope(&self) -> &PageScope {
        &self.scope
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
pub(super) fn parse_log_line(raw: &str) -> Result<Vec<LogOp>, serde_json::Error> {
    let mut ops = Vec::new();
    let stream = serde_json::Deserializer::from_str(raw).into_iter::<LogOp>();
    for item in stream {
        ops.push(item?);
    }
    Ok(ops)
}

/// One physical record read from an op-log file, tolerant of the ways a
/// synced or crash-interrupted `.jsonl` can be malformed.
pub(super) enum RecordRead {
    /// Clean end of file.
    Eof,
    /// A parsed record (one op, or several recovered from a glued line) and
    /// the number of bytes it spanned on disk.
    Ops { consumed: u64, ops: Vec<LogOp> },
    /// A byte span carrying no usable op — a blank line, non-UTF8 bytes (a
    /// partial sync can leave them mid-file), or JSON that didn't parse. The
    /// span length is still reported so the caller keeps a correct offset and
    /// reads the *next* record instead of aborting the whole file. `reason` is
    /// `Some` for a real defect (worth a warning), `None` for a benign blank.
    Skip {
        consumed: u64,
        reason: Option<String>,
    },
}

/// Read one newline-delimited record, never failing on encoding.
///
/// Unlike [`BufRead::read_line`], this reads raw bytes via `read_until`, so a
/// non-UTF8 byte in the middle of a file does not abort the read — it is
/// surfaced as a skippable record. Only a genuine device I/O error returns
/// `Err`. This is what lets one corrupt line (torn tail, partial sync) cost a
/// single record instead of every op after it in the file.
pub(super) fn read_log_record<R: std::io::BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<RecordRead> {
    buf.clear();
    let n = reader.read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(RecordRead::Eof);
    }
    let consumed = n as u64;
    let Ok(text) = std::str::from_utf8(buf) else {
        return Ok(RecordRead::Skip {
            consumed,
            reason: Some("non-UTF8 bytes".to_string()),
        });
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(RecordRead::Skip {
            consumed,
            reason: None,
        });
    }
    match parse_log_line(trimmed) {
        Ok(ops) => Ok(RecordRead::Ops { consumed, ops }),
        Err(e) => Ok(RecordRead::Skip {
            consumed,
            reason: Some(e.to_string()),
        }),
    }
}

pub(super) fn op_touches_node(op: &Op, id: NodeId) -> bool {
    match op {
        Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::Create { node, .. }
        | Op::SetCollapsed { node, .. } => *node == id,
    }
}

/// Extract the `NodeId` an op targets, if any. Every `Op` variant
/// carries one — there is no op that touches zero nodes. Returns
/// `Option` so callers can `filter_map` cleanly.
pub(super) fn op_node(op: &Op) -> Option<NodeId> {
    match op {
        Op::Create { node, .. }
        | Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::SetCollapsed { node, .. } => Some(*node),
    }
}

/// Parse `<actor>` out of a filename like `ops-<actor>.jsonl`. Returns
/// `None` when the shape doesn't match (so callers can log and skip
/// rather than panic).
pub(super) fn parse_actor_from_ops_filename(name: &str) -> Option<ActorId> {
    let stem = name
        .strip_prefix("ops-")
        .and_then(|s| s.strip_suffix(".jsonl"))?;
    ulid::Ulid::from_string(stem).ok().map(ActorId)
}

impl Storage for JsonlStorage {
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
        self.append_op_inner(op)
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
        // Hot path: cache hits cover recent ops.
        let cache = self.cache.read();
        let mut warm: Vec<LogOp> = cache
            .iter()
            .filter(|(_, op)| op_touches_node(&op.op, id))
            .map(|(_, op)| op.clone())
            .collect();
        drop(cache);
        if warm.is_empty() {
            // No warm hits at all; fall back to the per-node index.
            return self.cold_ops_for_node(id);
        }
        warm.sort_by_key(|op| op.ts);
        Ok(warm)
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

    fn resize_cache(&mut self, cap: usize) {
        // Delegate to the inherent method so test code can call the
        // same logic without going through the trait.
        JsonlStorage::resize_cache(self, cap);
    }
}
