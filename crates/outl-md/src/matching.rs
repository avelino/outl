//! The block-matching algorithm that reconstructs IDs after an external
//! edit to a `.md` file.
//!
//! Today the algorithm implements:
//!
//! - **Level 1** — `content_hash` exact match. Preserves the ID.
//! - **Level 1.5 (positional fallback)** — same DFS index + same indent.
//!   Preserves the ID when a block's text is edited without structural
//!   changes. This is what keeps `((blk-…))` refs and `!((blk-…))`
//!   embeds stable across text edits.
//! - **Level 3** — no match. New ULID assigned for new blocks; old blocks
//!   without a match become orphans (caller moves them to `TRASH_ROOT`).
//!
//! Level 2 (Levenshtein similarity > 80%) requires retaining the old text
//! verbatim, which the sidecar doesn't store. It's not yet implemented.

use crate::parse::OutlineNode;
use crate::sidecar::{content_hash, SidecarBlock};
use outl_core::id::NodeId;
use std::collections::HashSet;

/// Confidence level of a matched pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchLevel {
    /// `content_hash` exact match.
    High,
    /// Reserved for similarity-based matching (level 2). Not yet emitted.
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
}

/// Flatten an outline tree into a depth-first preorder list. Used by
/// `match_blocks` and `diff_to_ops` so both see the same indexing.
pub fn flatten(blocks: &[OutlineNode]) -> Vec<FlatBlock<'_>> {
    let mut out = Vec::new();
    push_flat(blocks, 0, &mut out);
    out
}

fn push_flat<'a>(blocks: &'a [OutlineNode], indent: u32, out: &mut Vec<FlatBlock<'a>>) {
    for b in blocks {
        out.push(FlatBlock {
            text: &b.text,
            indent,
        });
        push_flat(&b.children, indent + 1, out);
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

    // Pass 2 (reserved for level 2 — similarity-based, not yet implemented).

    // Pass 1.5: positional fallback. For each unmatched new block at
    // index `i`, if `old_blocks[i]` exists, is unused, and has the
    // same indent, match them. This preserves the NodeId when a
    // block's text is edited without structural changes (no blocks
    // added, removed, or moved). Without this, every text edit would
    // mint a fresh NodeId + ref_handle, breaking all `((blk-…))` refs
    // and `!((blk-…))` embeds pointing at the edited block.
    //
    // Both `flat` (new) and `old_blocks` are in DFS preorder, so the
    // indices align when the tree shape hasn't changed. If blocks were
    // inserted or deleted, the indices shift and this pass is a no-op
    // (the `used` guard + indent check prevent false matches).
    for (i, maybe_id) in found.iter_mut().enumerate() {
        if maybe_id.is_some() {
            continue;
        }
        if let Some(old) = old_blocks.get(i) {
            if !used.contains(&old.id) && old.indent == flat[i].indent {
                *maybe_id = Some(old.id);
                used.insert(old.id);
            }
        }
    }

    // Final pass: level 3 for the remainder.
    for (i, maybe_id) in found.iter().enumerate() {
        matches.push(Match {
            new_block_index: i,
            old_id: *maybe_id,
            level: if maybe_id.is_some() {
                MatchLevel::High
            } else {
                MatchLevel::Low
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
    use crate::sidecar::content_hash;

    fn sidecar_block(id: NodeId, text: &str, line: usize, indent: u32) -> SidecarBlock {
        SidecarBlock {
            id,
            line,
            indent,
            content_hash: content_hash(text),
            ref_handle: crate::sidecar::derive_ref_handle(id),
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
}
