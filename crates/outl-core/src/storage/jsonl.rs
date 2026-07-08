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
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;

use lru::LruCache;
use parking_lot::RwLock;
use tracing::{debug, warn};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{
    ActorIndex, ActorNodeIndex, NodeIndex, OffsetIndex, PageScope, Snapshot, Storage, StorageError,
};

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
    /// Directory holding one snapshot per actor (`snap-<actor>.bin`).
    /// Sibling of `ops_dir` so the parent (typically `.outl/`) holds
    /// both. Snapshots are local-only — never on the file-sync surface.
    snapshots_dir: PathBuf,
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

    fn snapshot_path(&self) -> PathBuf {
        // Per-page snapshots are namespaced by slug so multiple pages
        // don't clobber each other under `snapshots/`.
        match &self.scope {
            PageScope::Global => self.snapshots_dir.join(format!("snap-{}.bin", self.actor)),
            PageScope::PerPage(slug) => self
                .snapshots_dir
                .join(format!("snap-{}-{slug}.bin", self.actor)),
        }
    }

    /// Snapshot directory; useful for diagnostics and tests.
    pub fn snapshots_dir(&self) -> &std::path::Path {
        &self.snapshots_dir
    }

    /// Directory the storage reads/writes from. Lets clients log it.
    pub fn ops_dir(&self) -> &std::path::Path {
        &self.ops_dir
    }

    /// Scope this storage was opened with.
    pub fn scope(&self) -> &PageScope {
        &self.scope
    }

    /// Read a single op from disk by `(actor, ts)`. Returns `None`
    /// when the HLC isn't in the offset index or the read fails.
    ///
    /// Cold path: only used when an op has been evicted from the LRU.
    /// Hot ops come straight out of `cache`. Uses seek + line read
    /// (no mmap) so the `unsafe`-free invariant of the crate holds.
    ///
    /// Under `PageScope::PerPage`, the offset index only knows about
    /// `self.actor`'s ops for this page (peer ops under PerPage come
    /// from their own `<peer-actor>/<slug>.jsonl` file). Cross-actor
    /// cold reads walk the per-actor file directory.
    fn read_op_at(&self, actor: ActorId, ts: Hlc) -> Option<LogOp> {
        let offset = self.index.get(actor, ts)?;
        let path = self.ops_path_for_actor(actor)?;
        let mut file = File::open(&path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut line = String::new();
        BufReader::new(file).read_line(&mut line).ok()?;
        if line.is_empty() {
            return None;
        }
        // A glued line yields multiple ops; pick the one whose HLC
        // matches (same recovery semantics as the boot path).
        let ops = parse_log_line(line.trim()).ok()?;
        ops.into_iter().find(|o| o.ts == ts)
    }

    /// Path of the `.jsonl` for `actor` under this storage's scope.
    /// Under Global that's `ops/ops-<actor>.jsonl`; under PerPage it's
    /// `ops/<actor>/<slug>.jsonl` (and only `self.actor` is expected).
    fn ops_path_for_actor(&self, actor: ActorId) -> Option<PathBuf> {
        match &self.scope {
            PageScope::Global => Some(self.ops_dir.join(format!("ops-{actor}.jsonl"))),
            PageScope::PerPage(slug) => {
                if actor == self.actor {
                    Some(
                        self.ops_dir
                            .join(self.actor.to_string())
                            .join(format!("{slug}.jsonl")),
                    )
                } else {
                    // PerPage doesn't currently merge peer files; the
                    // caller (cold_ops_for_node) iterates this actor
                    // only. Returning None makes the cold read a no-op
                    // for peers, which is correct under Phase B's
                    // single-page-per-storage model.
                    None
                }
            }
        }
    }

    /// Re-read every `ops-*.jsonl` from disk into the cache.
    ///
    /// Under `PageScope::Global` this scans every `ops-<actor>.jsonl`
    /// in `ops_dir` (legacy multi-actor merge). Under
    /// `PageScope::PerPage(slug)` it only opens the single file for
    /// `(this actor, this slug)` — that's the whole point of Phase B:
    /// boot is proportional to one page, not the whole workspace.
    pub fn reload(&mut self) -> Result<(), StorageError> {
        let scope = self.scope.clone();
        match scope {
            PageScope::Global => self.reload_global(),
            PageScope::PerPage(slug) => self.reload_per_page(&slug),
        }
    }

    fn reload_global(&mut self) -> Result<(), StorageError> {
        let mut all: Vec<LogOp> = Vec::new();
        let mut per_file: Vec<(String, u64, usize, usize)> = Vec::new();
        let dir = std::fs::read_dir(&self.ops_dir)
            .map_err(|e| StorageError::Backend(format!("read {}: {e}", self.ops_dir.display())))?;

        // Reset every per-actor index — reload rebuilds from scratch.
        let mut seen_hlc: HashMap<ActorId, OffsetIndex> = HashMap::new();
        let mut seen_node: HashMap<ActorId, NodeIndex> = HashMap::new();

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

            let mut lines_read = 0usize;
            let mut ops_parsed = 0usize;
            let mut rebuilt_hlc = OffsetIndex::new();
            let mut rebuilt_node = NodeIndex::new();
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
                            rebuilt_hlc.insert(op.ts, start);
                            if let Some(node) = op_node(&op.op) {
                                rebuilt_node.insert(node, op.ts, start);
                            }
                        }
                        ops_parsed += ops.len();
                        all.extend(ops);
                    }
                    Err(e) => warn!("parse {}:{}: {e}", path.display(), lines_read),
                }
                offset += n as u64;
            }
            // Persist both sidecars next to the .jsonl.
            let hlc_path = ActorIndex::sidecar_path(&self.ops_dir, file_actor);
            if let Err(e) = rebuilt_hlc.save(&hlc_path) {
                warn!("could not persist index {}: {e}", hlc_path.display());
            }
            let node_path = ActorNodeIndex::sidecar_path(&self.ops_dir, file_actor);
            if let Err(e) = rebuilt_node.save(&node_path) {
                warn!("could not persist node index {}: {e}", node_path.display());
            }
            seen_hlc.insert(file_actor, rebuilt_hlc);
            seen_node.insert(file_actor, rebuilt_node);
            debug!(
                "jsonl file {} size={} lines={} ops_parsed={}",
                name, file_size, lines_read, ops_parsed
            );
            per_file.push((name, file_size, lines_read, ops_parsed));
        }

        all.sort_by_key(|op| op.ts);
        debug!(
            "jsonl storage (global) loaded {} ops from {} ({} files)",
            all.len(),
            self.ops_dir.display(),
            per_file.len()
        );

        {
            let mut cache = self.cache.write();
            cache.clear();
            for op in &all {
                cache.put(op.ts, op.clone());
            }
        }
        for (actor, idx) in seen_hlc {
            self.index.replace(actor, idx);
        }
        for (actor, idx) in seen_node {
            self.node_index.replace(actor, idx);
        }
        Ok(())
    }

    /// Reload under `PageScope::PerPage(slug)`. Opens exactly one file:
    /// `ops/<actor>/<slug>.jsonl`. Boot cost is O(this page's history),
    /// not O(workspace history).
    fn reload_per_page(&mut self, slug: &str) -> Result<(), StorageError> {
        let path = self.own_ops_path();
        let mut all: Vec<LogOp> = Vec::new();
        let mut rebuilt_hlc = OffsetIndex::new();
        let mut rebuilt_node = NodeIndex::new();

        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Page has no ops yet — fresh workspace, nothing to load.
                self.cache.write().clear();
                self.index.replace(self.actor, OffsetIndex::new());
                self.node_index.replace(self.actor, NodeIndex::new());
                return Ok(());
            }
            Err(e) => {
                return Err(StorageError::Backend(format!(
                    "open {}: {e}",
                    path.display()
                )))
            }
        };

        let mut offset: u64 = 0;
        let mut reader = BufReader::new(file);
        let mut buf = String::new();
        let mut lines_read = 0usize;
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
                    for op in &ops {
                        rebuilt_hlc.insert(op.ts, start);
                        if let Some(node) = op_node(&op.op) {
                            rebuilt_node.insert(node, op.ts, start);
                        }
                    }
                    all.extend(ops);
                }
                Err(e) => warn!("parse {}:{}: {e}", path.display(), lines_read),
            }
            offset += n as u64;
        }

        let own_dir = self.own_ops_dir();
        let hlc_path = ActorIndex::sidecar_path(&own_dir, self.actor);
        if let Err(e) = rebuilt_hlc.save(&hlc_path) {
            warn!("could not persist index {}: {e}", hlc_path.display());
        }
        let node_path = ActorNodeIndex::sidecar_path(&own_dir, self.actor);
        if let Err(e) = rebuilt_node.save(&node_path) {
            warn!("could not persist node index {}: {e}", node_path.display());
        }

        debug!(
            "jsonl storage (per-page {}) loaded {} ops from {} ({} lines)",
            slug,
            all.len(),
            path.display(),
            lines_read
        );

        {
            let mut cache = self.cache.write();
            cache.clear();
            for op in &all {
                cache.put(op.ts, op.clone());
            }
        }
        self.index.replace(self.actor, rebuilt_hlc);
        self.node_index.replace(self.actor, rebuilt_node);
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
    /// Cold-path `ops_for_node` when the LRU has no warm entry for the
    /// node. Walks the per-node secondary index across every known
    /// actor and pulls each op from the cache (if still resident) or
    /// the disk file via [`Self::read_op_at`]. RFC #137 Phase A.
    fn cold_ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let mut out: Vec<LogOp> = Vec::new();
        // Derive the actor set from the node index (persistent), not
        // from the LRU cache (eviction-sensitive). If all ops for a
        // peer actor were evicted, the index still tracks them.
        for actor in self.node_index.actors() {
            let entries = self.node_index.get(actor, id);
            for (ts, _offset) in entries {
                // Cache hit first.
                if let Some(op) = self.cache.read().peek(&ts).cloned() {
                    out.push(op);
                    continue;
                }
                // Cold: read from disk via the offset index.
                if let Some(op) = self.read_op_at(actor, ts) {
                    out.push(op);
                }
            }
        }
        out.sort_by_key(|op| op.ts);
        Ok(out)
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

/// Extract the `NodeId` an op targets, if any. Every `Op` variant
/// carries one — there is no op that touches zero nodes. Returns
/// `Option` so callers can `filter_map` cleanly.
fn op_node(op: &Op) -> Option<NodeId> {
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
        // Capture the byte offset where this op's line will land BEFORE
        // opening the file in append mode. POSIX `O_APPEND` (set by
        // `OpenOptions::append(true)`) does not update the file offset
        // for `stream_position()` until after the next write, so reading
        // `stream_position()` post-open would return 0 every time.
        // `metadata.len()` is the current end-of-file, which is the
        // offset we are about to write at.
        let offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
        writeln!(file, "{line}")
            .map_err(|e| StorageError::Backend(format!("write {}: {e}", path.display())))?;
        file.sync_all()
            .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", path.display())))?;

        // Mirror into both indexes (in-memory + sidecar append). Sidecar
        // failures are best-effort — the index is a cache, a missing
        // entry just means the next boot rebuilds from the .jsonl.
        let own_dir = self.own_ops_dir();
        self.index.insert(op.actor, op.ts, offset);
        let hlc_idx_path = ActorIndex::sidecar_path(&own_dir, op.actor);
        if let Err(e) = OffsetIndex::append_to(&hlc_idx_path, op.ts, offset) {
            warn!("could not append to index {}: {e}", hlc_idx_path.display());
        }
        if let Some(node) = op_node(&op.op) {
            self.node_index.insert(op.actor, node, op.ts, offset);
            let node_idx_path = ActorNodeIndex::sidecar_path(&own_dir, op.actor);
            if let Err(e) = NodeIndex::append_to(&node_idx_path, node, op.ts, offset) {
                warn!(
                    "could not append to node index {}: {e}",
                    node_idx_path.display()
                );
            }
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

    /// After LRU eviction, `ops_for_node` must still return every op
    /// that touched the node — pulled back from disk via the per-node
    /// secondary index. This is the correctness guarantee RFC #137
    /// Phase A needs: shedding cold history can't lose data.
    #[test]
    fn ops_for_node_survives_lru_eviction() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // Use the legacy unbounded API so we can append without LRU
        // eviction interfering, then explicitly shrink afterwards.
        let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        let target = NodeId::new();
        // 5 edits on `target`, 1 edit on filler nodes between each so
        // they push `target` ops out of a small LRU.
        let mut target_ts: Vec<Hlc> = Vec::new();
        for _ in 0..5 {
            let ts = g.next();
            target_ts.push(ts);
            storage
                .append_op(&LogOp {
                    ts,
                    actor,
                    op: Op::Edit {
                        node: target,
                        text_op: vec![1, 2, 3, 4],
                    },
                })
                .unwrap();
            // Filler edit on a different node — 5 of these between
            // each target edit means a cap of 5 leaves all target ops
            // evicted.
            let _ = g.next();
            storage
                .append_op(&LogOp {
                    ts: g.next(),
                    actor,
                    op: Op::Edit {
                        node: NodeId::new(),
                        text_op: vec![5, 6],
                    },
                })
                .unwrap();
        }

        // Sanity: pre-shrink, ops_for_node returns every target op.
        let pre = storage.ops_for_node(target).unwrap();
        assert_eq!(pre.len(), 5);

        // Shrink the LRU so all target ops get evicted. cap=1 keeps
        // only the very last put (a filler), so every target op
        // becomes a cold read.
        storage.resize_cache(1);
        // Cache no longer holds any target op.
        let cached = storage.all_ops().unwrap();
        assert!(
            !cached.iter().any(|o| op_touches_node(&o.op, target)),
            "target ops should be evicted by resize_cache(1)"
        );

        // Cold path must still find every target op via the per-node
        // index + offset index.
        let post = storage.ops_for_node(target).unwrap();
        assert_eq!(
            post.len(),
            5,
            "ops_for_node must return every op even when the LRU has evicted them all"
        );
        // Same HLCs as we appended.
        let mut post_ts: Vec<Hlc> = post.iter().map(|o| o.ts).collect();
        post_ts.sort();
        let mut expected = target_ts.clone();
        expected.sort();
        assert_eq!(post_ts, expected);
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

    /// `PageScope::PerPage` writes ops to `ops/<actor>/<slug>.jsonl`,
    /// not the legacy `ops-<actor>.jsonl`. Boot reads them back from
    /// the same path. RFC #137 Phase B.
    #[test]
    fn per_page_scope_routes_to_actor_subdir() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let mut storage = JsonlStorage::open_with_scope_cap(
            tmp.path().to_path_buf(),
            actor,
            PageScope::PerPage("project-x".into()),
            0,
        )
        .unwrap();
        let op = mk_create(&g);
        storage.append_op(&op).unwrap();

        // File landed under `ops/<actor>/project-x.jsonl`, not
        // `ops-<actor>.jsonl`.
        let expected = tmp.path().join(format!("{actor}")).join("project-x.jsonl");
        assert!(
            expected.exists(),
            "expected per-page file at {}",
            expected.display()
        );
        let legacy = tmp.path().join(format!("ops-{actor}.jsonl"));
        assert!(
            !legacy.exists(),
            "legacy global file should not exist under PerPage scope"
        );

        // Reload reads from the per-page path.
        let reopened = JsonlStorage::open_with_scope_cap(
            tmp.path().to_path_buf(),
            actor,
            PageScope::PerPage("project-x".into()),
            0,
        )
        .unwrap();
        let ops = reopened.all_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].ts, op.ts);

        // Scope is reported back.
        assert_eq!(reopened.scope(), &PageScope::PerPage("project-x".into()));
    }

    /// PerPage storage with no ops on disk boots cleanly (fresh page).
    #[test]
    fn per_page_scope_with_missing_file_boots_clean() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let storage = JsonlStorage::open_with_scope_cap(
            tmp.path().to_path_buf(),
            actor,
            PageScope::PerPage("never-existed".into()),
            0,
        )
        .unwrap();
        assert_eq!(storage.all_ops().unwrap().len(), 0);
    }

    /// Global and PerPage storages coexist in the same `ops/` dir
    /// without clobbering each other.
    #[test]
    fn global_and_per_page_coexist() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        // Write one op under Global.
        let mut global = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        let global_op = mk_create(&g);
        global.append_op(&global_op).unwrap();
        drop(global);

        // Write one op under PerPage("home").
        let mut per_page = JsonlStorage::open_with_scope_cap(
            tmp.path().to_path_buf(),
            actor,
            PageScope::PerPage("home".into()),
            0,
        )
        .unwrap();
        let page_op = mk_create(&g);
        per_page.append_op(&page_op).unwrap();
        drop(per_page);

        // Both reload independently and see only their own ops.
        let g2 = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        let p2 = JsonlStorage::open_with_scope_cap(
            tmp.path().to_path_buf(),
            actor,
            PageScope::PerPage("home".into()),
            0,
        )
        .unwrap();
        assert_eq!(g2.all_ops().unwrap().len(), 1);
        assert_eq!(p2.all_ops().unwrap().len(), 1);
        assert_ne!(g2.all_ops().unwrap()[0].ts, p2.all_ops().unwrap()[0].ts);
    }
}
