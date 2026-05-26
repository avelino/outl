//! The block-matching algorithm that reconstructs IDs after an external
//! edit to a `.md` file.
//!
//! Phase 1 implements levels 1 and 3 from `docs/markdown-format.md`:
//!
//! - **Level 1** — `content_hash` exact match. Preserves the ID.
//! - **Level 3** — no match. New ULID assigned for new blocks; old blocks
//!   without a match become orphans (caller moves them to `TRASH_ROOT`).
//!
//! Level 2 (Levenshtein similarity > 80%) requires retaining the old text
//! verbatim, which the sidecar doesn't store. It's deferred to phase 4.
//! Until then, heavy edits silently lose the block ID — but the block
//! **never disappears**: it shows up as an orphan and goes through
//! `outl reconcile`.

use crate::parse::OutlineNode;
use crate::sidecar::{content_hash, SidecarBlock};
use outl_core::id::NodeId;
use std::collections::HashSet;

/// Confidence level of a matched pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchLevel {
    /// `content_hash` exact match.
    High,
    /// Reserved for phase 4 (similarity-based). Never emitted in phase 1.
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

    // Pass 2 (reserved for level 2 — phase 4).

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
    fn heavy_edit_loses_id_and_orphans_old() {
        // Block text completely changed → hash differs → no match.
        let md = "- totally different content here\n";
        let new_ast = parse(md);
        let id = NodeId::new();
        let old = vec![sidecar_block(id, "original wording", 1, 0)];

        let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].old_id, None);
        assert_eq!(matches[0].level, MatchLevel::Low);
        assert_eq!(orphans, vec![id]);
    }
}
