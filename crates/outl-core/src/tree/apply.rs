//! `apply_op` — the orchestrator.
//!
//! Wires `do_op` and `undo_op` into the reorder loop from
//! Kleppmann et al. 2022. New op with a smaller HLC than the log's
//! tail forces an undo/replay window so the final state matches
//! whatever order the total HLC ordering would have applied the ops
//! in.

use super::Tree;
use crate::log::OpLog;
use crate::op::LogOp;

impl Tree {
    /// Apply a new op to the log, reordering via undo/replay if necessary.
    ///
    /// Algorithm (per `docs/crdt.md` §`apply_op`):
    ///
    /// 1. If `new_op.ts` already in the log → idempotent no-op.
    /// 2. Otherwise, pop and `undo_op` all log entries whose ts > new_op.ts.
    /// 3. `do_op(new_op)`, push to log.
    /// 4. Re-apply the popped ops in original order, calling `do_op` again
    ///    on each (which re-derives `old_*` against the new state).
    pub fn apply_op(&mut self, log: &mut OpLog, new_op: LogOp) {
        if log.contains_ts(&new_op.ts) {
            return;
        }

        // Undo phase: pop everything strictly newer than new_op.
        let mut undone: Vec<LogOp> = Vec::new();
        while let Some(last) = log.last() {
            if last.ts > new_op.ts {
                let op = log.pop().expect("just peeked Some");
                self.undo_op(&op);
                undone.push(op);
            } else {
                break;
            }
        }

        // Apply the new op.
        let mut op = new_op;
        self.do_op(&mut op);
        log.append(op);

        // Replay the undone ops in their original order. `undone` is
        // newest-first (it's a stack from the pop loop); popping it gives
        // us oldest-first.
        while let Some(mut op) = undone.pop() {
            self.do_op(&mut op);
            log.append(op);
        }
    }
}
