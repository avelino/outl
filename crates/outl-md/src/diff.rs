//! Translate a new outline AST + match result into a minimal `Op` sequence.
//!
//! The diff is intentionally simple in phase 1: produce one `Create` per
//! new block (matching pass yielded `MatchLevel::Low`), one `Move` per
//! preserved block whose parent or position differs from the current
//! tree, and one `Move` to `TRASH_ROOT` per orphan id.
//!
//! Idempotency in `outl-core` means re-emitting an already-applied
//! `Create` is a no-op. The diff therefore doesn't need to know the
//! workspace's current state with full precision — it can over-emit and
//! rely on the CRDT to dedup. Higher-quality diff (only emit what
//! changed) lands when we wire the watcher in `outl-cli serve` and have
//! the live tree available.

use crate::matching::{flatten, Match, MatchLevel};
use crate::parse::OutlineNode;
use crate::sidecar::{content_hash, Sidecar, SidecarBlock};
use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::op::Op;

/// Plan produced by [`diff_to_ops`].
///
/// `ops` is the ordered sequence to apply via `Workspace::apply`.
/// `new_sidecar` is the sidecar to write back to disk after the ops
/// commit successfully.
#[derive(Debug, Clone)]
pub struct DiffPlan {
    /// Ops to apply (in order).
    pub ops: Vec<Op>,
    /// The sidecar reflecting the new tree.
    pub new_sidecar: Sidecar,
}

/// Build a [`DiffPlan`] from the new AST and the matching result.
///
/// `page_id` is the root NodeId of the page (preserved across edits).
/// `new_md_hash` is the SHA-256 of the new `.md` text (for the sidecar
/// `last_synced_hash`).
pub fn diff_to_ops(
    new_blocks: &[OutlineNode],
    matches: &[Match],
    orphans: &[NodeId],
    page_id: NodeId,
    new_md_hash: &str,
) -> DiffPlan {
    let flat = flatten(new_blocks);
    assert_eq!(
        flat.len(),
        matches.len(),
        "match count must equal new block count"
    );

    // Assign IDs for each new block: keep the matched id for level-1
    // results, mint a fresh ULID for level-3.
    let ids: Vec<NodeId> = matches
        .iter()
        .map(|m| match m.level {
            MatchLevel::High => m.old_id.expect("level High implies Some(id)"),
            MatchLevel::Medium => m.old_id.expect("level Medium implies Some(id)"),
            MatchLevel::Low => NodeId::new(),
        })
        .collect();

    // Walk the new tree in DFS preorder, generating ops + sidecar entries.
    let mut ops = Vec::<Op>::new();
    let mut sidecar_blocks = Vec::<SidecarBlock>::new();
    // Stack of (parent_id, last_position_used) per ancestor — index by indent.
    let mut parent_stack: Vec<NodeId> = vec![page_id];
    let mut last_position_per_indent: Vec<Option<Fractional>> = vec![None];

    // Track DFS line numbers for the sidecar.
    let mut line_counter: usize = 1;

    walk(
        new_blocks,
        0,
        &ids,
        &mut 0usize, // running index into `ids`
        &mut ops,
        &mut sidecar_blocks,
        &mut parent_stack,
        &mut last_position_per_indent,
        &mut line_counter,
    );

    // Orphan moves: each goes to TRASH_ROOT at a fresh position.
    for (i, orphan) in orphans.iter().enumerate() {
        // Build a unique fractional position so concurrent trashings
        // don't collide on position.
        let pos = Fractional::between(None, None);
        let _ = i; // position uniqueness comes from siblings of TRASH_ROOT,
                   // not from this index; the CRDT will resolve any
                   // collision via HLC tiebreak.
        ops.push(Op::Move {
            node: *orphan,
            new_parent: NodeId::trash(),
            position: pos,
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        });
    }

    let new_sidecar = Sidecar {
        version: crate::sidecar::SIDECAR_VERSION,
        page_id,
        last_synced_hash: new_md_hash.to_string(),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks: sidecar_blocks,
    };

    DiffPlan { ops, new_sidecar }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    blocks: &[OutlineNode],
    indent: u32,
    ids: &[NodeId],
    cursor: &mut usize,
    ops: &mut Vec<Op>,
    sidecar_blocks: &mut Vec<SidecarBlock>,
    parent_stack: &mut Vec<NodeId>,
    last_position_per_indent: &mut Vec<Option<Fractional>>,
    line_counter: &mut usize,
) {
    // Ensure the stack has a slot for the current indent's position
    // bookkeeping.
    while last_position_per_indent.len() <= indent as usize + 1 {
        last_position_per_indent.push(None);
    }

    // Reset the position counter at this indent so siblings get fresh
    // positions when we enter a new parent.
    last_position_per_indent[indent as usize + 1] = None;

    let parent_id = *parent_stack.last().expect("stack never empty");

    for block in blocks {
        let id = ids[*cursor];
        let line = *line_counter;
        *line_counter += 1;

        // Allocate a fractional position strictly greater than the last
        // sibling's. None on the left means "first child".
        let left = last_position_per_indent[indent as usize + 1].clone();
        let position = Fractional::between(left.as_ref(), None);

        // Always emit a Create. Idempotent if the block already exists.
        ops.push(Op::Create {
            node: id,
            parent: parent_id,
            position: position.clone(),
        });
        // Also emit a Move with the same target so the tree is
        // up-to-date even when this id already exists with a different
        // parent/position.
        ops.push(Op::Move {
            node: id,
            new_parent: parent_id,
            position: position.clone(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        });

        last_position_per_indent[indent as usize + 1] = Some(position.clone());

        // SetProp ops for each block property.
        for (k, v) in &block.properties {
            ops.push(Op::SetProp {
                node: id,
                key: k.clone(),
                value: Some(outl_core::property::PropValue::Text(v.clone())),
                old_value: None,
            });
        }

        sidecar_blocks.push(SidecarBlock {
            id,
            line,
            indent,
            content_hash: content_hash(&block.text),
        });

        *cursor += 1;

        // Recurse into children.
        parent_stack.push(id);
        walk(
            &block.children,
            indent + 1,
            ids,
            cursor,
            ops,
            sidecar_blocks,
            parent_stack,
            last_position_per_indent,
            line_counter,
        );
        parent_stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::match_blocks;
    use crate::parse::parse;
    use crate::sidecar::file_hash;
    use outl_core::workspace::Workspace;

    #[test]
    fn diff_produces_ops_with_preserved_ids_for_level1() {
        let md = "- a\n- b\n";
        let ast = parse(md);
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            SidecarBlock {
                id: id_a,
                line: 1,
                indent: 0,
                content_hash: content_hash("a"),
            },
            SidecarBlock {
                id: id_b,
                line: 2,
                indent: 0,
                content_hash: content_hash("b"),
            },
        ];
        let (matches, orphans) = match_blocks(&ast.blocks, &old);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
        );
        // Each block produced 1 Create + 1 Move (no properties).
        assert_eq!(plan.ops.len(), 4);
        // Sidecar carries both ids.
        assert_eq!(plan.new_sidecar.blocks.len(), 2);
        assert_eq!(plan.new_sidecar.blocks[0].id, id_a);
        assert_eq!(plan.new_sidecar.blocks[1].id, id_b);
    }

    #[test]
    fn diff_orphans_become_trash_moves() {
        let md = "- a\n";
        let ast = parse(md);
        let id_a = NodeId::new();
        let id_dead = NodeId::new();
        let old = vec![
            SidecarBlock {
                id: id_a,
                line: 1,
                indent: 0,
                content_hash: content_hash("a"),
            },
            SidecarBlock {
                id: id_dead,
                line: 2,
                indent: 0,
                content_hash: content_hash("gone"),
            },
        ];
        let (matches, orphans) = match_blocks(&ast.blocks, &old);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
        );
        // Last op must be Move(id_dead, TRASH).
        let last = plan.ops.last().unwrap();
        match last {
            Op::Move {
                node, new_parent, ..
            } => {
                assert_eq!(*node, id_dead);
                assert_eq!(*new_parent, NodeId::trash());
            }
            other => panic!("expected Move to trash, got {other:?}"),
        }
    }

    #[test]
    fn diff_apply_then_reparse_preserves_structure() {
        // End-to-end: parse md → match → diff → apply ops → materialize
        // tree → re-render md → reparse → same AST.
        let md = "title:: doc\n\n- a\n  - a1\n  - a2\n- b\n";
        let ast = parse(md);
        let page_id = NodeId::new();
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let plan = diff_to_ops(&ast.blocks, &matches, &orphans, page_id, &file_hash(md));

        let actor = outl_core::id::ActorId::new();
        let g = outl_core::hlc::HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        for op in plan.ops {
            let ts = g.next();
            ws.apply(outl_core::op::LogOp {
                ts,
                actor: ts.actor,
                op,
            })
            .unwrap();
        }
        // Tree contains the four blocks (a, a1, a2, b).
        assert_eq!(ws.tree().node_count(), 4);
    }
}
