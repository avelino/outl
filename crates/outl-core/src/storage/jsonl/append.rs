//! Write path for [`JsonlStorage`].
//!
//! The single append that lands an op on this actor's `.jsonl`, including
//! the torn-tail self-heal and the offset/node index mirroring. An
//! inherent `impl JsonlStorage` block; `Storage::append_op` in `mod.rs`
//! forwards straight to [`JsonlStorage::append_op_inner`].

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use tracing::warn;

use crate::op::LogOp;
use crate::storage::{ActorIndex, ActorNodeIndex, NodeIndex, OffsetIndex, StorageError};

use super::{op_node, JsonlStorage};

impl JsonlStorage {
    pub(super) fn append_op_inner(&mut self, op: &LogOp) -> Result<(), StorageError> {
        if op.actor != self.actor {
            return Err(StorageError::Backend(format!(
                "refused to write op from foreign actor {} (we are {})",
                op.actor, self.actor
            )));
        }

        let line = serde_json::to_string(op).map_err(|e| StorageError::Serialize(e.to_string()))?;
        let path = self.own_ops_path();
        // Open the file once (read + append): the single handle serves the
        // offset probe, the torn-tail check, and the write — instead of a
        // separate `metadata` stat plus a read-only open before the append
        // open. `O_APPEND` sends every write to EOF regardless of the read
        // cursor, so seeking back to read the last byte can't disturb where our
        // op lands. append_op is the SINGLE writer for its own actor file
        // (guarded by `ActorWriteLock`), so there is no concurrent-writer
        // TOCTOU between the tail check and the write.
        let mut file = OpenOptions::new()
            .read(true)
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
        // Byte offset where this op's line will land = current EOF. `O_APPEND`
        // leaves `stream_position()` at 0 until the first write, so seek to the
        // end explicitly to read it.
        let mut offset = file
            .seek(SeekFrom::End(0))
            .map_err(|e| StorageError::Backend(format!("seek {}: {e}", path.display())))?;
        // Torn-tail self-heal. If the file doesn't already end in a newline, a
        // previous append was cut off mid-line (crash / power loss / iOS
        // jetsam), leaving a partial record with no terminator. Appending our
        // op straight after would glue our JSON onto that fragment
        // (`{"ts":…partial{"ts":…ours}`); the reader can't split a torn prefix
        // from a good op, so BOTH would be lost. Close the torn line with a
        // newline first, so our op lands as its own parseable record.
        let needs_separator = if offset > 0 {
            file.seek(SeekFrom::End(-1))
                .map_err(|e| StorageError::Backend(format!("seek {}: {e}", path.display())))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)
                .map_err(|e| StorageError::Backend(format!("read {}: {e}", path.display())))?;
            last[0] != b'\n'
        } else {
            false
        };
        if needs_separator {
            writeln!(file)
                .map_err(|e| StorageError::Backend(format!("heal tail {}: {e}", path.display())))?;
            // Our op's JSON now starts one byte later; the offset index must
            // point at the JSON, not the healing newline.
            offset += 1;
        }
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
}
