//! Append-only op log ordered by HLC.
//!
//! The log is the source of truth for the CRDT. `Tree::apply_op` calls
//! `append`/`pop` directly when reordering; outside code should treat the
//! log as read-only and route mutations through `Workspace`.

use crate::hlc::Hlc;
use crate::op::LogOp;

/// In-memory op log, sorted by HLC.
#[derive(Debug, Default, Clone)]
pub struct OpLog {
    ops: Vec<LogOp>,
}

impl OpLog {
    /// Build an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of ops currently in the log.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Most recent op (highest HLC), if any.
    pub fn last(&self) -> Option<&LogOp> {
        self.ops.last()
    }

    /// Append an op at the tail.
    ///
    /// Callers (`Tree::apply_op` in particular) are responsible for ensuring
    /// that `op.ts > self.last().ts`; the log does not re-sort on append.
    pub fn append(&mut self, op: LogOp) {
        if let Some(last) = self.ops.last() {
            debug_assert!(
                op.ts > last.ts,
                "OpLog::append called out of order: last.ts={:?} new.ts={:?}",
                last.ts,
                op.ts
            );
        }
        self.ops.push(op);
    }

    /// Pop the most recent op. Used by `Tree::apply_op` while reordering.
    pub fn pop(&mut self) -> Option<LogOp> {
        self.ops.pop()
    }

    /// Iterate all ops in HLC order.
    pub fn iter(&self) -> impl Iterator<Item = &LogOp> {
        self.ops.iter()
    }

    /// Op at index `i` in HLC order, if any.
    ///
    /// Indices are only stable between reorders; callers that hold one
    /// across an `apply_op` must re-derive it.
    pub fn get(&self, i: usize) -> Option<&LogOp> {
        self.ops.get(i)
    }

    /// Whether the log already contains an op with this HLC.
    ///
    /// Implementation uses binary search over the HLC-sorted ops, so this
    /// is O(log n). `Tree::apply_op` calls this on every `apply` to enforce
    /// idempotency without quadratic cost.
    pub fn contains_ts(&self, ts: &Hlc) -> bool {
        self.ops.binary_search_by(|o| o.ts.cmp(ts)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{ActorId, NodeId};
    use crate::op::Op;

    fn op_at(physical: u64, logical: u32, actor: ActorId) -> LogOp {
        LogOp {
            ts: Hlc::new(physical, logical, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: crate::fractional::Fractional::first(),
            },
        }
    }

    #[test]
    fn append_and_last_track_tail() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        assert!(log.is_empty());
        log.append(op_at(1, 0, actor));
        log.append(op_at(2, 0, actor));
        assert_eq!(log.len(), 2);
        assert_eq!(log.last().unwrap().ts.physical_ms, 2);
    }

    #[test]
    fn pop_returns_most_recent() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        log.append(op_at(1, 0, actor));
        log.append(op_at(2, 0, actor));
        log.append(op_at(3, 0, actor));
        let popped = log.pop().unwrap();
        assert_eq!(popped.ts.physical_ms, 3);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn contains_ts_uses_total_order() {
        let actor = ActorId::new();
        let mut log = OpLog::new();
        let op = op_at(5, 0, actor);
        let ts = op.ts;
        log.append(op);
        assert!(log.contains_ts(&ts));
        let other = Hlc::new(5, 1, actor);
        assert!(!log.contains_ts(&other));
    }
}
