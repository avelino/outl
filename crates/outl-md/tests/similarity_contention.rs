//! Level 2 must assign ids by **confidence**, not by the order the new
//! blocks appear in the file.
//!
//! The regression these pin: the pass-2 loop used to walk
//! `0..flat.len()` and hand each new block the first old entry that
//! scored above the threshold. A block the user had just typed, sitting
//! at a lower DFS index than the block it merely resembles, took that
//! block's id **and its `ref_handle`** — and the real owner fell to
//! level 3 with a fresh ULID.
//!
//! The runner-up margin could not see it: it only compared candidates
//! belonging to the same new block, never two new blocks contending for
//! the same old entry. And because the old id *was* consumed, `orphans`
//! came back empty, so nothing reached `orphans.log`. Silent corruption
//! is exactly what level 2 exists to prevent — invariant 2.

use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_md::matching::{match_blocks, MatchLevel};
use outl_md::parse::parse;
use outl_md::reconcile::reconcile_md;
use outl_md::sidecar::{self, sidecar_path_for, SidecarBlock};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// The block the user actually edited: the sidecar text plus one word.
const MEETING_EDITED: &str = "reunião com o time de plataforma sobre a migração hoje";
/// The block the user just typed. Same shape, different subject — it
/// scores 0.837 against the sidecar entry, above the 0.8 threshold.
const MEETING_TYPED: &str = "reunião com o time de produto sobre a migração";
/// The sidecar entry both new blocks can claim. `MEETING_EDITED` scores
/// 0.907 against it, so the margin between the two claims is 0.07 —
/// wide enough that the right one wins outright.
const MEETING_OLD: &str = "reunião com o time de plataforma sobre a migração";
/// Untouched neighbour; matches by hash at level 1.
const RFC: &str = "revisar o RFC de storage com o time";

fn write_page(dir: &Path, body: &str) -> std::path::PathBuf {
    let pages = dir.join("pages");
    fs::create_dir_all(&pages).unwrap();
    let path = pages.join("journal.md");
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn a_newly_typed_block_cannot_steal_the_id_of_the_block_it_resembles() {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    let md_path = write_page(dir.path(), &format!("- {MEETING_OLD}\n- {RFC}\n"));
    let log_path = dir.path().join("orphans.log");

    reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();
    let before = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(before.blocks.len(), 2);
    let (meeting_id, meeting_handle) = (before.blocks[0].id, before.blocks[0].ref_handle.clone());
    let rfc_id = before.blocks[1].id;

    // One external save: a new block inserted at the top, and the
    // meeting block reworded. The inserted block is the one that used to
    // win, purely by being visited first.
    fs::write(
        &md_path,
        format!("- {MEETING_TYPED}\n- {MEETING_EDITED}\n- {RFC}\n"),
    )
    .unwrap();
    let report = reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();

    let after = sidecar::read(&sidecar_path_for(&md_path)).unwrap();
    assert_eq!(after.blocks.len(), 3);
    assert_eq!(
        after.blocks[1].id, meeting_id,
        "the block the user edited must keep its id, not the one that merely resembles it"
    );
    assert_eq!(
        after.blocks[1].ref_handle, meeting_handle,
        "…and its ((blk-…)) handle, or every reference to it renders someone else's text"
    );
    assert_ne!(
        after.blocks[0].id, meeting_id,
        "the newly typed block must get a fresh ULID"
    );
    assert_ne!(after.blocks[0].id, rfc_id);
    assert_eq!(after.blocks[2].id, rfc_id, "untouched neighbour, level 1");
    assert_eq!(
        ws.block_text(meeting_id).unwrap_or_default(),
        MEETING_EDITED,
        "the workspace must carry the new text under the same node"
    );
    assert_eq!(report.orphans, 0, "nothing was deleted");
}

#[test]
fn the_closer_claim_wins_regardless_of_which_new_block_comes_first() {
    // Same contention as above with the two new blocks swapped: the
    // edited block is now at index 0 and the typed one at index 1.
    // Confidence must decide, so the outcome is identical.
    let meeting_id = NodeId::new();
    let rfc_id = NodeId::new();
    let old = vec![
        SidecarBlock::from_text(meeting_id, 1, 0, MEETING_OLD),
        SidecarBlock::from_text(rfc_id, 2, 0, RFC),
    ];

    let ast = parse(&format!("- {MEETING_EDITED}\n- {MEETING_TYPED}\n- {RFC}\n"));
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].old_id, Some(meeting_id));
    assert_eq!(matches[0].level, MatchLevel::Medium);
    assert_eq!(
        matches[1].old_id, None,
        "the weaker claim on the same old entry falls to level 3"
    );
    assert_eq!(matches[1].level, MatchLevel::Low);
    assert_eq!(matches[2].old_id, Some(rfc_id));
    assert!(orphans.is_empty());
}

#[test]
fn the_typed_block_first_produces_the_same_assignment() {
    // The ordering from the bug report, at the matcher level — the
    // tightest expression of the regression.
    let meeting_id = NodeId::new();
    let rfc_id = NodeId::new();
    let old = vec![
        SidecarBlock::from_text(meeting_id, 1, 0, MEETING_OLD),
        SidecarBlock::from_text(rfc_id, 2, 0, RFC),
    ];

    let ast = parse(&format!("- {MEETING_TYPED}\n- {MEETING_EDITED}\n- {RFC}\n"));
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    assert_eq!(
        matches[0].old_id, None,
        "index 0 is not a claim to priority"
    );
    assert_eq!(matches[0].level, MatchLevel::Low);
    assert_eq!(matches[1].old_id, Some(meeting_id));
    assert_eq!(matches[1].level, MatchLevel::Medium);
    assert_eq!(matches[2].old_id, Some(rfc_id));
    assert!(
        orphans.is_empty(),
        "a level-3 block here is an insert, not a deletion"
    );
}

#[test]
fn two_equally_close_claims_on_one_old_block_both_decline() {
    // Both new blocks score 0.96 against the same sidecar entry. Picking
    // either is a coin flip that costs a ref handle, so level 2 declines
    // on both sides — and the old id surfaces as an orphan, which is
    // what puts it in `orphans.log` instead of silently on someone
    // else's block.
    let id = NodeId::new();
    let old = vec![SidecarBlock::from_text(
        id,
        1,
        0,
        "decide the storage backend before the sprint ends",
    )];

    let ast = parse(
        "- decide the storage backend before the sprint ended\n\
         - decide the storage backend before this sprint ends\n",
    );
    let (matches, orphans) = match_blocks(&ast.blocks, &old);

    assert_eq!(matches.len(), 2);
    for m in &matches {
        assert_eq!(m.old_id, None, "a tie must not be resolved by position");
        assert_eq!(m.level, MatchLevel::Low);
    }
    assert_eq!(orphans, vec![id], "the contested id must be recoverable");
}
