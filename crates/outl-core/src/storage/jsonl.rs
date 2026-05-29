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
//! ├── ops-<peer_actor>.jsonl    ← read-only mirrors of other devices
//! └── ...
//! ```
//!
//! The directory itself is the unit of sync; callers pick the parent
//! (e.g. an iCloud Ubiquity Container, a shared folder) and pass the
//! `.ops/` subpath in. The struct never reaches out to figure out
//! where it lives — it stays a plain filesystem backend.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use parking_lot::RwLock;
use tracing::{info, warn};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{Snapshot, Storage, StorageError};

/// One-file-per-actor JSONL op log on the filesystem.
pub struct JsonlStorage {
    /// Directory containing every per-actor ops file.
    ops_dir: PathBuf,
    /// This device's actor id; we never write into another actor's file.
    actor: ActorId,
    /// In-memory mirror of the merged op log, sorted by HLC.
    cache: RwLock<Vec<LogOp>>,
}

impl JsonlStorage {
    /// Open the storage rooted at `ops_dir` for the given `actor`. The
    /// directory is created if missing. The merged op log is loaded into
    /// memory on open.
    pub fn open(ops_dir: PathBuf, actor: ActorId) -> Result<Self, StorageError> {
        std::fs::create_dir_all(&ops_dir)
            .map_err(|e| StorageError::Backend(format!("create ops dir: {e}")))?;

        let mut storage = Self {
            ops_dir,
            actor,
            cache: RwLock::new(Vec::new()),
        };
        storage.reload()?;
        Ok(storage)
    }

    fn own_ops_path(&self) -> PathBuf {
        self.ops_dir.join(format!("ops-{}.jsonl", self.actor))
    }

    /// Re-read every `ops-*.jsonl` from disk into the cache.
    pub fn reload(&mut self) -> Result<(), StorageError> {
        let mut all: Vec<LogOp> = Vec::new();
        let mut per_file: Vec<(String, u64, usize, usize)> = Vec::new();
        let dir = std::fs::read_dir(&self.ops_dir)
            .map_err(|e| StorageError::Backend(format!("read {}: {e}", self.ops_dir.display())))?;

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
            let mut lines_read = 0usize;
            let mut ops_parsed = 0usize;
            for (lineno, line) in BufReader::new(file).lines().enumerate() {
                lines_read += 1;
                let raw = match line {
                    Ok(l) if !l.is_empty() => l,
                    Ok(_) => continue,
                    Err(e) => {
                        warn!("io error {}:{}: {e}", path.display(), lineno + 1);
                        continue;
                    }
                };
                match serde_json::from_str::<LogOp>(&raw) {
                    Ok(op) => {
                        all.push(op);
                        ops_parsed += 1;
                    }
                    Err(e) => warn!("parse {}:{}: {e}", path.display(), lineno + 1),
                }
            }
            info!(
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
        info!(
            "jsonl storage loaded {} ops from {} ({} files); per_actor={:?}",
            all.len(),
            self.ops_dir.display(),
            per_file.len(),
            per_actor
        );
        *self.cache.write() = all;
        Ok(())
    }

    /// Per-file diagnostics from the most recent `reload`. Useful for
    /// embedding inside debug snapshots without rerunning the parse.
    pub fn file_stats(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for op in self.cache.read().iter() {
            *counts.entry(format!("ops-{}.jsonl", op.actor)).or_insert(0) += 1;
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Directory the storage reads/writes from. Lets clients log it.
    pub fn ops_dir(&self) -> &std::path::Path {
        &self.ops_dir
    }
}

fn op_touches_node(op: &Op, id: NodeId) -> bool {
    match op {
        Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::Create { node, .. } => *node == id,
    }
}

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
        writeln!(file, "{line}")
            .map_err(|e| StorageError::Backend(format!("write {}: {e}", path.display())))?;
        file.sync_all()
            .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", path.display())))?;

        self.cache.write().push(op.clone());
        Ok(())
    }

    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
        Ok(self
            .cache
            .read()
            .iter()
            .filter(|o| o.ts > ts)
            .cloned()
            .collect())
    }

    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        Ok(self
            .cache
            .read()
            .iter()
            .filter(|o| op_touches_node(&o.op, id))
            .cloned()
            .collect())
    }

    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
        Ok(self
            .cache
            .read()
            .iter()
            .filter(|o| o.actor == id)
            .cloned()
            .collect())
    }

    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        let mut map: HashMap<ActorId, Hlc> = HashMap::new();
        for op in self.cache.read().iter() {
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
        Ok(self.cache.read().clone())
    }

    fn save_snapshot(&mut self, _snapshot: &Snapshot) -> Result<(), StorageError> {
        // Snapshots are not used yet; keeping the file out of the synced
        // directory avoids churning peers until we have a real format.
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::op::Op;
    use tempfile::TempDir;

    #[test]
    fn roundtrips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        assert_eq!(storage.all_ops().unwrap().len(), 0);

        let ts = g.next();
        let op = LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        };
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
        let ts = g.next();
        let op = LogOp {
            ts,
            actor: them,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        };
        assert!(storage.append_op(&op).is_err());
    }

    #[test]
    fn merges_ops_from_multiple_actor_files() {
        let tmp = TempDir::new().unwrap();
        let me = ActorId::new();
        let peer = ActorId::new();

        // Peer writes its own file first.
        {
            let mut peer_storage = JsonlStorage::open(tmp.path().to_path_buf(), peer).unwrap();
            let g = HlcGenerator::new(peer);
            let ts = g.next();
            let op = LogOp {
                ts,
                actor: peer,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            };
            peer_storage.append_op(&op).unwrap();
        }

        // I open the same dir as a different actor: I see the peer's op.
        let mine = JsonlStorage::open(tmp.path().to_path_buf(), me).unwrap();
        assert_eq!(mine.all_ops().unwrap().len(), 1);
    }
}
