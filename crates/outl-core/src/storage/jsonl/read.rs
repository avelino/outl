//! Read / reload path for [`JsonlStorage`].
//!
//! Everything that pulls ops back off disk lives here: the boot-time
//! `reload` (global multi-actor merge and per-page single-file), the
//! cold-read fallbacks that rehydrate an evicted op via the offset
//! index, and the per-node cold walk. All of it is an inherent
//! `impl JsonlStorage` block; the child module sees the struct's
//! private fields.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;

use tracing::{debug, warn};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::LogOp;
use crate::storage::{ActorIndex, ActorNodeIndex, NodeIndex, OffsetIndex, PageScope, StorageError};

use super::{
    op_node, parse_actor_from_ops_filename, parse_log_line, read_log_record, JsonlStorage,
    RecordRead,
};

impl JsonlStorage {
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
            let file_actor = match parse_actor_from_ops_filename(&name) {
                Some(actor) => actor,
                None => {
                    // A file-sync tool's conflict copy — `ops-<id> 2.jsonl`
                    // (iCloud) or `ops-<id>.sync-conflict-*.jsonl` (Syncthing)
                    // — matches the `ops-*.jsonl` shape but carries no valid
                    // actor id. Skip it and keep going: one stray file next to
                    // the real logs must never fail the whole workspace open.
                    warn!("skipping ops file with unparseable actor id: {name}");
                    continue;
                }
            };

            let mut lines_read = 0usize;
            let mut ops_parsed = 0usize;
            let mut rebuilt_hlc = OffsetIndex::new();
            let mut rebuilt_node = NodeIndex::new();
            let mut offset: u64 = 0;
            let mut reader = BufReader::new(file);
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let start = offset;
                let record = match read_log_record(&mut reader, &mut buf) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("io error {}:{}: {e}", path.display(), lines_read + 1);
                        break;
                    }
                };
                match record {
                    RecordRead::Eof => break,
                    RecordRead::Skip { consumed, reason } => {
                        lines_read += 1;
                        if let Some(reason) = reason {
                            warn!("skipping {}:{}: {reason}", path.display(), lines_read);
                        }
                        offset += consumed;
                    }
                    RecordRead::Ops { consumed, ops } => {
                        lines_read += 1;
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
                        offset += consumed;
                    }
                }
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
        let mut buf: Vec<u8> = Vec::new();
        let mut lines_read = 0usize;
        loop {
            let start = offset;
            let record = match read_log_record(&mut reader, &mut buf) {
                Ok(r) => r,
                Err(e) => {
                    warn!("io error {}:{}: {e}", path.display(), lines_read + 1);
                    break;
                }
            };
            match record {
                RecordRead::Eof => break,
                RecordRead::Skip { consumed, reason } => {
                    lines_read += 1;
                    if let Some(reason) = reason {
                        warn!("skipping {}:{}: {reason}", path.display(), lines_read);
                    }
                    offset += consumed;
                }
                RecordRead::Ops { consumed, ops } => {
                    lines_read += 1;
                    for op in &ops {
                        rebuilt_hlc.insert(op.ts, start);
                        if let Some(node) = op_node(&op.op) {
                            rebuilt_node.insert(node, op.ts, start);
                        }
                    }
                    all.extend(ops);
                    offset += consumed;
                }
            }
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

    /// Cold-path `ops_for_node` when the LRU has no warm entry for the
    /// node. Walks the per-node secondary index across every known
    /// actor and pulls each op from the cache (if still resident) or
    /// the disk file via [`Self::read_op_at`]. RFC #137 Phase A.
    pub(super) fn cold_ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
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
