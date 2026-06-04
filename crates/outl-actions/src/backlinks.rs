//! Backlinks: which blocks reference which pages.
//!
//! A reference is a literal `[[target]]` substring inside a block's
//! text. `target` matches either a page's slug or its title (the page
//! root's text). Tags (`#tag`) and block refs (`((blk-X))`) are
//! handled by `outl-md::inline` — this module is the workspace-level
//! "which page mentions me" view.
//!
//! **This is the single source of truth for backlinks.** Both the
//! mobile client and the TUI consume [`backlinks_for_page`]; the
//! `outl-md::index` crate intentionally does NOT carry a parallel
//! backlinks cache — that earlier duplication was the bug that made
//! self-references invisible on one surface but not the other.

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::{Serialize, Serializer};

use crate::journal::page_md_path;
use crate::outline::{project_outline_node, OutlineNode};
use crate::page::{page_meta, PageMeta};
use crate::todo::{split_todo, TodoState};
use crate::tree::children_of;

/// One backlink reference from a source block to a target page.
///
/// Carries everything a UI needs to render the source block inline —
/// the block's own text and TODO state, the page it lives in, plus
/// the source block as an [`OutlineNode`] subtree so the renderer can
/// surface children + properties without a second workspace lookup.
///
/// `block_text` is the body **without** the `TODO `/`DONE ` prefix;
/// the prefix (if any) lives in [`Self::todo`]. Clients must surface
/// the TODO state with their own checkbox widget — there is no
/// marker left in `block_text` to fall back on.
#[derive(Debug, Clone, Serialize)]
pub struct Backlink {
    /// Block that contains the `[[target]]` mention.
    pub block_id: String,
    /// Body of the source block, with the TODO/DONE prefix stripped.
    ///
    /// **Consumed by the CLI/MCP JSON envelope** (`outl page rename`
    /// returns it inside `affected_refs`), not by the mobile renderer.
    /// Mobile reads `source_block.tokens` / `source_block.text` instead;
    /// the TS `Backlink` interface deliberately omits this field.
    /// Removing it from Rust would break the CLI contract.
    pub block_text: String,
    /// `None` for a plain bullet, `Some(Todo)` / `Some(Done)` otherwise.
    /// Serialised as `"TODO"` / `"DONE"` / `null` to match
    /// [`crate::outline::OutlineNode::todo`].
    #[serde(serialize_with = "serialize_todo_state")]
    pub todo: Option<TodoState>,
    /// Page that contains the source block, if any. `None` only when
    /// the block lives outside any page (legacy / migrated workspaces).
    pub source_page: Option<PageMeta>,
    /// Source block as a self-contained outline subtree (children +
    /// properties). Mirrors the shape `read_page_view_with_workspace`
    /// would return for the same block. UI clients that render
    /// backlinks as a mini-outline (TUI today, mobile in the future)
    /// consume this; light clients can ignore it and use only
    /// `block_text` + `todo`.
    pub source_block: OutlineNode,
    /// DFS path of the source block inside its `source_page`. Empty
    /// vector means the block is a direct child of the page root.
    /// Used by the TUI to track which block inside a backlink's
    /// subtree the cursor is on (`Focus::Backlink { sub_path }`).
    pub source_block_path: Vec<usize>,
    /// On-disk path of the source page's `.md`. Derived from the
    /// workspace root passed to [`backlinks_for_page`] / [`backlinks_for_target`].
    /// `None` when the block has no enclosing page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
}

fn serialize_todo_state<S>(state: &Option<TodoState>, ser: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match state {
        None => ser.serialize_none(),
        Some(s) => ser.serialize_str(s.as_str()),
    }
}

/// Every block in the workspace whose text mentions `[[target]]`.
///
/// `target` is matched literally — pass the page's slug AND title
/// separately if you want to catch both forms. `root` is the
/// workspace root directory; it's needed so each backlink can carry
/// its `source_path` (the `.md` of the page the source block lives
/// in).
pub fn backlinks_for_target(workspace: &Workspace, root: &Path, target: &str) -> Vec<Backlink> {
    let needle = format!("[[{target}]]");
    let mut out: Vec<Backlink> = Vec::new();

    for (page_id, _) in children_of(workspace, NodeId::root()) {
        let Some(meta) = page_meta(workspace, page_id) else {
            continue;
        };
        let source_path = page_md_path(root, &meta);
        let mut path: Vec<usize> = Vec::new();
        walk_inside_page(
            workspace,
            page_id,
            &meta,
            &source_path,
            &mut path,
            &needle,
            &mut out,
        );
    }
    out
}

/// Convenience: backlinks against either the page's slug or its title.
pub fn backlinks_for_page(workspace: &Workspace, root: &Path, meta: &PageMeta) -> Vec<Backlink> {
    let mut by_slug = backlinks_for_target(workspace, root, &meta.slug);
    if meta.title != meta.slug {
        let by_title = backlinks_for_target(workspace, root, &meta.title);
        // De-dupe by block id (a block might mention both forms).
        for link in by_title {
            if !by_slug.iter().any(|l| l.block_id == link.block_id) {
                by_slug.push(link);
            }
        }
    }
    by_slug
}

/// Recursive helper: descend into `parent`'s children, tracking the
/// DFS path. Every block whose text contains `needle` becomes a
/// [`Backlink`] in `out`.
#[allow(clippy::too_many_arguments)]
fn walk_inside_page(
    workspace: &Workspace,
    parent: NodeId,
    meta: &PageMeta,
    source_path: &Path,
    path: &mut Vec<usize>,
    needle: &str,
    out: &mut Vec<Backlink>,
) {
    for (idx, (child_id, _)) in children_of(workspace, parent).into_iter().enumerate() {
        path.push(idx);
        let text = workspace.block_text(child_id).unwrap_or_default();
        if text.contains(needle) {
            let (todo, body) = split_todo(&text);
            let source_block = project_outline_node(workspace, child_id);
            out.push(Backlink {
                block_id: child_id.to_string(),
                block_text: body.to_string(),
                todo,
                source_page: Some(meta.clone()),
                source_block,
                source_block_path: path.clone(),
                source_path: Some(source_path.to_path_buf()),
            });
        }
        walk_inside_page(workspace, child_id, meta, source_path, path, needle, out);
        path.pop();
    }
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

    /// Tests don't need a real filesystem — `source_path` is just
    /// `root + journals/<slug>.md` / `pages/<slug>.md`. Using a
    /// constant root keeps assertions readable.
    fn root() -> &'static Path {
        Path::new("/tmp/outl-test")
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

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
        assert_eq!(
            links[0].source_page.as_ref().map(|p| p.slug.clone()),
            Some("2026-05-27".to_string())
        );
        assert_eq!(
            links[0].source_path.as_deref(),
            Some(root().join("journals/2026-05-27.md").as_path()),
            "journal source page resolves to journals/<slug>.md"
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
        let links = backlinks_for_page(&w, root(), &meta);

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
    fn backlinks_split_todo_prefix_from_body() {
        // Regression: the mobile client renders `block_text` through a
        // plain markdown tokenizer, so the `TODO `/`DONE ` prefix can't
        // leak into `block_text` — it has to live in `todo` so the
        // frontend can paint a checkbox instead of literal text.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "derick", "Derick", PageKind::Page).unwrap();
        let _ = target;

        let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 2).unwrap()).unwrap();
        let _todo = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("TODO [[derick]] agendar papo"),
        )
        .unwrap();
        let _done = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("DONE [[derick]] enviou contrato"),
        )
        .unwrap();
        let _plain = append_block(&mut w, &hlc, Some(day), Some("[[derick]] note solta")).unwrap();

        let mut links = backlinks_for_target(&w, root(), "derick");
        // Order is DFS-stable; sort by text just so assertions don't
        // depend on insertion order.
        links.sort_by(|a, b| a.block_text.cmp(&b.block_text));

        assert_eq!(links.len(), 3);

        assert_eq!(links[0].block_text, "[[derick]] agendar papo");
        assert_eq!(links[0].todo, Some(TodoState::Todo));

        assert_eq!(links[1].block_text, "[[derick]] enviou contrato");
        assert_eq!(links[1].todo, Some(TodoState::Done));

        assert_eq!(links[2].block_text, "[[derick]] note solta");
        assert_eq!(links[2].todo, None);
    }

    #[test]
    fn backlinks_include_self_references_inside_the_same_page() {
        // Reproduce user-reported bug: in today's journal there is a
        // block whose text contains `[[2026-06-02]]` (a link back to
        // the page the block lives in). The "Linked from" panel
        // should still list it — the user typed the ref expecting it
        // to show up in their cross-references view.
        let (mut w, hlc) = ws();
        let today_date = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let today = open_journal(&mut w, &hlc, today_date).unwrap();
        let block = append_block(
            &mut w,
            &hlc,
            Some(today),
            Some("@Derick agendar papo [[2026-06-02]]"),
        )
        .unwrap();

        let meta = page_meta(&w, today).unwrap();
        let links = backlinks_for_page(&w, root(), &meta);

        assert!(
            links.iter().any(|l| l.block_id == block.to_string()),
            "self-ref backlink missing: links = {links:#?}"
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

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, n.to_string());
    }

    #[test]
    fn block_with_repeated_reference_only_emits_one_backlink() {
        // A block whose text contains the same `[[target]]` twice
        // should still appear once. The previous `outl-md` index used
        // a per-block `HashSet` to dedupe; we get the same behaviour
        // here because `text.contains(needle)` is a yes/no probe, not
        // a counter.
        let (mut w, hlc) = ws();
        let _avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let _ = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("[[avelino]] and again [[avelino]]"),
        )
        .unwrap();

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn backlink_carries_source_block_subtree_and_path() {
        // The TUI renders backlinks as a mini-outline: the source
        // block plus its children, with the DFS path so the cursor
        // can land on a specific descendant. The struct must carry
        // both, ready-to-render.
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let _ = avelino;

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let _first = append_block(&mut w, &hlc, Some(day), Some("warmup block")).unwrap();
        let parent =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] led the project")).unwrap();
        let _child_a = append_block(&mut w, &hlc, Some(parent), Some("milestone A")).unwrap();
        let _child_b = append_block(&mut w, &hlc, Some(parent), Some("milestone B")).unwrap();

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
        let bl = &links[0];

        // Path = [1] because the matching block is the second direct
        // child of `day` (index 1, after `warmup block` at index 0).
        assert_eq!(bl.source_block_path, vec![1]);

        // Source block subtree mirrors what the outline panel would
        // show on that page.
        assert_eq!(bl.source_block.text, "[[avelino]] led the project");
        assert_eq!(bl.source_block.children.len(), 2);
        assert_eq!(bl.source_block.children[0].text, "milestone A");
        assert_eq!(bl.source_block.children[1].text, "milestone B");
    }
}
