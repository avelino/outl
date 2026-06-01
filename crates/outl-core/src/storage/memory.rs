//! In-memory storage backend.
//!
//! Pure `Vec<LogOp>` with no filesystem footprint. Used by tests and
//! by [`crate::workspace::Workspace::open_in_memory`] when callers
//! want a workspace that never touches disk.
//!
//! Not a sync backend — there's no per-actor file, no merging across
//! peers. If two devices share the same workspace they need
//! [`JsonlStorage`](crate::storage::JsonlStorage); this exists purely
//! to keep tests fast and to give the public `open_in_memory`
//! constructor a destination.

use std::collections::HashMap;

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{Snapshot, Storage, StorageError};

/// In-memory op log + snapshot slot.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    ops: Vec<LogOp>,
    snapshot: Option<Snapshot>,
}

impl MemoryStorage {
    /// Build an empty in-memory storage.
    pub fn new() -> Self {
        Self::default()
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

impl Storage for MemoryStorage {
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
        self.ops.push(op.clone());
        // Keep total order so `all_ops` / `ops_since` don't depend on
        // insertion order. Cost is tolerable: tests use small logs.
        self.ops.sort_by_key(|o| o.ts);
        Ok(())
    }

    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
        Ok(self.ops.iter().filter(|o| o.ts > ts).cloned().collect())
    }

    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        Ok(self
            .ops
            .iter()
            .filter(|o| op_touches_node(&o.op, id))
            .cloned()
            .collect())
    }

    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
        Ok(self.ops.iter().filter(|o| o.actor == id).cloned().collect())
    }

    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        let mut out: HashMap<ActorId, Hlc> = HashMap::new();
        for op in &self.ops {
            out.entry(op.actor)
                .and_modify(|cur| {
                    if op.ts > *cur {
                        *cur = op.ts;
                    }
                })
                .or_insert(op.ts);
        }
        Ok(out)
    }

    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
        Ok(self.ops.clone())
    }

    fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        self.snapshot = Some(snapshot.clone());
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError> {
        Ok(self.snapshot.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;

    fn make_op(g: &HlcGenerator) -> LogOp {
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
    fn round_trip_preserves_ops() {
        let mut s = MemoryStorage::new();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);

        let a = make_op(&g);
        let b = make_op(&g);
        s.append_op(&a).unwrap();
        s.append_op(&b).unwrap();

        let all = s.all_ops().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].ts, a.ts);
        assert_eq!(all[1].ts, b.ts);
    }

    #[test]
    fn ops_since_filters_strictly() {
        let mut s = MemoryStorage::new();
        let g = HlcGenerator::new(ActorId::new());
        let a = make_op(&g);
        let b = make_op(&g);
        s.append_op(&a).unwrap();
        s.append_op(&b).unwrap();
        let after_a = s.ops_since(a.ts).unwrap();
        assert_eq!(after_a.len(), 1);
        assert_eq!(after_a[0].ts, b.ts);
    }

    #[test]
    fn snapshot_round_trip() {
        let mut s = MemoryStorage::new();
        assert!(s.load_snapshot().unwrap().is_none());
        s.save_snapshot(&Snapshot {
            bytes: vec![1, 2, 3],
        })
        .unwrap();
        assert_eq!(s.load_snapshot().unwrap().unwrap().bytes, vec![1, 2, 3]);
    }
}
