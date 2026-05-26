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

use crate::block_index::{BlockEntry, BlockIndex, BlockReference};
use crate::inline::{tokenize, InlineTok};
use crate::parse::{parse, OutlineNode};
use crate::sidecar::{self, Sidecar};
use crate::slug::slugify;
use outl_core::id::NodeId;
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
    /// `pinned:: true` page-level property. Surfaces that ship a
    /// sidebar (TUI, future Tauri) list pinned pages prominently so
    /// frequently-touched notes are a single click away.
    pub pinned: bool,
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
    /// Block-level index — owns the `((blk-XXXXXX))` lookup machinery.
    /// Kept private so the public surface stays a single `WorkspaceIndex`.
    blocks: BlockIndex,
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
        let mut parsed_pages: Vec<(String, crate::parse::ParsedPage, Option<Sidecar>)> =
            Vec::with_capacity(64);

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
                let pinned = parsed
                    .properties
                    .iter()
                    .any(|(k, v)| k == "pinned" && is_truthy(v));

                idx.pages.insert(
                    slug.to_string(),
                    PageEntry {
                        path: path.to_path_buf(),
                        slug: slug.to_string(),
                        title: title.clone(),
                        icon,
                        is_journal,
                        pinned,
                    },
                );
                idx.title_to_slug.insert(title.clone(), slug.to_string());

                // Block-level indexing phase 1: register every block
                // (id, handle, text, subtree) without recording
                // reverse refs yet. Phase 2 below scans citations
                // once all handles are known — that way a page B
                // that cites a block of page A still gets its edge
                // registered even when B is walked before A.
                let cached_sidecar = read_sidecar_best_effort(path);
                if let Some(sc) = &cached_sidecar {
                    idx.blocks
                        .collect_page_blocks(slug, path, &parsed.blocks, &sc.blocks);
                }

                parsed_pages.push((slug.to_string(), parsed, cached_sidecar));
            }
        }

        // Phase 2 of block indexing: now that every handle is in
        // `handle_to_block`, scan each page's blocks for
        // `((blk-XXXXXX))` and record the reverse edges.
        for (slug, parsed, cached_sidecar) in &parsed_pages {
            if let Some(sc) = cached_sidecar {
                idx.blocks
                    .collect_page_refs(slug, &parsed.blocks, &sc.blocks);
            }
        }

        // Second pass: scan blocks for `[[ref]]` and `#tag`, populate
        // backlinks. Reuses the AST cached in `parsed_pages` so we
        // don't pay another read + parse round-trip.
        for (slug, parsed, _sc) in &parsed_pages {
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
        let pinned = page
            .properties
            .iter()
            .any(|(k, v)| k == "pinned" && is_truthy(v));

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
            pinned,
        };
        self.pages.insert(slug.clone(), entry.clone());
        self.title_to_slug.insert(title.clone(), slug.clone());

        let mut path_stack: Vec<usize> = Vec::new();
        collect_backlinks_recursive(&page.blocks, &mut path_stack, &entry, self);

        // Block-level re-index: drop everything this page used to
        // contribute, then re-collect from the fresh AST + current
        // sidecar. `forget_page` is O(blocks_in_workspace) today; the
        // bench in #12 measures whether that holds up at scale.
        self.blocks.forget_page(&slug);
        if let Some(sc) = read_sidecar_best_effort(path) {
            self.blocks
                .collect_page(&slug, path, &page.blocks, &sc.blocks);
        }
    }

    /// Drop a page from the index entirely. Use when a `.md` is
    /// deleted on disk. Removes the `PageEntry`, its title alias,
    /// every backlink whose source was this slug, and every block
    /// the page contributed to the block index.
    pub fn remove_page(&mut self, slug: &str) {
        self.pages.remove(slug);
        self.title_to_slug.retain(|_, s| s != slug);
        for list in self.backlinks.values_mut() {
            list.retain(|bl| bl.source_slug != slug);
        }
        self.backlinks.retain(|_, v| !v.is_empty());
        self.blocks.forget_page(slug);
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

    /// Resolve `((blk-XXXXXX))` to the indexed block, if known.
    ///
    /// `O(1)`. Returns `None` for handles that don't match any
    /// indexed block — orphan references are surfaced this way for
    /// the doctor pass to flag.
    pub fn resolve_block_ref(&self, handle: &str) -> Option<&BlockEntry> {
        self.blocks.resolve(handle)
    }

    /// Look up a block by its `NodeId`.
    pub fn block_by_id(&self, id: NodeId) -> Option<&BlockEntry> {
        self.blocks.get(id)
    }

    /// Blocks that cite `id` via `((blk-XXXXXX))`. May be empty.
    pub fn block_refs_to(&self, id: NodeId) -> &[BlockReference] {
        self.blocks.refs_to(id)
    }

    /// Iterate every indexed block in unspecified order. Used by
    /// the TUI's `((` autocomplete to fuzzy-match on block text.
    pub fn iter_blocks(&self) -> impl Iterator<Item = &BlockEntry> {
        self.blocks.iter_blocks()
    }

    /// Search indexed blocks by text substring (case-insensitive).
    ///
    /// Powers the `((` autocomplete in any UI surface (TUI today,
    /// Tauri / mobile later). Returns up to `limit` `BlockEntry`s
    /// ranked by match position then text length.
    pub fn search_block_text(&self, query: &str, limit: usize) -> Vec<&BlockEntry> {
        self.blocks.search_text(query, limit)
    }

    /// Find the block at `(slug, dfs_path)` in O(1).
    ///
    /// Backs `yr` / `/refer` / `/refer-embed` so the TUI doesn't
    /// scan `iter_blocks()` linearly per chord press.
    pub fn block_at_location(&self, slug: &str, path: &[usize]) -> Option<&BlockEntry> {
        self.blocks.at_location(slug, path)
    }

    /// Total indexed blocks. Tests and the future bench (#12) read
    /// this to sanity-check workspace coverage.
    pub fn block_count(&self) -> usize {
        self.blocks.block_count()
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

/// Read the sidecar paired with `md_path`, swallowing I/O and parse
/// errors so the index build (a best-effort pass) cannot abort over a
/// single bad file.
fn read_sidecar_best_effort(md_path: &Path) -> Option<Sidecar> {
    let p = sidecar::sidecar_path_for(md_path);
    sidecar::read(&p).ok()
}

/// Loose truthy check for boolean-ish property values (`pinned::`,
/// `archived::`, etc). Accepts `true`, `yes`, `1`, `on` (case
/// insensitive). Empty string is treated as falsy so a stray
/// `pinned:: ` doesn't flip a page into the pinned list.
fn is_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
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

// Integration-level tests for WorkspaceIndex live in
// `tests/workspace_index.rs`. They exercise only the public API surface
// so the same suite would catch a regression introduced by a UI client
// (TUI, future Tauri, mobile).
