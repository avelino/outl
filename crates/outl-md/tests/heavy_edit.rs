//! When a block's text changes substantially, the hash no longer matches
//! the sidecar entry. The positional fallback (pass 1.5) catches this
//! case when the block is at the same DFS index, same indent and same
//! parent, preserving the NodeId — so `((blk-…))` refs and `!((blk-…))`
//! embeds stay stable across edits.
//!
//! When blocks are inserted or deleted, the DFS indices shift and
//! positional fallback can't help. Level 2 (similarity against the
//! sidecar's recorded text) takes over there — see
//! `tests/edit_and_delete.rs`; below its threshold the block falls
//! through to level 3 (new ID) and the old ID surfaces as an orphan.

use outl_core::id::NodeId;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::sidecar::SidecarBlock;

#[test]
fn heavy_edit_preserves_id_via_positional_fallback() {
    let id = NodeId::new();
    let old = vec![SidecarBlock::from_text(
        id,
        1,
        0,
        "the original wording of this block",
    )];

    let edited = "- a wholly different sentence now\n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].old_id,
        Some(id),
        "positional fallback must preserve the NodeId on text edit"
    );
    assert!(orphans.is_empty(), "no orphan on same-position edit");
}

#[test]
fn whitespace_only_change_is_still_a_match() {
    // The content hash normalizes whitespace; thus inserting extra
    // spaces should still match level 1.
    let id = NodeId::new();
    let old = vec![SidecarBlock::from_text(id, 1, 0, "hello world")];

    let edited = "-   hello   world   \n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);
    assert!(orphans.is_empty(), "no orphans on whitespace-only edit");
    assert_eq!(matches[0].old_id, Some(id));
    assert_eq!(matches[0].level, MatchLevel::High);
}
