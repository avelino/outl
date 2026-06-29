//! Two blocks with identical text swap parents.
//!
//! Matching is greedy first-fit on hash. Both blocks have the
//! same hash, so the algorithm consumes them in order. Final assignment
//! is deterministic; the test here is that the outcome converges and
//! the algorithm does not crash on the ambiguity.
//!
//! Parent/position tiebreaking is not yet implemented. Documenting current
//! behavior in test form so regressions are visible.

use outl_core::id::NodeId;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::sidecar::{content_hash, derive_ref_handle, SidecarBlock};

#[test]
fn identical_blocks_get_first_fit_matching() {
    // Before edit: X contains "TODO" (id_a), Y contains "TODO" (id_b).
    let id_a = NodeId::new();
    let id_b = NodeId::new();
    let old = vec![
        SidecarBlock {
            id: id_a,
            line: 2,
            indent: 1,
            content_hash: content_hash("TODO"),
            ref_handle: derive_ref_handle(id_a),
        },
        SidecarBlock {
            id: id_b,
            line: 4,
            indent: 1,
            content_hash: content_hash("TODO"),
            ref_handle: derive_ref_handle(id_b),
        },
    ];

    // After edit: swapped order (now Y first, then X). Both still say "TODO".
    let edited = "- Y\n  - TODO\n- X\n  - TODO\n";
    let ast = parse(edited);
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    // No orphans — both old TODOs found a new TODO.
    assert!(orphans.is_empty(), "got unexpected orphans: {orphans:?}");

    // The two TODO matches (flat indices 1 and 3 in DFS preorder) are
    // both High level.
    let high_count = matches
        .iter()
        .filter(|m| m.level == MatchLevel::High)
        .count();
    assert!(
        high_count >= 2,
        "expected both TODO blocks to match at High level"
    );

    // Both old IDs were used.
    let used: std::collections::HashSet<NodeId> = matches.iter().filter_map(|m| m.old_id).collect();
    assert!(used.contains(&id_a));
    assert!(used.contains(&id_b));
}
