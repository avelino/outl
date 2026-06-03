//! `creates_cycle` — the cycle-detection guard for `Op::Move`.
//!
//! Algorithm: walk up from `new_parent` following `parent(_)` until
//! we hit a sentinel (ROOT / TRASH_ROOT) or `node` itself. If we
//! reach `node`, then `node` is an ancestor of `new_parent` and the
//! move would close a loop. Bounded by `node_count + 2` steps so a
//! malformed tree (no sentinel reachable) cannot spin forever.

use super::Tree;
use crate::id::NodeId;

impl Tree {
    /// Whether moving `node` under `new_parent` would create a cycle.
    ///
    /// `node == new_parent` is always a cycle. Otherwise we walk up from
    /// `new_parent` toward the root. If we encounter `node` along the way,
    /// `node` is an ancestor of `new_parent` and the move would close a
    /// loop. The ROOT / TRASH_ROOT sentinels terminate the walk.
    pub fn creates_cycle(&self, node: NodeId, new_parent: NodeId) -> bool {
        if node == new_parent {
            return true;
        }
        let mut cursor = new_parent;
        // Bound the walk by the number of edges in the tree as a safety
        // net against malformed state. A well-formed tree terminates in
        // at most `node_count` steps.
        let mut steps = 0usize;
        let max_steps = self.nodes.len() + 2;
        loop {
            if cursor == NodeId::root() || cursor == NodeId::trash() {
                return false;
            }
            match self.parent(cursor) {
                None => return false,
                Some(p) => {
                    if p == node {
                        return true;
                    }
                    cursor = p;
                }
            }
            steps += 1;
            debug_assert!(
                steps <= max_steps,
                "creates_cycle: malformed tree (loop without sentinel)"
            );
            if steps > max_steps {
                return true;
            }
        }
    }
}
