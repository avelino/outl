//! What the truncation scan must and must not report.
//!
//! The bar these hold: a false positive costs the user one line of
//! output, a false negative costs them the content. So the "must not
//! report" cases are the ones that keep the command usable, and the
//! "must report" cases are the ones that make it worth running.

use super::*;
use crate::block::{append_block, delete as delete_block, edit_text};
use crate::page::{open_or_create as open_page, PageKind};
use outl_core::id::ActorId;

fn workspace() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("in-memory workspace"),
        HlcGenerator::new(actor),
    )
}

/// A block that went through `texts` in order, under `parent`.
fn block_through(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    parent: Option<NodeId>,
    texts: &[&str],
) -> NodeId {
    let node = append_block(ws, hlc, parent, Some(texts[0])).expect("append");
    for text in &texts[1..] {
        edit_text(ws, hlc, node, text).expect("edit");
    }
    node
}

const LONG: &str = "🚨 Briefing\nfirst finding\nsecond finding\nthird finding";

#[test]
fn a_truncating_edit_is_reported_with_everything_it_dropped() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].node, node);
    assert_eq!(found[0].current, "🚨 Briefing");
    assert_eq!(found[0].recovered, LONG);
    assert_eq!(
        found[0].lost_lines,
        vec!["first finding", "second finding", "third finding"]
    );
}

#[test]
fn a_block_that_only_ever_grew_is_not_a_candidate() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &["one", "one two", "one two three"]);
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

/// A rewrite is not a truncation. The user replaced the text; nothing in
/// the log says the old version was meant to survive.
#[test]
fn an_edit_that_rewrote_the_block_is_not_a_candidate() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &[LONG, "something else entirely"]);
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

/// The empty string is a prefix of everything, so a cleared block would
/// otherwise drag every deletion in the workspace into the report.
#[test]
fn a_cleared_block_is_not_a_candidate() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &[LONG, ""]);
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

/// The renderer's trailing-newline behaviour changed between releases,
/// and a guard that fires on that noise is a guard nobody reads.
#[test]
fn losing_only_trailing_whitespace_is_not_a_loss() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &["a briefing\n\n  \n", "a briefing"]);
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

/// A trashed block is one the user deleted. Its history is just as
/// recoverable, and offering it back resurrects something removed on
/// purpose.
#[test]
fn a_trashed_block_is_left_alone() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    assert_eq!(scan_truncated_blocks(&ws, 1).expect("scan").len(), 1);

    delete_block(&mut ws, &hlc, node).expect("delete");
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

#[test]
fn min_lost_lines_raises_the_bar_and_never_lowers_it() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &["kept\ndropped", "kept"]);

    assert_eq!(scan_truncated_blocks(&ws, 1).expect("scan").len(), 1);
    assert!(scan_truncated_blocks(&ws, 2).expect("scan").is_empty());
}

/// Three revisions, the middle one also a truncation. The longest
/// revision that still starts with the current text is the one worth
/// recovering — picking the nearest would silently settle for less.
#[test]
fn the_longest_still_matching_revision_wins() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &["a\nb\nc\nd", "a\nb", "a"]);

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    assert_eq!(found[0].recovered, "a\nb\nc\nd");
    assert_eq!(found[0].lost_lines, vec!["b", "c", "d"]);
}

/// The property that makes `--apply` safe by construction: restoring
/// puts back a superset of what the block shows today.
#[test]
fn every_recovered_revision_contains_the_current_text() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    block_through(&mut ws, &hlc, None, &["a\nb\nc", "a"]);

    for found in scan_truncated_blocks(&ws, 1).expect("scan") {
        assert!(
            found.recovered.starts_with(&found.current),
            "{:?} does not start with {:?}",
            found.recovered,
            found.current
        );
    }
}

#[test]
fn the_worst_loss_is_reported_first() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &["small\nonly one", "small"]);
    block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    let losses: Vec<usize> = found.iter().map(|f| f.lost_lines.len()).collect();
    assert_eq!(losses, vec![3, 1]);
}

#[test]
fn a_block_is_reported_with_the_page_hosting_it() {
    let (mut ws, hlc) = workspace();
    let page = open_page(&mut ws, &hlc, "briefing", "Briefing", PageKind::Page).expect("open page");
    block_through(&mut ws, &hlc, Some(page), &[LONG, "🚨 Briefing"]);

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    assert_eq!(found[0].page_slug.as_deref(), Some("briefing"));
}

/// Restoring is a new `Op::Edit`, never a rewrite of the log (root
/// `CLAUDE.md` invariant 1). The truncating edit stays in the history.
#[test]
fn restoring_appends_an_op_and_leaves_the_earlier_ones_in_place() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    let before = ws.log().len();

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    restore_truncated_block(&mut ws, &hlc, &found[0]).expect("restore");

    assert_eq!(ws.block_text(node).as_deref(), Some(LONG));
    assert_eq!(ws.log().len(), before + 1);
    let history = ws.block_text_history(node).expect("history");
    assert_eq!(history.len(), 3);
    assert_eq!(history[1], "🚨 Briefing");
}

/// Once restored, the block is no longer a candidate — so a second run
/// of the command is a no-op instead of a loop.
#[test]
fn a_restored_block_is_not_reported_again() {
    let (mut ws, hlc) = workspace();
    block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);

    let found = scan_truncated_blocks(&ws, 1).expect("scan");
    restore_truncated_block(&mut ws, &hlc, &found[0]).expect("restore");
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}

/// The scan and the write are two passes, and the workspace is shared
/// with a running GUI and with peers. A block that moved on in between
/// must not be reverted to a revision from before that change.
#[test]
fn restoring_refuses_a_block_that_changed_since_the_scan() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    let found = scan_truncated_blocks(&ws, 1).expect("scan");

    edit_text(&mut ws, &hlc, node, "the user rewrote this").expect("edit");
    assert!(restore_truncated_block(&mut ws, &hlc, &found[0]).is_err());
    assert_eq!(
        ws.block_text(node).as_deref(),
        Some("the user rewrote this")
    );
}

/// The stale shape a prefix test waves through: the user cleared the
/// block after the scan. Every recovery starts with the empty string, so
/// "is the live text still a prefix" says yes and the deletion is undone
/// behind their back.
#[test]
fn restoring_refuses_a_block_the_user_cleared_since_the_scan() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    let found = scan_truncated_blocks(&ws, 1).expect("scan");

    edit_text(&mut ws, &hlc, node, "").expect("clear");
    assert!(restore_truncated_block(&mut ws, &hlc, &found[0]).is_err());
    assert_eq!(ws.block_text(node).as_deref(), Some(""));
}

/// Same hole, one step less extreme: the user trimmed the block to a
/// shorter prefix of what the scan saw. Still a change they made after
/// the scan, still not ours to revert.
#[test]
fn restoring_refuses_a_block_shortened_to_a_prefix_since_the_scan() {
    let (mut ws, hlc) = workspace();
    let node = block_through(&mut ws, &hlc, None, &[LONG, "🚨 Briefing"]);
    let found = scan_truncated_blocks(&ws, 1).expect("scan");

    edit_text(&mut ws, &hlc, node, "🚨").expect("shorten");
    assert!(restore_truncated_block(&mut ws, &hlc, &found[0]).is_err());
    assert_eq!(ws.block_text(node).as_deref(), Some("🚨"));
}

/// A workspace where nothing was ever truncated must cost nothing and
/// report nothing — the ordinary case for every user who never hit the
/// parser bug.
#[test]
fn a_healthy_workspace_reports_nothing() {
    let (mut ws, hlc) = workspace();
    let page = open_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("open page");
    block_through(&mut ws, &hlc, Some(page), &["just a note"]);
    block_through(&mut ws, &hlc, Some(page), &["grew", "grew a lot"]);
    assert!(scan_truncated_blocks(&ws, 1).expect("scan").is_empty());
}
