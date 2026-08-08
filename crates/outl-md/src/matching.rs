//! The block-matching algorithm that reconstructs IDs after an external
//! edit to a `.md` file.
//!
//! The levels, in the order they run:
//!
//! - **Level 1** — `content_hash` exact match. Preserves the ID.
//! - **Level 1.5 (positional fallback)** — same DFS index, same indent,
//!   **same parent**. Preserves the ID when a block's text is edited
//!   without structural changes. Only runs when the flat block counts
//!   agree, since any insert/delete shifts every following DFS index.
//! - **Level 2 (similarity)** — normalized Levenshtein over the text the
//!   sidecar recorded at last sync (`SidecarBlock::text`) versus
//!   the freshly parsed text, gated on a similarity threshold of 80%
//!   (`SIMILARITY_THRESHOLD`) and — **unconditionally** — on the two
//!   DFS indices sitting within `SIMILARITY_POSITION_WINDOW` positions
//!   of each other. Whether the parents agree gates nothing; it only
//!   selects which warning is logged. Preserves the ID and logs that
//!   warning.
//!   This is what survives the common external edit: one save
//!   that reworded a block **and** added or removed another, which
//!   makes the counts disagree and takes level 1.5 out of play.
//!   Candidates are assigned by **global confidence** — every pair in
//!   the window is scored first and the pairs resolve highest score
//!   down, with a two-sided runner-up margin — never in the order the
//!   new blocks appear. See the `similarity` module for why.
//! - **Level 3** — no match. New ULID assigned for new blocks; old blocks
//!   without a match become orphans (caller moves them to `TRASH_ROOT`).
//!
//! Level 3 says nothing about *how much* it is deleting: one orphan and
//! five thousand come back in the same vector. Any caller that turns
//! orphans into `Move(node, TRASH_ROOT)` should go through
//! [`guard::match_blocks_guarded`] instead of [`match_blocks`], which
//! asks that second question and refuses the whole pass — rather than
//! half of it — when the answer is "the page".
//!
//! A sidecar entry with no recorded text — written before the field
//! existed, or by a peer binary that doesn't know it — simply doesn't
//! fire level 2: the result is exactly the pre-level-2 behaviour, never
//! worse. The next write by a binary that knows the field records it.
//! The gate is the **empty string, not the sidecar version number**;
//! see `SIDECAR_VERSION` for why the two are deliberately decoupled.

pub mod guard;

use crate::parse::OutlineNode;
use crate::sidecar::{content_hash, SidecarBlock};
use crate::similarity;
use outl_core::id::NodeId;
use std::collections::HashSet;

/// Confidence level of a matched pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchLevel {
    /// `content_hash` exact match, or the structural (level 1.5)
    /// positional fallback.
    High,
    /// Similarity match (level 2): the text changed but stayed above
    /// the 80% similarity threshold. Emitted with a `tracing` warning.
    Medium,
    /// No old block matched the new one — assign a fresh id.
    Low,
}

/// A new block paired with an optional preserved id.
#[derive(Clone, Debug)]
pub struct Match {
    /// Index of the new block within the flattened (DFS preorder) walk
    /// of the parsed AST.
    pub new_block_index: usize,
    /// Old id, if a match was found. `None` for level-3 matches; the
    /// caller assigns a fresh ULID for these.
    pub old_id: Option<NodeId>,
    /// Confidence level of this match.
    pub level: MatchLevel,
}

/// Flattened view of a single block: text plus the indent at which it
/// appeared in the source `.md`.
#[derive(Clone, Debug)]
pub struct FlatBlock<'a> {
    /// Block content.
    pub text: &'a str,
    /// Depth (root-level = 0).
    pub indent: u32,
    /// DFS index of this block's parent, or `None` for a direct child
    /// of the page root.
    ///
    /// Indent alone is *not* a parent: two blocks can sit at the same
    /// depth under different parents. Matching that treats them as
    /// interchangeable hands one subtree's id to another subtree's
    /// block.
    pub parent: Option<usize>,
}

/// Flatten an outline tree into a depth-first preorder list. Used by
/// `match_blocks` and `diff_to_ops` so both see the same indexing.
pub fn flatten(blocks: &[OutlineNode]) -> Vec<FlatBlock<'_>> {
    let mut out = Vec::new();
    push_flat(blocks, 0, None, &mut out);
    out
}

fn push_flat<'a>(
    blocks: &'a [OutlineNode],
    indent: u32,
    parent: Option<usize>,
    out: &mut Vec<FlatBlock<'a>>,
) {
    for b in blocks {
        let idx = out.len();
        out.push(FlatBlock {
            text: &b.text,
            indent,
            parent,
        });
        push_flat(&b.children, indent + 1, Some(idx), out);
    }
}

/// Reconstruct each sidecar entry's parent (as an index into the same
/// slice) from the recorded indents.
///
/// The sidecar stores blocks in DFS preorder with an explicit indent,
/// which is enough to rebuild the tree: a block's parent is the most
/// recent entry one level shallower. A malformed sidecar that skips an
/// indent level degrades to "closest known ancestor" rather than
/// shifting every following parent.
fn sidecar_parents(old_blocks: &[SidecarBlock]) -> Vec<Option<usize>> {
    let mut parents = Vec::with_capacity(old_blocks.len());
    // `by_depth[d]` = index of the most recent entry seen at depth `d`.
    let mut by_depth: Vec<usize> = Vec::new();
    for (i, b) in old_blocks.iter().enumerate() {
        let depth = b.indent as usize;
        by_depth.truncate(depth);
        parents.push(by_depth.last().copied());
        while by_depth.len() < depth {
            by_depth.push(i);
        }
        by_depth.push(i);
    }
    parents
}

/// Do the new block's parent and the old block's parent refer to the
/// same node?
///
/// Both sides are DFS indices into their own list, so they're compared
/// through the ids matching has already resolved: the new parent's slot
/// must hold exactly the old parent's id. Two direct children of the
/// page root also agree. An unresolved new parent (level 3, or not yet
/// visited) counts as disagreement — the conservative answer.
fn parents_agree(
    new_parent: Option<usize>,
    old_parent: Option<usize>,
    resolved: &[Option<NodeId>],
    old_blocks: &[SidecarBlock],
) -> bool {
    match (new_parent, old_parent) {
        (None, None) => true,
        (Some(p), Some(q)) => match (resolved.get(p), old_blocks.get(q)) {
            (Some(Some(new_parent_id)), Some(old)) => *new_parent_id == old.id,
            _ => false,
        },
        _ => false,
    }
}

/// Run the matching algorithm against new outline and old sidecar entries.
///
/// Returns:
///
/// - One [`Match`] per new block, in DFS preorder of the new AST.
/// - The list of orphan old IDs (no new counterpart). Caller MUST log
///   each orphan to `.outl/orphans.log` before emitting any deletion op.
pub fn match_blocks(
    new_blocks: &[OutlineNode],
    old_blocks: &[SidecarBlock],
) -> (Vec<Match>, Vec<NodeId>) {
    let flat = flatten(new_blocks);
    let mut matches: Vec<Match> = Vec::with_capacity(flat.len());
    let mut used: HashSet<NodeId> = HashSet::new();

    // Pre-compute hashes for the new blocks.
    let new_hashes: Vec<String> = flat.iter().map(|b| content_hash(b.text)).collect();

    // Pass 1: level 1 matches (hash exact). Greedy first-fit.
    let mut found: Vec<Option<NodeId>> = vec![None; flat.len()];
    for (i, h) in new_hashes.iter().enumerate() {
        for old in old_blocks {
            if used.contains(&old.id) {
                continue;
            }
            if old.content_hash == *h {
                found[i] = Some(old.id);
                used.insert(old.id);
                break;
            }
        }
    }

    let old_parents = sidecar_parents(old_blocks);

    // Pass 1.5: positional fallback. For each unmatched new block at
    // index `i`, if `old_blocks[i]` exists, is unused, sits at the same
    // indent AND under the same parent, match them. This preserves the
    // NodeId when a block's text is edited without structural changes
    // (no blocks added, removed, or moved). Without this, every text
    // edit would mint a fresh NodeId + ref_handle, breaking all
    // `((blk-…))` refs and `!((blk-…))` embeds pointing at the edited
    // block.
    //
    // The parent check is not redundant with the indent check: equal
    // indent only means "same depth", and a reparent can leave the
    // depth untouched while moving the block into a different subtree.
    // Matching on indent alone handed the old id to a block that now
    // lives somewhere else entirely. A rejection here is not fatal —
    // level 2 below can still recover the id on similarity, and it
    // warns when it does so across parents.
    //
    // Only runs when the flat block count matches — if blocks were
    // inserted or deleted, DFS indices shift and positional matching
    // would assign the wrong NodeId. That case is level 2's job.
    //
    // Parents are always resolved before their children here: DFS
    // preorder puts a parent at a lower index than its descendants, and
    // this loop walks indices in ascending order.
    if flat.len() == old_blocks.len() {
        for i in 0..flat.len() {
            if found[i].is_some() {
                continue;
            }
            let old = &old_blocks[i];
            if used.contains(&old.id) || old.indent != flat[i].indent {
                continue;
            }
            if !parents_agree(flat[i].parent, old_parents[i], &found, old_blocks) {
                continue;
            }
            found[i] = Some(old.id);
            used.insert(old.id);
        }
    }

    // Pass 2: level 2 — similarity over the text the sidecar recorded
    // at last sync. This is what carries a block's id (and its
    // `((blk-…))` handle) through the common external edit: one save in
    // vim / Obsidian that both rewords a block and adds or removes
    // another. The count mismatch takes level 1.5 out of play, so
    // without this pass every reworded block minted a fresh ULID and
    // the old one went to the trash with its references dangling.
    //
    // Assignment is by **global confidence**, not by the order the new
    // blocks appear: every in-window pair is scored, then resolved from
    // the highest score down with a two-sided runner-up margin. Walking
    // `0..flat.len()` and taking the first candidate above the threshold
    // let a newly typed block steal the id of the block it merely
    // resembles, purely by sitting at a lower index — and because the
    // old id was consumed, `orphans` came back empty and nothing was
    // logged. See the `similarity` module.
    let new_texts: Vec<&str> = flat.iter().map(|b| b.text).collect();
    let candidates = similarity::collect_candidates(&new_texts, old_blocks, &found, &used);
    let accepted =
        similarity::resolve_candidates(candidates, old_blocks, flat.len(), &mut found, &mut used);

    let mut medium: HashSet<usize> = HashSet::new();
    for (i, j, score) in accepted {
        medium.insert(i);
        let old = &old_blocks[j];
        // Evaluated after every level-2 winner landed, so a parent
        // resolved by a later assignment still counts. It picks the
        // warning, nothing else — the window above is the only gate.
        if parents_agree(flat[i].parent, old_parents[j], &found, old_blocks) {
            tracing::warn!(
                block = %old.id,
                similarity = score,
                "level-2 match: block text changed beyond an exact hash; id preserved"
            );
        } else {
            // The crate's `CLAUDE.md` calls this out explicitly:
            // matching on similarity across different parents without
            // a warning is a "never do this". The match is still the
            // right call (the user moved a block and edited it in the
            // same save), but it is the one that most deserves a trace
            // when a reference later looks misplaced.
            tracing::warn!(
                block = %old.id,
                similarity = score,
                "level-2 match ACROSS PARENTS: block was reparented and reworded in the same save; id preserved"
            );
        }
    }

    // Final pass: level 3 for the remainder.
    for (i, maybe_id) in found.iter().enumerate() {
        matches.push(Match {
            new_block_index: i,
            old_id: *maybe_id,
            level: match maybe_id {
                Some(_) if medium.contains(&i) => MatchLevel::Medium,
                Some(_) => MatchLevel::High,
                None => MatchLevel::Low,
            },
        });
    }

    let orphans: Vec<NodeId> = old_blocks
        .iter()
        .filter(|o| !used.contains(&o.id))
        .map(|o| o.id)
        .collect();

    (matches, orphans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn sidecar_block(id: NodeId, text: &str, line: usize, indent: u32) -> SidecarBlock {
        SidecarBlock::from_text(id, line, indent, text)
    }

    /// A sidecar entry with no `text` recorded — written before the
    /// field existed, or by a peer binary that doesn't know it. The
    /// input level 2 cannot work with.
    fn legacy_sidecar_block(id: NodeId, text: &str, line: usize, indent: u32) -> SidecarBlock {
        SidecarBlock {
            text: String::new(),
            ..SidecarBlock::from_text(id, line, indent, text)
        }
    }

    #[test]
    fn identical_md_yields_only_level1_matches() {
        let md = "- a\n- b\n- c\n";
        let new_ast = parse(md);
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let id_c = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "a", 1, 0),
            sidecar_block(id_b, "b", 2, 0),
            sidecar_block(id_c, "c", 3, 0),
        ];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert!(orphans.is_empty());
        assert_eq!(matches.len(), 3);
        for m in &matches {
            assert_eq!(m.level, MatchLevel::High);
            assert!(m.old_id.is_some());
        }
        assert_eq!(matches[0].old_id, Some(id_a));
        assert_eq!(matches[1].old_id, Some(id_b));
        assert_eq!(matches[2].old_id, Some(id_c));
    }

    #[test]
    fn new_blocks_get_level3_and_no_orphans() {
        let md = "- a\n- new!\n- b\n";
        let new_ast = parse(md);
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "a", 1, 0),
            sidecar_block(id_b, "b", 2, 0),
        ];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].old_id, Some(id_a));
        assert_eq!(matches[0].level, MatchLevel::High);
        assert_eq!(matches[1].old_id, None);
        assert_eq!(matches[1].level, MatchLevel::Low);
        assert_eq!(matches[2].old_id, Some(id_b));
        assert!(orphans.is_empty());
    }

    #[test]
    fn deleted_blocks_become_orphans() {
        let md = "- a\n";
        let new_ast = parse(md);
        let id_a = NodeId::new();
        let id_gone = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "a", 1, 0),
            sidecar_block(id_gone, "gone", 2, 0),
        ];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].old_id, Some(id_a));
        assert_eq!(orphans, vec![id_gone]);
    }

    #[test]
    fn duplicated_block_first_keeps_id_second_gets_new() {
        // User Ctrl+D'd the block in vscode → two identical lines.
        let md = "- hello\n- hello\n";
        let new_ast = parse(md);
        let id = NodeId::new();
        let old = vec![sidecar_block(id, "hello", 1, 0)];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 2);
        // First match consumes the old id.
        assert_eq!(matches[0].old_id, Some(id));
        assert_eq!(matches[0].level, MatchLevel::High);
        // Second falls through to level 3.
        assert_eq!(matches[1].old_id, None);
        assert_eq!(matches[1].level, MatchLevel::Low);
        assert!(orphans.is_empty());
    }

    #[test]
    fn text_edit_preserves_id_via_positional_fallback() {
        // Block text changed but position (DFS index) and indent are
        // the same → positional fallback preserves the NodeId.
        // Without this, every text edit would mint a fresh NodeId,
        // breaking all ((blk-…)) refs and !((blk-…)) embeds.
        let md = "- TODO buy groceries\n";
        let new_ast = parse(md);
        let id = NodeId::new();
        let old = vec![sidecar_block(id, "TODO buy milk", 1, 0)];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].old_id, Some(id));
        assert!(orphans.is_empty());
    }

    #[test]
    fn insert_shift_prevents_false_positional_match() {
        // Inserting a block before an existing one shifts DFS indices,
        // so positional fallback can't match the shifted block. The
        // inserted block gets a new ID (level 3); the existing block
        // still matches by hash (level 1).
        let md = "- new block\n- original\n";
        let new_ast = parse(md);
        let id = NodeId::new();
        let old = vec![sidecar_block(id, "original", 1, 0)];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 2);
        // new[0] = "new block" — no hash match, positional fallback tries
        // old[0] = "original" → same indent, but "original" will be hash-
        // matched to new[1] first, so it's already `used`. Thus new[0]
        // falls through to level 3.
        // Actually wait — hash matching happens in pass 1 before
        // positional fallback. So "original" at new[1] hash-matches
        // old[0]. Then positional fallback for new[0] finds old[0]
        // already used → no match → level 3. Correct!
        assert_eq!(matches[0].old_id, None);
        assert_eq!(matches[0].level, MatchLevel::Low);
        assert_eq!(matches[1].old_id, Some(id));
        assert_eq!(matches[1].level, MatchLevel::High);
        assert!(orphans.is_empty());
    }

    // ---------------------------------------------------------------
    // Level 2 — similarity.
    // ---------------------------------------------------------------

    #[test]
    fn edit_plus_delete_in_one_save_keeps_the_edited_block_id() {
        // THE common external edit: the user opens the `.md` in vim,
        // rewords one block and deletes another, saves once. The counts
        // disagree, so the positional fallback is out of play — before
        // level 2 existed, the reworded block minted a fresh ULID and
        // the old one went to the trash, breaking every `((blk-…))`
        // pointing at it.
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let id_c = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "buy groceries at the market", 1, 0),
            sidecar_block(id_b, "call the plumber back", 2, 0),
            sidecar_block(id_c, "ship the release notes", 3, 0),
        ];

        // "a" reworded, "b" deleted, "c" untouched.
        let new_ast = parse("- buy groceries at the market today\n- ship the release notes\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches.len(), 2);
        assert_eq!(
            matches[0].old_id,
            Some(id_a),
            "the reworded block must keep its id"
        );
        assert_eq!(matches[0].level, MatchLevel::Medium);
        assert_eq!(matches[1].old_id, Some(id_c));
        assert_eq!(matches[1].level, MatchLevel::High);
        assert_eq!(orphans, vec![id_b], "only the deleted block orphans");
    }

    #[test]
    fn edit_plus_insert_in_one_save_keeps_the_edited_block_id() {
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "review the storage RFC", 1, 0),
            sidecar_block(id_b, "answer Samara", 2, 0),
        ];

        let new_ast =
            parse("- brand new unrelated item\n- review the storage RFCs\n- answer Samara\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].old_id, None, "the inserted block is level 3");
        assert_eq!(
            matches[1].old_id,
            Some(id_a),
            "the reworded block keeps its id despite the index shift"
        );
        assert_eq!(matches[1].level, MatchLevel::Medium);
        assert_eq!(matches[2].old_id, Some(id_b));
        assert!(orphans.is_empty(), "nothing was deleted");
    }

    #[test]
    fn similarity_below_threshold_falls_to_level3_and_orphans() {
        // Replacing a block's content outright is a delete + an insert,
        // not an edit. The old id must surface as an orphan so the
        // caller records it in `orphans.log` before trashing it —
        // silent deletion is the P0 this crate exists to prevent.
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "buy groceries at the market", 1, 0),
            sidecar_block(id_b, "call the plumber back", 2, 0),
        ];

        let new_ast = parse("- completely unrelated sentence about compilers\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].old_id, None);
        assert_eq!(matches[0].level, MatchLevel::Low);
        assert_eq!(orphans, vec![id_a, id_b], "both old blocks orphan");
    }

    #[test]
    fn level2_matches_across_parents_when_position_is_close() {
        // A block reparented AND reworded in the same save. Level 2
        // still recovers the id (the alternative is a broken ref), and
        // `match_blocks` logs the cross-parent warning.
        let id_a = NodeId::new();
        let id_child = NodeId::new();
        let old = vec![
            sidecar_block(id_a, "sprint planning", 1, 0),
            sidecar_block(id_child, "draft the migration plan", 2, 1),
        ];

        // The child was outdented to top level and reworded.
        let new_ast = parse("- sprint planning\n- draft the migration plans\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches[1].old_id, Some(id_child));
        assert_eq!(matches[1].level, MatchLevel::Medium);
        assert!(orphans.is_empty());
    }

    #[test]
    fn legacy_sidecar_without_text_degrades_to_pre_level2_behaviour() {
        // A sidecar with no recorded text can't fire level 2 at all.
        // The result must be exactly what shipped before — level 3 plus
        // an orphan — never a panic and never a wrong match.
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            legacy_sidecar_block(id_a, "buy groceries at the market", 1, 0),
            legacy_sidecar_block(id_b, "call the plumber back", 2, 0),
        ];

        let new_ast = parse("- buy groceries at the market today\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches[0].old_id, None);
        assert_eq!(matches[0].level, MatchLevel::Low);
        assert_eq!(orphans, vec![id_a, id_b]);
    }

    // ---------------------------------------------------------------
    // Level 1.5 — the parent gate.
    // ---------------------------------------------------------------

    #[test]
    fn positional_fallback_requires_the_same_parent_not_just_the_same_indent() {
        // Equal block counts, and index 2 sits at indent 1 on both
        // sides — but under a different parent. `old[2]` was a child of
        // `alpha`; the new block at index 2 is a child of `beta`.
        // Matching on indent alone teleported alpha's child id (and its
        // `((blk-…))` handle) into beta's subtree.
        let id_alpha = NodeId::new();
        let id_beta = NodeId::new();
        let id_gamma = NodeId::new();
        let old = vec![
            sidecar_block(id_alpha, "alpha", 1, 0),
            sidecar_block(id_beta, "beta", 2, 1),
            sidecar_block(id_gamma, "gamma", 3, 1),
        ];

        // `beta` outdented to top level; a brand-new block takes the
        // index-2 slot under it.
        let new_ast = parse("- alpha\n- beta\n  - zeta\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches[0].old_id, Some(id_alpha));
        assert_eq!(matches[1].old_id, Some(id_beta), "matched by hash");
        assert_eq!(
            matches[2].old_id, None,
            "a different parent must not inherit gamma's id"
        );
        assert_eq!(matches[2].level, MatchLevel::Low);
        assert_eq!(orphans, vec![id_gamma]);
    }

    #[test]
    fn positional_fallback_still_matches_under_the_same_parent() {
        // The companion of the test above: same shape, same parent —
        // the edited child keeps its id, exactly as before the parent
        // gate landed.
        let id_alpha = NodeId::new();
        let id_child = NodeId::new();
        let old = vec![
            sidecar_block(id_alpha, "alpha", 1, 0),
            sidecar_block(id_child, "child", 2, 1),
        ];

        let new_ast = parse("- alpha\n  - child reworded entirely differently\n");
        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

        assert_eq!(matches[1].old_id, Some(id_child));
        assert_eq!(matches[1].level, MatchLevel::High);
        assert!(orphans.is_empty());
    }

    #[test]
    fn sidecar_parents_rebuilds_the_tree_from_indents() {
        let ids: Vec<NodeId> = (0..5).map(|_| NodeId::new()).collect();
        // a / a1 / a11 / a2 / b
        let old = vec![
            sidecar_block(ids[0], "a", 1, 0),
            sidecar_block(ids[1], "a1", 2, 1),
            sidecar_block(ids[2], "a11", 3, 2),
            sidecar_block(ids[3], "a2", 4, 1),
            sidecar_block(ids[4], "b", 5, 0),
        ];
        assert_eq!(
            sidecar_parents(&old),
            vec![None, Some(0), Some(1), Some(0), None]
        );
    }
}
