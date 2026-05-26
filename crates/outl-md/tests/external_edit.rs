//! User opens `.md` in VS Code, edits lightly (adds, deletes), saves.
//! Existing IDs must be preserved for unchanged blocks.

use outl_core::id::NodeId;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::sidecar::{content_hash, derive_ref_handle, SidecarBlock};

fn sb(id: NodeId, text: &str, line: usize, indent: u32) -> SidecarBlock {
    SidecarBlock {
        id,
        line,
        indent,
        content_hash: content_hash(text),
        ref_handle: derive_ref_handle(id),
    }
}

#[test]
fn untouched_save_preserves_all_ids() {
    let md = "- a\n- b\n- c\n";
    let id_a = NodeId::new();
    let id_b = NodeId::new();
    let id_c = NodeId::new();
    let old = vec![
        sb(id_a, "a", 1, 0),
        sb(id_b, "b", 2, 0),
        sb(id_c, "c", 3, 0),
    ];

    let new_ast = parse(md);
    let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
    assert!(orphans.is_empty());
    let preserved: Vec<NodeId> = matches.iter().filter_map(|m| m.old_id).collect();
    assert_eq!(preserved, vec![id_a, id_b, id_c]);
}

#[test]
fn inserting_new_block_in_middle_preserves_others() {
    let original = "- a\n- b\n- c\n";
    let original_ast = parse(original);
    let id_a = NodeId::new();
    let id_b = NodeId::new();
    let id_c = NodeId::new();
    let old = vec![
        sb(id_a, "a", 1, 0),
        sb(id_b, "b", 2, 0),
        sb(id_c, "c", 3, 0),
    ];

    // User inserts "- new" between "a" and "b".
    let edited = "- a\n- new\n- b\n- c\n";
    let new_ast = parse(edited);
    let (matches, orphans) = match_blocks(&new_ast.blocks, &old);
    assert!(orphans.is_empty());

    assert_eq!(matches.len(), 4);
    assert_eq!(matches[0].old_id, Some(id_a));
    assert_eq!(matches[1].old_id, None); // "new" → level 3
    assert_eq!(matches[1].level, MatchLevel::Low);
    assert_eq!(matches[2].old_id, Some(id_b));
    assert_eq!(matches[3].old_id, Some(id_c));

    // Silence unused-import warning when this fixture grows.
    let _ = original_ast;
}

#[test]
fn deleting_middle_block_orphans_it() {
    let original = "- a\n- b\n- c\n";
    let original_ast = parse(original);
    let id_a = NodeId::new();
    let id_b = NodeId::new();
    let id_c = NodeId::new();
    let old = vec![
        sb(id_a, "a", 1, 0),
        sb(id_b, "b", 2, 0),
        sb(id_c, "c", 3, 0),
    ];

    // User deletes "- b".
    let edited = "- a\n- c\n";
    let new_ast = parse(edited);
    let (matches, orphans) = match_blocks(&new_ast.blocks, &old);

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].old_id, Some(id_a));
    assert_eq!(matches[1].old_id, Some(id_c));
    assert_eq!(orphans, vec![id_b]);

    let _ = original_ast;
}
