//! Backlinks: which blocks reference which pages.
//!
//! A reference is a literal `[[target]]` substring inside a block's
//! text. `target` matches either a page's slug or its title (the page
//! root's text). Tags (`#tag`) and block refs (`((blk-X))`) are
//! handled by `outl-md::inline` — this module is the workspace-level
//! "which page mentions me" view.

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::page::{page_meta, PageMeta};
use crate::tree::children_of;

/// One backlink reference from a source block to a target page.
#[derive(Debug, Clone, Serialize)]
pub struct Backlink {
    /// Block that contains the `[[target]]` mention.
    pub block_id: String,
    /// Text of the source block, as stored.
    pub block_text: String,
    /// Page that contains the source block, if any. `None` only when
    /// the block lives outside any page (legacy / migrated workspaces).
    pub source_page: Option<PageMeta>,
}

/// Every block in the workspace whose text mentions `[[target]]`.
///
/// `target` is matched literally — pass the page's slug AND title
/// separately if you want to catch both forms.
pub fn backlinks_for_target(workspace: &Workspace, target: &str) -> Vec<Backlink> {
    let needle = format!("[[{target}]]");
    let mut out = Vec::new();
    walk(workspace, NodeId::root(), &mut |id| {
        let text = workspace.block_text(id).unwrap_or_default();
        if text.contains(&needle) {
            out.push(Backlink {
                block_id: id.to_string(),
                block_text: text,
                source_page: enclosing_page(workspace, id),
            });
        }
    });
    out
}

/// Convenience: backlinks against either the page's slug or its title.
pub fn backlinks_for_page(workspace: &Workspace, meta: &PageMeta) -> Vec<Backlink> {
    let mut by_slug = backlinks_for_target(workspace, &meta.slug);
    if meta.title != meta.slug {
        let by_title = backlinks_for_target(workspace, &meta.title);
        // De-dupe by block id (a block might mention both forms).
        for link in by_title {
            if !by_slug.iter().any(|l| l.block_id == link.block_id) {
                by_slug.push(link);
            }
        }
    }
    by_slug
}

/// Extract every `[[ref]]` target out of a block's text. An
/// unterminated `[[` is skipped without consuming anything inside it,
/// so a later well-formed `[[ok]]` is still picked up.
pub fn extract_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if !(bytes[i] == b'[' && bytes[i + 1] == b'[') {
            i += 1;
            continue;
        }
        let start = i + 2;
        let mut j = start;
        let mut closed = false;
        while j + 1 < bytes.len() {
            if bytes[j] == b'[' && bytes[j + 1] == b'[' {
                // Outer was unterminated; bail so the inner gets its
                // own attempt on the next outer-loop iteration.
                break;
            }
            if bytes[j] == b']' && bytes[j + 1] == b']' {
                closed = true;
                break;
            }
            j += 1;
        }
        if closed {
            if let Ok(s) = std::str::from_utf8(&bytes[start..j]) {
                if !s.is_empty() {
                    refs.push(s.to_string());
                }
            }
            i = j + 2;
        } else {
            // Skip just the unterminated `[[` and try again.
            i += 2;
        }
    }
    refs
}

fn walk<F: FnMut(NodeId)>(workspace: &Workspace, parent: NodeId, f: &mut F) {
    for (id, _) in children_of(workspace, parent) {
        f(id);
        walk(workspace, id, f);
    }
}

fn enclosing_page(workspace: &Workspace, mut node: NodeId) -> Option<PageMeta> {
    loop {
        let parent = workspace.tree().parent(node)?;
        if parent == NodeId::root() {
            return page_meta(workspace, node);
        }
        node = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{append_block, edit_text};
    use crate::page::{open_journal, open_or_create, PageKind};
    use chrono::NaiveDate;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn extract_refs_finds_multiple_tokens() {
        let refs = extract_refs("see [[avelino]] and [[2026-05-27]] please");
        assert_eq!(refs, vec!["avelino".to_string(), "2026-05-27".to_string()]);
    }

    #[test]
    fn extract_refs_ignores_unbalanced() {
        let refs = extract_refs("[[unterminated and [[ok]] mixed");
        assert!(refs.contains(&"ok".to_string()));
    }

    #[test]
    fn backlinks_find_blocks_pointing_at_slug() {
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] shipped it")).unwrap();
        let _ = target;

        let links = backlinks_for_target(&w, "avelino");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
        assert_eq!(
            links[0].source_page.as_ref().map(|p| p.slug.clone()),
            Some("2026-05-27".to_string())
        );
    }

    #[test]
    fn backlinks_for_a_future_journal_finds_blocks_in_past_journals() {
        // Scenario: I open today's journal and write a task that
        // tags tomorrow (`[[2026-05-30]]`). Tomorrow, when I open
        // that journal, the section "Linked from" should list my
        // task. This is the workflow the user described as their
        // primary use of journals.
        let (mut w, hlc) = ws();

        let today = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 5, 30).unwrap();

        // Today's journal carries a block tagging tomorrow.
        let today_id = open_journal(&mut w, &hlc, today).unwrap();
        let task = append_block(
            &mut w,
            &hlc,
            Some(today_id),
            Some("call avelino back [[2026-05-30]]"),
        )
        .unwrap();

        // Open tomorrow's journal and pull its backlinks.
        let tomorrow_id = open_journal(&mut w, &hlc, tomorrow).unwrap();
        let meta = page_meta(&w, tomorrow_id).unwrap();
        let links = backlinks_for_page(&w, &meta);

        assert_eq!(
            links.len(),
            1,
            "tomorrow's journal should see today's mention"
        );
        assert_eq!(links[0].block_id, task.to_string());
        assert_eq!(
            links[0].source_page.as_ref().map(|p| p.slug.clone()),
            Some("2026-05-29".to_string())
        );
    }

    #[test]
    fn backlinks_for_page_dedup_slug_and_title() {
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, avelino).unwrap();

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        // One block mentions both forms.
        let n = append_block(&mut w, &hlc, Some(day), Some("[[avelino]] aka [[Avelino]]")).unwrap();
        let _ = edit_text;

        let links = backlinks_for_page(&w, &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, n.to_string());
    }
}
