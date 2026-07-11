//! Repair journal titles doubled by concurrent offline creation.
//!
//! Before the title moved into the `title::` property (see
//! [`crate::page::open_or_create`]), a page/journal root stored its title
//! in the root node's Yrs text. With deterministic root ids two devices
//! that opened the same day's journal offline minted the **same** root
//! node — the `Op::Create` converges — but each device also ran
//! `edit_text(root, slug)`, and those two concurrent Yrs inserts at
//! position 0 concatenate into `"2026-06-252026-06-25"`.
//!
//! This pass finds journal roots whose text is the slug repeated k >= 2
//! times and clears it, so the title falls back to the slug via
//! [`crate::page::page_meta`]. The clear is an `Op::Edit` through
//! [`Workspace::apply`], so the repair converges to every device through
//! the op log — running it on any one client fixes the others.
//!
//! It scales with the number of pages, so clients run it on their
//! **background** reconcile pass, never the synchronous boot path.
//! Idempotent: a workspace with no doubled titles emits no ops.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::page::{PageKind, KIND_KEY, SLUG_KEY};
use crate::tree::children_of;

/// Clear the doubled text on every journal root whose text is its slug
/// repeated two or more times. Returns the number of roots repaired
/// (`0` when the workspace is already clean).
///
/// Restricted to journals (`page-kind = "journal"`): a journal's title
/// equals its slug, so slug-repetition is an unambiguous corruption
/// signature there. Regular pages carry a distinct human title, so this
/// pass leaves them untouched.
pub fn repair_doubled_journal_titles(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<usize, ActionError> {
    // Collect first — `edit_text` mutates the tree while we'd otherwise
    // still be borrowing the child iterator.
    let targets: Vec<NodeId> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| {
            if PageKind::parse(workspace.tree().property(id, KIND_KEY)) != PageKind::Journal {
                return None;
            }
            let slug = match workspace.tree().property(id, SLUG_KEY) {
                Some(PropValue::Text(s)) => s.clone(),
                _ => return None,
            };
            let text = workspace.block_text(id)?;
            is_repeated_slug(&text, &slug).then_some(id)
        })
        .collect();

    let mut repaired = 0usize;
    for id in targets {
        crate::block::edit_text(workspace, hlc, id, "")?;
        repaired += 1;
    }
    Ok(repaired)
}

/// True when `text` is `slug` repeated k >= 2 times. A single copy
/// (k == 1, a normal pre-migration journal title) and an empty text are
/// left alone so a clean workspace stays op-free.
fn is_repeated_slug(text: &str, slug: &str) -> bool {
    if slug.is_empty() || text.len() <= slug.len() || text.len() % slug.len() != 0 {
        return false;
    }
    let k = text.len() / slug.len();
    k >= 2 && *text == slug.repeat(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{find_by_slug, open_journal, open_or_create, page_meta, PageKind};
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::op::LogOp;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    /// Cross-deliver every op `from` produced into `into`.
    fn deliver(from: &Workspace, into: &mut Workspace) {
        let ops: Vec<LogOp> = from.log().iter().cloned().collect();
        for op in ops {
            into.apply(op).unwrap();
        }
    }

    /// The prevention half: two devices create the SAME page offline, then
    /// sync. The title used to double (concurrent Yrs inserts into the same
    /// deterministic-id root's text); storing it as a `title::` property
    /// (`Op::SetProp`, last-write-wins) converges to a single value.
    #[test]
    fn concurrent_create_does_not_double_a_regular_page_title() {
        let a1 = ActorId::new();
        let a2 = ActorId::new();
        let g1 = HlcGenerator::new(a1);
        let g2 = HlcGenerator::new(a2);
        let mut ws1 = Workspace::open_in_memory(a1).unwrap();
        let mut ws2 = Workspace::open_in_memory(a2).unwrap();

        open_or_create(&mut ws1, &g1, "ideas", "Ideas", PageKind::Page).unwrap();
        open_or_create(&mut ws2, &g2, "ideas", "Ideas", PageKind::Page).unwrap();

        // Sync both ways. Re-delivery is idempotent (dedup by op id), so the
        // second call redelivering the first's now-merged log is harmless.
        deliver(&ws2, &mut ws1);
        deliver(&ws1, &mut ws2);

        for w in [&ws1, &ws2] {
            let id = find_by_slug(w, "ideas").unwrap();
            assert_eq!(page_meta(w, id).unwrap().title, "Ideas");
        }
    }

    /// Same, for journals: two devices auto-create the same day. The title
    /// derives from the slug (no root text, no `title::`), so there is no
    /// concurrent text to concatenate.
    #[test]
    fn concurrent_journal_open_does_not_double_the_title() {
        let a1 = ActorId::new();
        let a2 = ActorId::new();
        let g1 = HlcGenerator::new(a1);
        let g2 = HlcGenerator::new(a2);
        let mut ws1 = Workspace::open_in_memory(a1).unwrap();
        let mut ws2 = Workspace::open_in_memory(a2).unwrap();
        let date = crate::dates::date_from_slug("2026-06-25").unwrap();

        open_journal(&mut ws1, &g1, date).unwrap();
        open_journal(&mut ws2, &g2, date).unwrap();

        let ops2: Vec<LogOp> = ws2.log().iter().cloned().collect();
        let ops1: Vec<LogOp> = ws1.log().iter().cloned().collect();
        for op in ops2 {
            ws1.apply(op).unwrap();
        }
        for op in ops1 {
            ws2.apply(op).unwrap();
        }

        for w in [&ws1, &ws2] {
            let id = find_by_slug(w, "2026-06-25").unwrap();
            assert_eq!(page_meta(w, id).unwrap().title, "2026-06-25");
        }
    }

    #[test]
    fn detects_doubled_and_tripled_slug() {
        assert!(is_repeated_slug("2026-06-252026-06-25", "2026-06-25"));
        assert!(is_repeated_slug(
            "2026-06-252026-06-252026-06-25",
            "2026-06-25"
        ));
    }

    #[test]
    fn leaves_single_and_empty_and_unrelated_alone() {
        assert!(!is_repeated_slug("2026-06-25", "2026-06-25"));
        assert!(!is_repeated_slug("", "2026-06-25"));
        assert!(!is_repeated_slug("morning notes", "2026-06-25"));
        // A partial second copy is not a whole repetition.
        assert!(!is_repeated_slug("2026-06-252026", "2026-06-25"));
    }

    /// Simulate the corruption: a journal root whose text is the slug
    /// twice. The repair clears it and the title falls back to the slug.
    #[test]
    fn repairs_a_doubled_journal_title() {
        let (mut w, hlc) = ws();
        let date = crate::dates::date_from_slug("2026-06-25").unwrap();
        let id = open_journal(&mut w, &hlc, date).unwrap();
        // Force the doubled text a corrupted workspace would carry.
        crate::block::edit_text(&mut w, &hlc, id, "2026-06-252026-06-25").unwrap();
        assert_eq!(w.block_text(id).as_deref(), Some("2026-06-252026-06-25"));

        let n = repair_doubled_journal_titles(&mut w, &hlc).unwrap();
        assert_eq!(n, 1);
        // Text cleared → title derives from the slug.
        assert!(w.block_text(id).unwrap_or_default().is_empty());
        assert_eq!(crate::page::page_meta(&w, id).unwrap().title, "2026-06-25");
    }

    #[test]
    fn is_a_noop_on_a_clean_workspace_and_idempotent() {
        let (mut w, hlc) = ws();
        let date = crate::dates::date_from_slug("2026-06-25").unwrap();
        open_journal(&mut w, &hlc, date).unwrap();
        // Fresh journals no longer store any root text, so nothing to fix.
        assert_eq!(repair_doubled_journal_titles(&mut w, &hlc).unwrap(), 0);

        // Corrupt one, repair, then a second run is a no-op.
        let id = open_journal(
            &mut w,
            &hlc,
            crate::dates::date_from_slug("2026-06-23").unwrap(),
        )
        .unwrap();
        crate::block::edit_text(&mut w, &hlc, id, "2026-06-232026-06-23").unwrap();
        assert_eq!(repair_doubled_journal_titles(&mut w, &hlc).unwrap(), 1);
        assert_eq!(repair_doubled_journal_titles(&mut w, &hlc).unwrap(), 0);
    }

    /// A regular page whose title is not a slug-repetition is never
    /// touched, even if its text happens to be non-empty.
    #[test]
    fn ignores_regular_pages() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        // Simulate a legacy in-app page that stored its title as root text.
        crate::block::edit_text(&mut w, &hlc, id, "IdeasIdeas").unwrap();
        assert_eq!(repair_doubled_journal_titles(&mut w, &hlc).unwrap(), 0);
        assert_eq!(w.block_text(id).as_deref(), Some("IdeasIdeas"));
    }
}
