//! Backlinks: which blocks reference which pages.
//!
//! A reference is either a literal `[[target]]` substring inside a
//! block's text or a `#tag` token whose slug form resolves to the
//! target page — the same `slugify` rule a tag click goes through
//! (`open_or_create_by_name`), so "what opens the page" and "what
//! shows up in the page's backlinks" can't drift. `target` matches
//! either a page's slug or its title (the page root's text). Block
//! refs (`((blk-X))`) are handled by `outl-md::inline` — this module
//! is the workspace-level "which page mentions me" view.
//!
//! **This is the single source of truth for backlinks.** Both the
//! mobile client and the TUI consume [`backlinks_for_page`]; the
//! `outl-md::index` crate intentionally does NOT carry a parallel
//! backlinks cache — that earlier duplication was the bug that made
//! self-references invisible on one surface but not the other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::{Serialize, Serializer};

use crate::journal::page_md_path;
use crate::outline::{project_outline_node_indexed, ChildrenIndex, OutlineNode};
use crate::page::{page_meta, PageMeta};
use crate::todo::{split_todo, TodoState};

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
    /// Ancestor blocks between the page root and the source block,
    /// **root-first**: `ancestors[0]` is the direct child of the page
    /// root, the last entry is the source block's immediate parent.
    /// Empty when the source block sits at page-root level.
    ///
    /// This is the breadcrumb every client renders as dimmed context
    /// above the citing block, so a reference buried inside a nested
    /// outline still reads with the branch it belongs to. The page
    /// root itself is **not** included — clients already show it as the
    /// group header (the page title).
    pub ancestors: Vec<BacklinkCrumb>,
    /// On-disk path of the source page's `.md`. Derived from the
    /// workspace root passed to [`backlinks_for_page`] / [`backlinks_for_target`].
    /// `None` when the block has no enclosing page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
}

/// One ancestor step in a backlink's breadcrumb.
///
/// Plain text only (no inline tokens): the breadcrumb is dimmed
/// context, not an interactive surface, so clients render it as a
/// muted trail rather than re-rendering links/bold the way they do for
/// the citing block itself. `text` already has the `TODO `/`DONE `
/// prefix stripped, mirroring [`Backlink::block_text`]. `id` lets a
/// client make a crumb tappable (jump to that ancestor) later without
/// a shape change.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinkCrumb {
    /// Node id of the ancestor block.
    pub id: String,
    /// Ancestor block's text, TODO/DONE prefix stripped, single line.
    pub text: String,
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

/// Every block in the workspace that mentions `target` — either as a
/// literal `[[target]]` substring or as a `#tag` token whose slug
/// form equals `target`'s slug form.
///
/// `[[target]]` is matched literally — pass the page's slug AND title
/// separately if you want to catch both forms. Tags go through
/// `outl_md::slug::slugify` on both sides, mirroring how a tag click
/// resolves its page (`open_or_create_by_name`), so `#Avelino` counts
/// as a mention of the page whose slug is `avelino`. `root` is the
/// workspace root directory; it's needed so each backlink can carry
/// its `source_path` (the `.md` of the page the source block lives
/// in).
pub fn backlinks_for_target(workspace: &Workspace, root: &Path, target: &str) -> Vec<Backlink> {
    let index = build_children_index(workspace);
    collect_backlinks(workspace, root, &TargetMatcher::refs(target), &index)
}

/// Build a `parent -> children (in fractional order)` map once per
/// backlinks pass (one scan + per-parent sort; `O(n log n)` worst case).
///
/// The walk visits every block in the workspace and, for each match,
/// materialises its subtree. Both steps used to go through
/// [`children_of`], which **rescans every node** on each call (`Tree`
/// stores only `node -> (parent, position)`, with no child index) —
/// making the whole pass `O(n²)`. Building this map once and threading
/// it through the walk + [`project_outline_node_indexed`] eliminates the
/// `O(n²)` rescans (the remaining cost is one full walk + sorting).
/// Rebuilt per `backlinks_for_page` call, so it never goes stale: it is a
/// scratch accelerator, not cached state.
fn build_children_index(workspace: &Workspace) -> ChildrenIndex {
    let mut grouped: HashMap<NodeId, Vec<(NodeId, Fractional)>> = HashMap::new();
    for (id, parent, pos) in workspace.tree().iter_nodes() {
        grouped.entry(parent).or_default().push((id, pos.clone()));
    }
    grouped
        .into_iter()
        .map(|(parent, mut kids)| {
            kids.sort_by(|a, b| a.1.cmp(&b.1));
            (parent, kids.into_iter().map(|(id, _)| id).collect())
        })
        .collect()
}

/// Walk every page's blocks and collect the ones `matcher` accepts.
/// `index` is the shared `parent -> children` map (see
/// [`build_children_index`]); pages are the children of
/// [`NodeId::root`].
fn collect_backlinks(
    workspace: &Workspace,
    root: &Path,
    matcher: &TargetMatcher,
    index: &ChildrenIndex,
) -> Vec<Backlink> {
    let mut out: Vec<Backlink> = Vec::new();
    let Some(pages) = index.get(&NodeId::root()) else {
        return out;
    };
    for &page_id in pages {
        let Some(meta) = page_meta(workspace, page_id) else {
            continue;
        };
        let source_path = page_md_path(root, &meta);
        let mut path: Vec<usize> = Vec::new();
        let mut ancestors: Vec<BacklinkCrumb> = Vec::new();
        walk_inside_page(
            workspace,
            page_id,
            &meta,
            &source_path,
            &mut path,
            &mut ancestors,
            matcher,
            &mut out,
            index,
        );
    }
    out
}

/// Pre-computed match state for one backlink target.
///
/// A block counts as a backlink when it matches any enabled channel:
/// the literal `[[target]]` needle, a `#tag` whose slug form equals
/// the target's, a ` ```call:<name> ` fence (callable-template render
/// site), or a `from-template:: <slug>` property (structural-template
/// instance). The last two are how a template page surfaces where it
/// was used without the user hand-writing a `[[link]]`.
struct TargetMatcher {
    /// `[[target]]` literal; empty disables the ref channel.
    needle: String,
    /// Slug form for `#tag` comparison; `None` disables the tag channel.
    tag_slug: Option<String>,
    /// Template invocation name for `call:<name>` fences; `None`
    /// disables the callable channel.
    call_name: Option<String>,
    /// Page slug matched against `from-template::`; `None` disables the
    /// structural-instance channel.
    provenance_slug: Option<String>,
}

impl TargetMatcher {
    /// The ordinary text-reference matcher: `[[target]]` + `#tag`.
    fn refs(target: &str) -> Self {
        Self {
            needle: format!("[[{target}]]"),
            tag_slug: Some(outl_md::slug::slugify(target)),
            call_name: None,
            provenance_slug: None,
        }
    }

    /// The template-provenance matcher for a template page: a
    /// `call:<name>` fence or a `from-template:: <slug>` instance.
    fn template(slug: &str, name: &str) -> Self {
        Self {
            needle: String::new(),
            tag_slug: None,
            call_name: Some(name.to_string()),
            provenance_slug: Some(slug.to_string()),
        }
    }

    /// Does this block mention the target on any enabled channel?
    /// `[[ref]]` is a substring probe (cheap, exact thanks to the `]]`
    /// terminator); `#tag` goes through the real inline tokenizer so
    /// tags inside code spans don't count and `#avelino-foo` doesn't
    /// false-match `avelino`.
    fn matches(&self, workspace: &Workspace, block_id: NodeId, text: &str) -> bool {
        if !self.needle.is_empty() && text.contains(&self.needle) {
            return true;
        }
        if let Some(name) = &self.call_name {
            if crate::template::call_target_name(text).as_deref() == Some(name.as_str()) {
                return true;
            }
        }
        if let Some(slug) = &self.provenance_slug {
            let from = crate::page::read_text_prop(
                workspace,
                block_id,
                crate::template::FROM_TEMPLATE_KEY,
            );
            if from.as_deref() == Some(slug.as_str()) {
                return true;
            }
        }
        if let Some(want) = &self.tag_slug {
            if text.contains('#') {
                return outl_md::inline::tokenize(text).iter().any(|tok| match tok {
                    outl_md::inline::InlineTok::Tag { name } => {
                        outl_md::slug::slugify(name) == *want
                    }
                    _ => false,
                });
            }
        }
        false
    }
}

/// The template invocation name of `meta`'s page, when it is a template
/// (has a non-empty `template::` property).
fn template_name_of(workspace: &Workspace, meta: &PageMeta) -> Option<String> {
    let id = crate::page::find_by_slug(workspace, &meta.slug)?;
    let name = crate::page::read_text_prop(workspace, id, crate::template::TEMPLATE_KEY)?;
    (!name.trim().is_empty()).then_some(name)
}

/// Convenience: backlinks against either the page's slug or its title.
///
/// When the page is a person (`page_type == Some("person")`), we also
/// scan for the **`@`-prefixed** alias forms (`@<slug>` and
/// `@<title>`). Person pages don't carry the `@` in their slug or
/// title — the `@` is purely the mention affordance produced by the
/// `@` autocomplete (`[[@avelino]]` resolves to the page `avelino`).
/// Without scanning the alias, every mention of `@avelino` would fall
/// off the person's backlinks panel.
pub fn backlinks_for_page(workspace: &Workspace, root: &Path, meta: &PageMeta) -> Vec<Backlink> {
    // Build the `parent -> children` index once and reuse it across every
    // channel below, so a template page (up to four scans) still walks the
    // tree a single logical time instead of rebuilding the accelerator per
    // target.
    let index = build_children_index(workspace);
    let mut acc: Vec<Backlink> = Vec::new();
    let mut add = |matcher: TargetMatcher| {
        for link in collect_backlinks(workspace, root, &matcher, &index) {
            if !acc.iter().any(|l| l.block_id == link.block_id) {
                acc.push(link);
            }
        }
    };

    add(TargetMatcher::refs(&meta.slug));
    if meta.title != meta.slug {
        add(TargetMatcher::refs(&meta.title));
    }
    if meta.page_type.as_deref() == Some(crate::person::PERSON_TYPE) {
        add(TargetMatcher::refs(&format!("@{}", meta.slug)));
        if meta.title != meta.slug {
            add(TargetMatcher::refs(&format!("@{}", meta.title)));
        }
    }
    // A template page also surfaces where it was rendered (`call:<name>`
    // fences) or instantiated (`from-template:: <slug>`) — neither is a
    // `[[ref]]`, so the scans above miss them.
    if let Some(name) = template_name_of(workspace, meta) {
        add(TargetMatcher::template(&meta.slug, &name));
    }
    acc
}

/// Recursive helper: descend into `parent`'s children, tracking the
/// DFS path. Every block whose text matches `matcher` becomes a
/// [`Backlink`] in `out`.
#[allow(clippy::too_many_arguments)]
fn walk_inside_page(
    workspace: &Workspace,
    parent: NodeId,
    meta: &PageMeta,
    source_path: &Path,
    path: &mut Vec<usize>,
    ancestors: &mut Vec<BacklinkCrumb>,
    matcher: &TargetMatcher,
    out: &mut Vec<Backlink>,
    index: &ChildrenIndex,
) {
    let Some(children) = index.get(&parent) else {
        return;
    };
    for (idx, child_id) in children.iter().copied().enumerate() {
        path.push(idx);
        let text = workspace.block_text(child_id).unwrap_or_default();
        let (todo, body) = split_todo(&text);
        if matcher.matches(workspace, child_id, &text) {
            let source_block = project_outline_node_indexed(workspace, child_id, index);
            out.push(Backlink {
                block_id: child_id.to_string(),
                block_text: body.to_string(),
                todo,
                source_page: Some(meta.clone()),
                source_block,
                source_block_path: path.clone(),
                // `ancestors` holds the chain down to `parent` — i.e.
                // exactly this block's ancestors, excluding the block
                // itself (pushed only before recursing below).
                ancestors: ancestors.clone(),
                source_path: Some(source_path.to_path_buf()),
            });
        }
        // Descend: this block becomes an ancestor of its descendants.
        ancestors.push(BacklinkCrumb {
            id: child_id.to_string(),
            text: body.to_string(),
        });
        walk_inside_page(
            workspace,
            child_id,
            meta,
            source_path,
            path,
            ancestors,
            matcher,
            out,
            index,
        );
        ancestors.pop();
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
    use crate::page::{find_by_slug, open_journal, open_or_create, PageKind};
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

    /// Build a template page named `name` with a single code block, so
    /// it can be both instantiated (structural) and called (callable).
    fn make_template(workspace: &mut Workspace, hlc: &HlcGenerator, slug: &str, name: &str) {
        use crate::page::set_property;
        use crate::template::TEMPLATE_KEY;
        use outl_core::property::PropValue;

        let id = open_or_create(workspace, hlc, slug, slug, PageKind::Page).unwrap();
        set_property(
            workspace,
            hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text(name.into())),
        )
        .unwrap();
        append_block(workspace, hlc, Some(id), Some("- **Item:**")).unwrap();
    }

    #[test]
    fn structural_instance_shows_in_template_backlinks() {
        // Instantiate a template into a journal, then the template
        // page's backlinks must list where it was stamped — the
        // `from-template::` property is not a `[[ref]]`, so this only
        // works because the matcher reads the property directly.
        let (mut w, hlc) = ws();
        make_template(&mut w, &hlc, "template-1on1", "1on1");
        let j = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 9).unwrap()).unwrap();
        let target = append_block(&mut w, &hlc, Some(j), Some("host")).unwrap();
        crate::template::instantiate_template(&mut w, &hlc, "1on1", target, "2026-07-09", None)
            .unwrap();

        let meta = page_meta(&w, find_by_slug(&w, "template-1on1").unwrap()).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(
            !bl.is_empty(),
            "structural instance should appear in the template's backlinks"
        );
        assert!(bl.iter().any(|b| b
            .source_page
            .as_ref()
            .is_some_and(|p| p.slug == "2026-07-09")));
    }

    #[test]
    fn callable_site_shows_in_template_backlinks() {
        // A `call:<name>` fence must surface in the template page's
        // backlinks without a hand-written `[[link]]`.
        let (mut w, hlc) = ws();
        make_template(&mut w, &hlc, "template-calc", "calc");
        let j = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 9).unwrap()).unwrap();
        append_block(&mut w, &hlc, Some(j), Some("```call:calc\nx: 1\n```")).unwrap();

        let meta = page_meta(&w, find_by_slug(&w, "template-calc").unwrap()).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(
            bl.iter().any(|b| b
                .source_page
                .as_ref()
                .is_some_and(|p| p.slug == "2026-07-09")),
            "callable site should appear in the template's backlinks"
        );
    }

    #[test]
    fn non_template_page_ignores_call_and_provenance() {
        // A regular page must not accidentally pull in call/provenance
        // matches — the template channel only fires for template pages.
        let (mut w, hlc) = ws();
        let p = open_or_create(&mut w, &hlc, "regular", "regular", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(p), Some("```call:regular\n```")).unwrap();

        let meta = page_meta(&w, p).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(bl.is_empty(), "regular page has no template channel");
    }

    #[test]
    fn tag_mentions_count_as_backlinks() {
        // `#avelino` must surface in the backlinks of the `avelino`
        // page exactly like `[[avelino]]` would — a tag click and a
        // ref click open the same page, so the "Linked from" panel
        // has to agree.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("pairing with #avelino today")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
    }

    #[test]
    fn tag_mentions_match_through_slugify() {
        // A tag click resolves its page via `slugify` (see
        // `open_or_create_by_name`), so `#Avelino` is a mention of
        // the page whose slug is `avelino`.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention = append_block(&mut w, &hlc, Some(day), Some("ship it #Avelino")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
    }

    #[test]
    fn longer_tag_does_not_false_match_a_prefix_target() {
        // `#avelino-foo` is a different page; it must NOT appear in
        // `avelino`'s backlinks. A substring probe on `#avelino`
        // would get this wrong — the tokenizer-based match doesn't.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("see #avelino-foo instead")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "prefix tag leaked in: {links:#?}");
    }

    #[test]
    fn tag_inside_inline_code_is_not_a_mention() {
        // `` `#avelino` `` is code, not a tag — the inline tokenizer
        // already knows that; the backlink matcher must respect it.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("escape it as `#avelino`")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "code-span tag leaked in: {links:#?}");
    }

    #[test]
    fn block_with_ref_and_tag_emits_one_backlink() {
        // Mentioning the same page via both forms in one block still
        // produces a single backlink (matcher is a yes/no probe and
        // `backlinks_for_page` dedupes by block id).
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] aka #avelino")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
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

    #[test]
    fn person_page_picks_up_at_alias_mentions() {
        use crate::page::{page_meta, set_property};
        use crate::person::{PERSON_TYPE, TYPE_KEY};
        use outl_core::property::PropValue;
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        set_property(
            &mut w,
            &hlc,
            avelino,
            TYPE_KEY,
            Some(PropValue::Text(PERSON_TYPE.into())),
        )
        .unwrap();
        let meta = page_meta(&w, avelino).unwrap();

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        // The `@` autocomplete inserts `[[@avelino]]` — a normal
        // wikilink whose target carries the `@`. The person's backlinks
        // panel must surface it even though the page slug is `avelino`.
        let at_mention =
            append_block(&mut w, &hlc, Some(day), Some("blocked on [[@avelino]]")).unwrap();
        let plain_mention =
            append_block(&mut w, &hlc, Some(day), Some("talked to [[avelino]] today")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        let block_ids: Vec<String> = links.iter().map(|l| l.block_id.clone()).collect();
        assert!(
            block_ids.contains(&at_mention.to_string()),
            "@-mention not surfaced in backlinks"
        );
        assert!(
            block_ids.contains(&plain_mention.to_string()),
            "plain [[avelino]] mention not surfaced"
        );
        // Plain pages (non-person) must NOT scan the `@` alias.
        let other = open_or_create(&mut w, &hlc, "projeto", "Projeto", PageKind::Page).unwrap();
        let projeto_meta = page_meta(&w, other).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("worked on [[@projeto]]")).unwrap();
        let projeto_links = backlinks_for_page(&w, root(), &projeto_meta);
        assert!(
            projeto_links.is_empty(),
            "non-person page must not match `@`-aliased refs"
        );
    }
}
