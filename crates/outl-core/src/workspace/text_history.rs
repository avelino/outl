//! Reconstructing a block's *past* text from the op log.
//!
//! [`Workspace::block_text`](super::Workspace::block_text) answers "what
//! does this block say now". This module answers "what did it say before",
//! and it exists because the two questions have very different failure
//! costs: an `Op::Edit` that replaced a block's text with a shorter string
//! looks, from the materialized tree, exactly like the block never held
//! more.
//!
//! It did. `Op::Edit` carries a Yrs `update_v1` delta, not a snapshot, and
//! the log is append-only — so the pre-edit text is still reconstructible
//! by replaying the block's `Edit`s up to (but not past) the shrink. That
//! is the only recovery route for content whose `.md` was overwritten
//! before anyone noticed, which `outl reconcile --ahead-of-log` (a
//! `.md → tree` path) cannot reach.
//!
//! See `outl_actions::recover` for the caller, and
//! [RFC 0210](../../../../docs/rfcs/0210-md-content-outside-op-log.md).

use super::{Workspace, WorkspaceError};
use crate::id::NodeId;
use crate::op::Op;

impl Workspace {
    /// Every intermediate state of `node`'s text, oldest first — one entry
    /// per `Op::Edit` in the block's history, the last entry being the
    /// block's current text. Empty when the block was never edited.
    ///
    /// Reads the node's ops from **storage**, never from the resident log
    /// or the text cache. Both are boot-mode dependent (a snapshot boot
    /// leaves the resident log holding only the post-cutoff delta), so a
    /// history read from either could silently *shorten* — and a shortened
    /// history is indistinguishable from "nothing was lost here", which is
    /// the exact wrong answer for a recovery caller to get. The op log is
    /// the source of truth; this reads it.
    ///
    /// Cost is one index-driven read set per node — O(edits-of-node), not
    /// O(log).
    pub fn block_text_history(&self, node: NodeId) -> Result<Vec<String>, WorkspaceError> {
        let ops = self.ops_for_node_combined(node)?;
        Ok(crate::content::text_revisions(ops.iter().filter_map(
            |logged| match &logged.op {
                Op::Edit { text_op, .. } => Some(text_op.as_slice()),
                _ => None,
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::fractional::Fractional;
    use crate::hlc::HlcGenerator;
    use crate::id::{ActorId, NodeId};
    use crate::op::{LogOp, Op};
    use crate::workspace::Workspace;

    fn logged(hlc: &HlcGenerator, op: Op) -> LogOp {
        let ts = hlc.next();
        LogOp {
            ts,
            actor: ts.actor,
            op,
        }
    }

    /// Build a workspace holding one block that went through `texts` in
    /// order, returning it plus the block id.
    fn block_with_edits(texts: &[&str]) -> (Workspace, NodeId) {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
        let node = NodeId::new();
        ws.apply(logged(
            &hlc,
            Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .expect("create");
        for text in texts {
            let update = ws.build_text_replace_update(node, text);
            ws.apply(logged(
                &hlc,
                Op::Edit {
                    node,
                    text_op: update,
                },
            ))
            .expect("edit");
        }
        (ws, node)
    }

    #[test]
    fn a_never_edited_block_has_no_history() {
        let (ws, node) = block_with_edits(&[]);
        assert!(ws.block_text_history(node).expect("history").is_empty());
    }

    #[test]
    fn history_holds_one_entry_per_edit_in_order() {
        let (ws, node) = block_with_edits(&["one", "one two", "one two three"]);
        assert_eq!(
            ws.block_text_history(node).expect("history"),
            vec!["one", "one two", "one two three"]
        );
    }

    #[test]
    fn the_last_entry_is_the_current_text() {
        let (ws, node) = block_with_edits(&["draft", "final"]);
        let history = ws.block_text_history(node).expect("history");
        assert_eq!(
            history.last().map(String::as_str),
            ws.block_text(node).as_deref()
        );
    }

    /// The whole point: an edit that shrank the block did not erase what
    /// it replaced. The long text is gone from the tree and still in the
    /// log.
    #[test]
    fn a_truncating_edit_leaves_the_earlier_text_recoverable() {
        let long = "title\nbody line one\nbody line two";
        let (ws, node) = block_with_edits(&[long, "title"]);
        assert_eq!(ws.block_text(node).as_deref(), Some("title"));
        assert_eq!(
            ws.block_text_history(node).expect("history")[0].as_str(),
            long
        );
    }

    /// A block never touched by an `Edit` reads back empty rather than
    /// erroring, so a caller can scan every node in a tree unconditionally.
    #[test]
    fn an_unknown_node_has_no_history() {
        let (ws, _) = block_with_edits(&["x"]);
        assert!(ws
            .block_text_history(NodeId::new())
            .expect("history")
            .is_empty());
    }
}
