//! Workspace-wide derived index.
//!
//! Walks the `pages/` and `journals/` directories, parses each `.md`,
//! and builds in-memory maps the TUI / GUI / mobile can query without
//! re-walking the filesystem:
//!
//! - `slug → PageEntry` (filename without `.md`).
//! - `title → slug` (the `title::` property; falls back to slug).
//! - `slug → Vec<Backlink>` (every block that contains `[[name]]` or
//!   `#name` where `slugify(name) == this`).
//!
//! Rebuild on demand: this is cheap for hundreds of pages, expensive
//! for thousands. The TUI calls `rebuild()` at startup and on a debounce
//! after writes.

use crate::inline::{tokenize, InlineTok};
use crate::parse::{parse, OutlineNode};
use crate::slug::slugify;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One entry in the index — the data we want to know about a page
/// without re-reading its file.
#[derive(Debug, Clone)]
pub struct PageEntry {
    /// Filesystem path, e.g. `pages/avelino.md`.
    pub path: PathBuf,
    /// Slug (filename without extension).
    pub slug: String,
    /// User-visible title (from `title::` property, or `slug` if unset).
    pub title: String,
    /// Optional decoration from the `icon::` property — usually a
    /// single emoji or short string the UI prepends to the title.
    /// `None` when the page has no `icon::` set.
    pub icon: Option<String>,
    /// Whether the file lives in `journals/`.
    pub is_journal: bool,
}

/// One backlink — a block in another page that references this slug.
///
/// Carries the full source `OutlineNode` (including its children
/// subtree) so the UI can render the referencing block in context —
/// not just as a truncated snippet. Editing surfaces resolve the
/// target block by descending into `source_block` along an extra
/// sub-path relative to `source_block_path`.
#[derive(Debug, Clone)]
pub struct Backlink {
    /// Slug of the page containing the reference.
    pub source_slug: String,
    /// Title of the source page.
    pub source_title: String,
    /// Icon of the source page (if any) — propagated so backlink
    /// surfaces can render the same `<icon> <title>` shape every other
    /// surface uses.
    pub source_icon: Option<String>,
    /// Filesystem path of the source.
    pub source_path: PathBuf,
    /// DFS path of the referencing block inside the source page's AST.
    /// Combined with a sub-path inside `source_block`, this lets the
    /// TUI/editor locate the exact node to mutate when the user edits
    /// a backlink in place.
    pub source_block_path: Vec<usize>,
    /// The referencing `OutlineNode` itself, including children.
    /// Cloned so backlink consumers don't need to re-read the source
    /// page from disk to render context.
    pub source_block: OutlineNode,
}

/// Full workspace index.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceIndex {
    pages: HashMap<String, PageEntry>,
    title_to_slug: HashMap<String, String>,
    backlinks: HashMap<String, Vec<Backlink>>,
}

impl WorkspaceIndex {
    /// Walk `pages/` and `journals/` under `workspace_root`, parse every
    /// `.md`, and return the populated index. Files that fail to parse
    /// are skipped with no error — the index is best-effort.
    ///
    /// Two logical passes (pages metadata first, then backlinks) but
    /// only **one read+parse per file**: the parsed AST is held in a
    /// buffer between passes. Halves the I/O + parsing cost vs the
    /// naive two-pass implementation; verified in `benches/index.rs`.
    pub fn build(workspace_root: &Path) -> Self {
        let mut idx = WorkspaceIndex::default();
        // Buffer of (slug, parsed AST). Populated in pass 1 alongside
        // the `pages` map; consumed in pass 2 for backlink collection.
        // Capacity-hint avoids regrowth on workspaces of any reasonable
        // size.
        let mut parsed_pages: Vec<(String, crate::parse::ParsedPage)> = Vec::with_capacity(64);

        for (dir, is_journal) in [
            (workspace_root.join("pages"), false),
            (workspace_root.join("journals"), true),
        ] {
            if !dir.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&dir).max_depth(1) {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("md") {
                    continue;
                }
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                let parsed = parse(&text);
                let title = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "title")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| slug.to_string());
                let icon = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "icon")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty());

                idx.pages.insert(
                    slug.to_string(),
                    PageEntry {
                        path: path.to_path_buf(),
                        slug: slug.to_string(),
                        title: title.clone(),
                        icon,
                        is_journal,
                    },
                );
                idx.title_to_slug.insert(title.clone(), slug.to_string());
                parsed_pages.push((slug.to_string(), parsed));
            }
        }

        // Second pass: scan blocks for `[[ref]]` and `#tag`, populate
        // backlinks. Reuses the AST cached in `parsed_pages` so we
        // don't pay another read + parse round-trip.
        for (slug, parsed) in &parsed_pages {
            // Clone is cheap — `PageEntry` is small and `Arc`-less.
            // Avoids holding an immutable borrow of `idx.pages` while
            // we mutate `idx.backlinks` below.
            let Some(entry) = idx.pages.get(slug).cloned() else {
                continue;
            };
            let mut path_stack: Vec<usize> = Vec::new();
            collect_backlinks_recursive(&parsed.blocks, &mut path_stack, &entry, &mut idx);
        }

        idx
    }

    /// Look up a page by its slug.
    pub fn by_slug(&self, slug: &str) -> Option<&PageEntry> {
        self.pages.get(slug)
    }

    /// Look up a page by its `title::` (or slug fallback). Title match
    /// is case-sensitive — use `pages_by_title_prefix` for autocomplete.
    pub fn by_title(&self, title: &str) -> Option<&PageEntry> {
        let slug = self.title_to_slug.get(title)?;
        self.pages.get(slug)
    }

    /// Iterate every page entry in unspecified order.
    pub fn pages(&self) -> impl Iterator<Item = &PageEntry> {
        self.pages.values()
    }

    /// Number of pages indexed.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Backlinks pointing at a given slug. The returned slice may be
    /// empty.
    pub fn backlinks(&self, slug: &str) -> &[Backlink] {
        self.backlinks
            .get(slug)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Re-index a single page in place — replacement for the global
    /// scan when only one page changed.
    ///
    /// Removes every backlink whose source was this slug, walks the
    /// new AST to emit fresh backlinks, and refreshes the `PageEntry`
    /// metadata (title/icon). `is_journal` is inferred from the
    /// parent directory of `path` (`journals/...` vs anything else).
    ///
    /// Roughly `O(blocks_in_this_page)` — orders of magnitude cheaper
    /// than `build`, which walks every `.md` in the workspace. Use
    /// this on every page save so the index stays fresh without
    /// paying for a full rescan.
    pub fn patch_page(&mut self, path: &Path, page: &crate::parse::ParsedPage) {
        let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
            return;
        };
        let slug = slug.to_string();
        let is_journal = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("journals");

        let title = page
            .properties
            .iter()
            .find(|(k, _)| k == "title")
            .map(|(_, v)| v.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slug.clone());
        let icon = page
            .properties
            .iter()
            .find(|(k, _)| k == "icon")
            .map(|(_, v)| v.trim().to_string())
            .filter(|s| !s.is_empty());

        // Drop every backlink that used to come from this page —
        // we'll re-emit the fresh set below.
        for list in self.backlinks.values_mut() {
            list.retain(|bl| bl.source_slug != slug);
        }
        self.backlinks.retain(|_, v| !v.is_empty());

        // Forget the page's previous `title -> slug` mapping in case
        // the title changed (otherwise a stale alias would shadow the
        // new one for `by_title` lookups).
        self.title_to_slug.retain(|_, s| s != &slug);

        let entry = PageEntry {
            path: path.to_path_buf(),
            slug: slug.clone(),
            title: title.clone(),
            icon,
            is_journal,
        };
        self.pages.insert(slug.clone(), entry.clone());
        self.title_to_slug.insert(title, slug);

        let mut path_stack: Vec<usize> = Vec::new();
        collect_backlinks_recursive(&page.blocks, &mut path_stack, &entry, self);
    }

    /// Drop a page from the index entirely. Use when a `.md` is
    /// deleted on disk. Removes the `PageEntry`, its title alias, and
    /// every backlink whose source was this slug.
    pub fn remove_page(&mut self, slug: &str) {
        self.pages.remove(slug);
        self.title_to_slug.retain(|_, s| s != slug);
        for list in self.backlinks.values_mut() {
            list.retain(|bl| bl.source_slug != slug);
        }
        self.backlinks.retain(|_, v| !v.is_empty());
    }

    /// Re-clone the cached `source_block` of every backlink whose
    /// `source_path` matches, pulling fresh content from `source_page`.
    ///
    /// Used as an **optimistic** refresh after the TUI mutates the
    /// source page in memory (e.g. structural ops triggered from
    /// inside a backlink). Lets the next frame show the new tree
    /// without paying for a full workspace rebuild. The next natural
    /// rebuild reconverges with disk truth.
    ///
    /// Backlinks whose `source_block_path` no longer resolves (e.g.
    /// the referencing block was moved or deleted) keep their stale
    /// node — the canonical fix happens when the index rebuilds.
    pub fn refresh_backlinks_from_source(
        &mut self,
        source_path: &Path,
        source_page: &crate::parse::ParsedPage,
    ) {
        for list in self.backlinks.values_mut() {
            for bl in list.iter_mut() {
                if bl.source_path != source_path {
                    continue;
                }
                if let Some(node) = walk_node(&source_page.blocks, &bl.source_block_path) {
                    bl.source_block = node.clone();
                }
            }
        }
    }

    /// Apply `new_text` to every cached `source_block` whose `(source_path,
    /// absolute_block_path)` matches `(source_path, target_path)`.
    ///
    /// `target_path` is interpreted as the DFS path inside the source
    /// page's AST — i.e. `source_block_path` (where the referencing
    /// block lives) concatenated with the sub-path *inside* that block
    /// (zero or more steps).
    ///
    /// This is an **optimistic** in-memory patch used by the TUI to
    /// reflect a backlink edit instantly while the actual disk write
    /// and reconcile happen out of the critical path. The next full
    /// index rebuild reconverges this with disk truth — until then,
    /// every backlink list that references the same source block
    /// stays consistent because they all carry their own clone of the
    /// node, and this method walks them all.
    pub fn patch_backlink_text(
        &mut self,
        source_path: &Path,
        target_path: &[usize],
        new_text: &str,
    ) {
        for list in self.backlinks.values_mut() {
            for bl in list.iter_mut() {
                if bl.source_path != source_path {
                    continue;
                }
                if !target_path.starts_with(&bl.source_block_path) {
                    continue;
                }
                let tail = &target_path[bl.source_block_path.len()..];
                if let Some(node) = walk_node_mut(&mut bl.source_block, tail) {
                    node.text = new_text.to_string();
                }
            }
        }
    }

    /// Titles starting with `prefix` (case-insensitive), best-effort
    /// for autocomplete. Returns at most `limit` results sorted by
    /// title length (shorter first).
    pub fn pages_by_title_prefix(&self, prefix: &str, limit: usize) -> Vec<&PageEntry> {
        let needle = prefix.to_lowercase();
        let mut hits: Vec<&PageEntry> = self
            .pages
            .values()
            .filter(|p| p.title.to_lowercase().starts_with(&needle))
            .collect();
        hits.sort_by_key(|p| (p.title.len(), p.title.clone()));
        hits.truncate(limit);
        hits
    }
}

fn walk_node_mut<'a>(root: &'a mut OutlineNode, path: &[usize]) -> Option<&'a mut OutlineNode> {
    let mut node = root;
    for &i in path {
        node = node.children.get_mut(i)?;
    }
    Some(node)
}

fn walk_node<'a>(blocks: &'a [OutlineNode], path: &[usize]) -> Option<&'a OutlineNode> {
    let mut current = blocks;
    let mut node: Option<&OutlineNode> = None;
    for &i in path {
        let n = current.get(i)?;
        node = Some(n);
        current = &n.children;
    }
    node
}

fn collect_backlinks_recursive(
    blocks: &[OutlineNode],
    path_stack: &mut Vec<usize>,
    source: &PageEntry,
    idx: &mut WorkspaceIndex,
) {
    for (i, b) in blocks.iter().enumerate() {
        path_stack.push(i);
        // A block may reference several pages/tags — but we record one
        // backlink per *unique* target slug so a page referenced twice
        // in the same block doesn't show up duplicated.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for tok in tokenize(&b.text) {
            let target_slug = match tok {
                InlineTok::PageRef { name } | InlineTok::Tag { name } => slugify(name),
                _ => continue,
            };
            if seen.insert(target_slug.clone()) {
                push_backlink(idx, &target_slug, source, path_stack, b);
            }
        }
        collect_backlinks_recursive(&b.children, path_stack, source, idx);
        path_stack.pop();
    }
}

fn push_backlink(
    idx: &mut WorkspaceIndex,
    target_slug: &str,
    source: &PageEntry,
    source_block_path: &[usize],
    source_block: &OutlineNode,
) {
    // Skip self-references: a page linking to itself is noise.
    if target_slug == source.slug {
        return;
    }
    idx.backlinks
        .entry(target_slug.to_string())
        .or_default()
        .push(Backlink {
            source_slug: source.slug.clone(),
            source_title: source.title.clone(),
            source_icon: source.icon.clone(),
            source_path: source.path.clone(),
            source_block_path: source_block_path.to_vec(),
            source_block: source_block.clone(),
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_workspace(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, content) in files {
            let full = dir.path().join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        dir
    }

    #[test]
    fn patch_page_replaces_backlinks_for_that_slug_only() {
        // Initial state: projeto.md and journal both reference Avelino.
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- led by [[Avelino]]\n",
            ),
            ("journals/2026-05-24.md", "- meeting with [[Avelino]]\n"),
        ]);
        let mut idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.backlinks("avelino").len(), 2);

        // User rewrites projeto.md so it no longer references Avelino
        // and now references "Other Page" instead. patch_page must:
        //   1. drop projeto's old backlink to Avelino
        //   2. emit a new backlink from projeto to other-page
        // ...without touching the journal's backlink.
        let new_md = "title:: Projeto\n\n- led by [[Other Page]]\n";
        let new_page = crate::parse::parse(new_md);
        let proj_path = dir.path().join("pages/projeto.md");
        idx.patch_page(&proj_path, &new_page);

        let avelino_bls = idx.backlinks("avelino");
        assert_eq!(avelino_bls.len(), 1, "journal backlink should survive");
        assert_eq!(avelino_bls[0].source_slug, "2026-05-24");

        let other_bls = idx.backlinks("other-page");
        assert_eq!(other_bls.len(), 1);
        assert_eq!(other_bls[0].source_slug, "projeto");
    }

    #[test]
    fn patch_page_updates_title_and_icon() {
        let dir = write_workspace(&[("pages/x.md", "title:: Old Title\nicon:: 🦀\n\n- body\n")]);
        let mut idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.by_slug("x").unwrap().title, "Old Title");
        assert_eq!(idx.by_slug("x").unwrap().icon.as_deref(), Some("🦀"));

        let new_page = crate::parse::parse("title:: New Title\nicon:: 🚀\n\n- body\n");
        idx.patch_page(&dir.path().join("pages/x.md"), &new_page);

        let entry = idx.by_slug("x").unwrap();
        assert_eq!(entry.title, "New Title");
        assert_eq!(entry.icon.as_deref(), Some("🚀"));
        // by_title should follow the new title and forget the old one.
        assert!(idx.by_title("Old Title").is_none());
        assert_eq!(idx.by_title("New Title").unwrap().slug, "x");
    }

    #[test]
    fn remove_page_drops_entry_and_its_backlinks() {
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- led by [[Avelino]]\n",
            ),
        ]);
        let mut idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.backlinks("avelino").len(), 1);

        idx.remove_page("projeto");
        assert!(idx.by_slug("projeto").is_none());
        assert!(idx.by_title("Projeto").is_none());
        assert!(idx.backlinks("avelino").is_empty());
    }

    #[test]
    fn empty_workspace_indexes_to_nothing() {
        let dir = TempDir::new().unwrap();
        let idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.page_count(), 0);
    }

    #[test]
    fn pages_get_indexed_by_slug_and_title() {
        let dir = write_workspace(&[
            (
                "pages/avelino.md",
                "title:: Avelino\n\n- some note about me\n",
            ),
            ("pages/projeto.md", "title:: Meu Projeto\n\n- objetivo\n"),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.page_count(), 2);
        assert_eq!(idx.by_slug("avelino").unwrap().title, "Avelino");
        assert_eq!(idx.by_title("Meu Projeto").unwrap().slug, "projeto");
    }

    #[test]
    fn missing_title_falls_back_to_slug() {
        let dir = write_workspace(&[("pages/no-title.md", "- bare bullet\n")]);
        let idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.by_slug("no-title").unwrap().title, "no-title");
    }

    #[test]
    fn icon_property_is_indexed_and_propagated_to_backlinks() {
        let dir = write_workspace(&[
            (
                "pages/avelino.md",
                "title:: Avelino\nicon:: 🦀\n\n- author\n",
            ),
            (
                "pages/projeto.md",
                "title:: Projeto\nicon:: 🚀\n\n- led by [[Avelino]]\n",
            ),
            // Page without icon — must produce None, not crash.
            ("pages/bare.md", "title:: Bare\n\n- nothing fancy\n"),
        ]);
        let idx = WorkspaceIndex::build(dir.path());

        assert_eq!(idx.by_slug("avelino").unwrap().icon.as_deref(), Some("🦀"));
        assert_eq!(idx.by_slug("projeto").unwrap().icon.as_deref(), Some("🚀"));
        assert_eq!(idx.by_slug("bare").unwrap().icon, None);

        // Backlink to Avelino comes from Projeto — must carry its icon.
        let bls = idx.backlinks("avelino");
        assert_eq!(bls.len(), 1);
        assert_eq!(bls[0].source_slug, "projeto");
        assert_eq!(bls[0].source_icon.as_deref(), Some("🚀"));
    }

    #[test]
    fn empty_icon_is_treated_as_none() {
        // `icon:: ` (no value) shouldn't show up as a present-but-empty
        // icon — the UI would render a stray space.
        let dir = write_workspace(&[("pages/x.md", "title:: X\nicon::\n\n- body\n")]);
        let idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.by_slug("x").unwrap().icon, None);
    }

    #[test]
    fn backlinks_are_collected_across_pages() {
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- I am the author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- led by [[Avelino]]\n",
            ),
            (
                "journals/2026-05-24.md",
                "- meeting with [[Avelino]] and #urgent stuff\n",
            ),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        let bl = idx.backlinks("avelino");
        assert_eq!(bl.len(), 2);
        let slugs: Vec<_> = bl.iter().map(|b| b.source_slug.as_str()).collect();
        assert!(slugs.contains(&"projeto"));
        assert!(slugs.contains(&"2026-05-24"));

        let urgent = idx.backlinks("urgent");
        assert_eq!(urgent.len(), 1);
    }

    #[test]
    fn self_references_are_skipped() {
        let dir = write_workspace(&[(
            "pages/recursive.md",
            "title:: Recursive\n\n- I link to [[Recursive]] myself\n",
        )]);
        let idx = WorkspaceIndex::build(dir.path());
        assert!(idx.backlinks("recursive").is_empty());
    }

    #[test]
    fn journals_are_treated_as_pages_for_lookup() {
        let dir = write_workspace(&[("journals/2026-05-24.md", "- entry\n")]);
        let idx = WorkspaceIndex::build(dir.path());
        let entry = idx.by_slug("2026-05-24").unwrap();
        assert!(entry.is_journal);
    }

    #[test]
    fn source_block_carries_text_and_children() {
        // The Backlink struct holds the full referencing block, not a
        // truncated snippet. The TUI relies on `source_block.children`
        // to draw nested context.
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- led by [[Avelino]]\n  - milestone A\n  - milestone B\n",
            ),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        let bls = idx.backlinks("avelino");
        assert_eq!(bls.len(), 1);
        assert_eq!(bls[0].source_block.text, "led by [[Avelino]]");
        assert_eq!(bls[0].source_block.children.len(), 2);
        assert_eq!(bls[0].source_block.children[0].text, "milestone A");
        assert_eq!(bls[0].source_block.children[1].text, "milestone B");
    }

    #[test]
    fn source_block_path_points_to_referencing_block() {
        // A backlink coming from a nested block records the DFS path
        // [parent_idx, child_idx] so the editor can navigate straight
        // to the right node without re-walking the AST.
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- root block\n  - nested ref to [[Avelino]]\n",
            ),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        let bls = idx.backlinks("avelino");
        assert_eq!(bls.len(), 1);
        assert_eq!(bls[0].source_block_path, vec![0, 0]);
        assert_eq!(bls[0].source_block.text, "nested ref to [[Avelino]]");
    }

    #[test]
    fn block_with_repeated_reference_only_emits_one_backlink() {
        let dir = write_workspace(&[
            ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
            (
                "pages/projeto.md",
                "title:: Projeto\n\n- [[Avelino]] and again [[Avelino]] same block\n",
            ),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        assert_eq!(idx.backlinks("avelino").len(), 1);
    }

    #[test]
    fn title_prefix_lookup() {
        let dir = write_workspace(&[
            ("pages/a.md", "title:: Apple\n\n- a\n"),
            ("pages/b.md", "title:: Apricot\n\n- a\n"),
            ("pages/c.md", "title:: Banana\n\n- a\n"),
        ]);
        let idx = WorkspaceIndex::build(dir.path());
        let hits = idx.pages_by_title_prefix("Ap", 10);
        assert_eq!(hits.len(), 2);
        let names: Vec<_> = hits.iter().map(|p| p.title.as_str()).collect();
        assert!(names.contains(&"Apple"));
        assert!(names.contains(&"Apricot"));
    }
}
