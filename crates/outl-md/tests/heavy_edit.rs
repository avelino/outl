//! When a block's text changes substantially, the hash no longer matches
//! the sidecar entry. Phase 1 (level 1 + level 3 only) treats this as a
//! delete + new block. The orphan must show up — never silent loss.
//!
//! Phase 4 will introduce level 2 (similarity > 80%) and the warning
//! path in `.outl/orphans.log`. Until then this test documents the
//! conservative-but-safe behavior.

use outl_core::id::NodeId;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::sidecar::{content_hash, SidecarBlock};

#[test]
fn heavy_edit_orphans_old_id_and_creates_new() {
    let id = NodeId::new();
    let old = vec![SidecarBlock {
        id,
        line: 1,
        indent: 0,
        content_hash: content_hash("the original wording of this block"),
    }];

    let edited = "- a wholly different sentence now\n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].old_id, None);
    assert_eq!(matches[0].level, MatchLevel::Low);
    assert_eq!(orphans, vec![id], "the old id must surface as an orphan");
}

#[test]
fn whitespace_only_change_is_still_a_match() {
    // The content hash normalizes whitespace; thus inserting extra
    // spaces should still match level 1.
    let id = NodeId::new();
    let old = vec![SidecarBlock {
        id,
        line: 1,
        indent: 0,
        content_hash: content_hash("hello world"),
    }];

    let edited = "-   hello   world   \n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);
    assert!(orphans.is_empty(), "no orphans on whitespace-only edit");
    assert_eq!(matches[0].old_id, Some(id));
    assert_eq!(matches[0].level, MatchLevel::High);
}
