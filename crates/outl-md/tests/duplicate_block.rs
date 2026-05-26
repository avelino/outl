//! Ctrl+D in VS Code duplicates a block. The first copy must keep the
//! original ID; the second must get a fresh ULID at level 3.

use outl_core::id::NodeId;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::sidecar::{content_hash, derive_ref_handle, SidecarBlock};

#[test]
fn ctrl_d_first_keeps_id_second_gets_new() {
    let id = NodeId::new();
    let old = vec![SidecarBlock {
        id,
        line: 1,
        indent: 0,
        content_hash: content_hash("hello"),
        ref_handle: derive_ref_handle(id),
    }];

    // After Ctrl+D in VS Code.
    let edited = "- hello\n- hello\n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);
    assert!(
        orphans.is_empty(),
        "no orphans expected when nothing was deleted"
    );

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].old_id, Some(id));
    assert_eq!(matches[0].level, MatchLevel::High);
    assert_eq!(matches[1].old_id, None);
    assert_eq!(matches[1].level, MatchLevel::Low);
}

#[test]
fn three_copies_of_same_content_two_get_new_ids() {
    let id = NodeId::new();
    let old = vec![SidecarBlock {
        id,
        line: 1,
        indent: 0,
        content_hash: content_hash("hi"),
        ref_handle: derive_ref_handle(id),
    }];

    let edited = "- hi\n- hi\n- hi\n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);
    assert!(orphans.is_empty());
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].old_id, Some(id));
    assert_eq!(matches[1].old_id, None);
    assert_eq!(matches[2].old_id, None);
}
