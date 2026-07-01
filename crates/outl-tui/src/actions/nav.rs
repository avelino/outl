//! Navigation: between pages, between journals, between blocks, and
//! inside a block's text. Also `[[ref]]` / `#tag` / date-link
//! resolution.
//!
//! Nothing in this file persists — `nav` only mutates `self.view`,
//! `self.selected`, and `self.cursor_col`. The lifecycle module is
//! the one that touches disk.

use crate::outline_ops::{node_at_path, path_for_index};
use crate::state::{App, Focus, Mode, View};
use anyhow::Result;
use chrono::Duration;
use outl_actions::{clock, flatten_subtree_paths};
use outl_md::inline::{ref_at_cursor, RefTarget};
use outl_md::reconcile::reconcile_md;
use std::fs;
use std::path::PathBuf;

impl App {
    pub(crate) fn current_path(&self) -> PathBuf {
        match &self.view {
            View::Journal(date) => self
                .workspace_root
                .join("journals")
                .join(format!("{}.md", date.format("%Y-%m-%d"))),
            View::Page(p) => p.clone(),
        }
    }

    #[allow(dead_code)] // header now uses chrome::breadcrumb; kept for future reuse
    pub(crate) fn current_title(&self) -> String {
        let mode_tag = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert { .. } => "INSERT",
            Mode::Visual { .. } => "VISUAL",
        };
        match &self.view {
            View::Journal(date) => {
                format!("Journal · {} · [{}]", date.format("%A, %Y-%m-%d"), mode_tag)
            }
            View::Page(p) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                // Pull title + icon from the workspace index. Falls
                // back to the slug when the index doesn't know about
                // this page yet (just-created file, before the next
                // rebuild). Title is preferred over slug because it's
                // what the user wrote — `Page · CTO` reads better than
                // `Page · cto`.
                let entry = self.index.by_slug(stem);
                let display_name = entry
                    .map(|e| e.title.clone())
                    .unwrap_or_else(|| stem.to_string());
                let icon_prefix = entry
                    .and_then(|e| e.icon.as_deref())
                    .map(|i| format!("{i} "))
                    .unwrap_or_default();
                format!("Page · {icon_prefix}{display_name} · [{mode_tag}]")
            }
        }
    }

    /// Slug of the currently-opened view, used to look up backlinks.
    pub(crate) fn current_slug(&self) -> String {
        self.current_path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }

    /// Compute the backlinks pointing at `slug` directly from the
    /// workspace. **This is the single source for backlinks across
    /// the TUI** — every call site (panel render, navigation,
    /// keyboard handlers) routes through here.
    ///
    /// Result is cached on [`Self::backlinks_cache`] keyed by slug so
    /// repeated reads (render frame, j/k in the backlink panel) hit
    /// the cache instead of re-scanning the workspace. The cache is
    /// transparent — callers always get a fresh `Vec<Backlink>`
    /// owned by them, the cache stores a reference copy for the
    /// next lookup. Invalidation happens on mutation paths via
    /// [`Self::invalidate_backlinks_cache`] (`save`,
    /// `save_page_with`, `reload_workspace_from_disk`,
    /// `load_current`, view-changing nav).
    ///
    /// The earlier code path read from `WorkspaceIndex.backlinks`,
    /// which has been removed: the `outl-md` index no longer
    /// duplicates this data (`outl_actions::backlinks_for_page` is
    /// the only producer now, shared with the mobile client). Raw
    /// scan cost is `O(blocks in workspace)` per call —
    /// sub-millisecond up to ~10k blocks, but a render frame at
    /// 60fps still issues 60 of them per second, which is what made
    /// the cache worth its weight.
    pub(crate) fn backlinks_for_slug(&self, slug: &str) -> Vec<outl_actions::Backlink> {
        // Cache hit: same slug as the last read, return the clone.
        if let Some((cached_slug, cached_list)) = self.backlinks_cache.borrow().as_ref() {
            if cached_slug == slug {
                return cached_list.clone();
            }
        }
        // Miss: recompute, store, and return.
        let computed = self.compute_backlinks_for_slug(slug);
        *self.backlinks_cache.borrow_mut() = Some((slug.to_string(), computed.clone()));
        computed
    }

    /// Raw computation path — the workspace scan without the cache
    /// layer. Public-in-crate for tests; production callers should
    /// go through [`Self::backlinks_for_slug`].
    pub(crate) fn compute_backlinks_for_slug(&self, slug: &str) -> Vec<outl_actions::Backlink> {
        let Some(id) = outl_actions::find_by_slug(&self.workspace, slug) else {
            return Vec::new();
        };
        let Some(meta) = outl_actions::page_meta(&self.workspace, id) else {
            return Vec::new();
        };
        outl_actions::backlinks_for_page(&self.workspace, &self.workspace_root, &meta)
    }

    /// Convenience: backlinks for the currently-opened page/journal.
    pub(crate) fn backlinks_for_current(&self) -> Vec<outl_actions::Backlink> {
        self.backlinks_for_slug(&self.current_slug())
    }

    /// Number of backlinks pointing at `slug`, without cloning the
    /// list.
    ///
    /// Callers that only need a count (the footer chip in
    /// `view::chrome`, the navigability probe in
    /// [`Self::backlinks_navigable`]) take this instead of
    /// [`Self::backlinks_for_slug`]. The rich `Backlink` struct now
    /// carries `source_block: OutlineNode` plus its subtree, so the
    /// clone the full accessor performs is non-trivial. On a cache
    /// hit `len()` is `O(1)`; on a miss we still scan and populate
    /// the cache but skip the extra `Vec` clone.
    pub(crate) fn backlinks_count_for_slug(&self, slug: &str) -> usize {
        if let Some((cached_slug, cached_list)) = self.backlinks_cache.borrow().as_ref() {
            if cached_slug == slug {
                return cached_list.len();
            }
        }
        // Miss: recompute and store. Length-only callers still benefit
        // because the next read (probably from a richer call site)
        // hits the cache.
        let computed = self.compute_backlinks_for_slug(slug);
        let len = computed.len();
        *self.backlinks_cache.borrow_mut() = Some((slug.to_string(), computed));
        len
    }

    /// Convenience: number of backlinks for the currently-opened
    /// page/journal. See [`Self::backlinks_count_for_slug`].
    pub(crate) fn backlinks_count_for_current(&self) -> usize {
        self.backlinks_count_for_slug(&self.current_slug())
    }

    /// Drop the cached backlinks list. Call this on every workspace
    /// mutation that can change the answer — saves, peer-ops reloads,
    /// view switches. Cheap (just sets the `Option` to `None`).
    pub(crate) fn invalidate_backlinks_cache(&self) {
        *self.backlinks_cache.borrow_mut() = None;
    }

    pub(crate) fn go_today(&mut self) -> Result<()> {
        self.view = View::Journal(clock::today());
        self.selected = 0;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    pub(crate) fn shift_journal(&mut self, days: i64) -> Result<()> {
        let new_date = match self.view {
            View::Journal(d) => d + Duration::days(days),
            _ => clock::today() + Duration::days(days),
        };
        self.view = View::Journal(new_date);
        self.selected = 0;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    pub(crate) fn move_selection(&mut self, delta: i32) {
        if self.flat_len == 0 && matches!(self.focus, Focus::Outline) {
            self.selected = 0;
            self.cursor_col = 0;
            return;
        }
        if delta > 0 {
            for _ in 0..delta {
                if !self.step_forward() {
                    break;
                }
            }
        } else {
            for _ in 0..(-delta) {
                if !self.step_backward() {
                    break;
                }
            }
        }
    }

    /// Advance the cursor by one position in the virtual flat list
    /// `outline blocks ++ backlink section blocks`. Returns `true` if
    /// the cursor moved, `false` when already at the bottom.
    ///
    /// Crosses the boundary between outline and backlinks transparently
    /// when the inline section is shown and non-empty.
    fn step_forward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // Walk forward until we hit a visible block (not
                // hidden under a collapsed ancestor) or fall off the
                // end of the outline.
                let mut next = self.selected + 1;
                while next < self.flat_len
                    && self.hidden_by_collapse.get(next).copied().unwrap_or(false)
                {
                    next += 1;
                }
                if next < self.flat_len {
                    self.selected = next;
                    self.cursor_col = 0;
                    return true;
                }
                // Bottom of outline → try entering the backlinks zone.
                if self.backlinks_navigable() {
                    self.focus = Focus::Backlink {
                        idx: 0,
                        sub_path: Vec::new(),
                    };
                    self.cursor_col = 0;
                    return true;
                }
                false
            }
            Focus::Backlink { idx, sub_path } => {
                // Borrow the backlinks slice directly instead of
                // cloning the whole `Vec<Backlink>` (each entry
                // carries an `OutlineNode` subtree — non-trivial to
                // clone per keystroke).
                let slug = self.current_slug();
                let new_focus = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos + 1 < paths.len() {
                        Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos + 1].clone(),
                        }
                    } else if idx + 1 < backlinks.len() {
                        Focus::Backlink {
                            idx: idx + 1,
                            sub_path: Vec::new(),
                        }
                    } else {
                        return false;
                    }
                };
                self.focus = new_focus;
                self.cursor_col = 0;
                true
            }
        }
    }

    /// Mirror of [`step_forward`], moving one position upward.
    fn step_backward(&mut self) -> bool {
        match self.focus.clone() {
            Focus::Outline => {
                // Walk backward over hidden subtree entries the same
                // way `step_forward` skips them going down.
                if self.selected == 0 {
                    return false;
                }
                let mut prev = self.selected - 1;
                while prev > 0 && self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    prev -= 1;
                }
                if self.hidden_by_collapse.get(prev).copied().unwrap_or(false) {
                    // Reached the top still inside a collapsed
                    // subtree — no visible previous block.
                    return false;
                }
                self.selected = prev;
                self.cursor_col = 0;
                true
            }
            Focus::Backlink { idx, sub_path } => {
                let slug = self.current_slug();
                // Resolve the new focus value while only borrowing the
                // backlinks slice — no `to_vec` clone per keystroke.
                let new_focus_opt = {
                    let backlinks = self.backlinks_for_slug(&slug);
                    let Some(bl) = backlinks.get(idx) else {
                        return false;
                    };
                    let paths = flatten_subtree_paths(&bl.source_block);
                    let cur_pos = paths.iter().position(|p| p == &sub_path).unwrap_or(0);
                    if cur_pos > 0 {
                        Some(Focus::Backlink {
                            idx,
                            sub_path: paths[cur_pos - 1].clone(),
                        })
                    } else if idx > 0 {
                        // Jump to the last block of the previous backlink.
                        let prev_paths = flatten_subtree_paths(&backlinks[idx - 1].source_block);
                        let last = prev_paths.last().cloned().unwrap_or_default();
                        Some(Focus::Backlink {
                            idx: idx - 1,
                            sub_path: last,
                        })
                    } else {
                        // Topping out → fall back into the outline.
                        None
                    }
                };
                match new_focus_opt {
                    Some(f) => self.focus = f,
                    None => {
                        self.focus = Focus::Outline;
                        self.selected = self.flat_len.saturating_sub(1);
                    }
                }
                self.cursor_col = 0;
                true
            }
        }
    }

    /// `true` when the inline backlinks section is rendered *and* has
    /// at least one block the cursor can land on. Drives the cross-zone
    /// transition in `step_forward`/`step_backward`.
    fn backlinks_navigable(&self) -> bool {
        self.show_backlinks && self.backlinks_count_for_current() > 0
    }

    /// Current selected block's text (or empty if no selection).
    /// Honours `app.focus` so backlink blocks return their own text.
    pub(crate) fn current_block_text(&self) -> String {
        match &self.focus {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return String::new();
                };
                node_at_path(&self.page.blocks, &path)
                    .map(|n| n.text.clone())
                    .unwrap_or_default()
            }
            Focus::Backlink { idx, sub_path } => {
                let backlinks = self.backlinks_for_current();
                let Some(bl) = backlinks.get(*idx) else {
                    return String::new();
                };
                let mut node = &bl.source_block;
                for &i in sub_path {
                    let Some(child) = node.children.get(i) else {
                        return String::new();
                    };
                    node = child;
                }
                node.text.clone()
            }
        }
    }

    pub(crate) fn current_block_char_count(&self) -> usize {
        self.current_block_text().chars().count()
    }

    pub(crate) fn move_cursor_col(&mut self, delta: i32) {
        let max = self.current_block_char_count() as i32;
        let next = (self.cursor_col as i32 + delta).clamp(0, max);
        self.cursor_col = next as usize;
    }

    pub(crate) fn cursor_to_home(&mut self) {
        self.cursor_col = 0;
    }

    pub(crate) fn cursor_to_end(&mut self) {
        self.cursor_col = self.current_block_char_count();
    }

    pub(crate) fn cursor_word_left(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let mut i = self.cursor_col;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.cursor_col = i;
    }

    pub(crate) fn cursor_word_right(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = self.cursor_col;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor_col = i;
    }

    /// `zz` — center the viewport vertically on the selected block.
    /// Adjusts `scroll_y` so the selection lands at the midpoint of
    /// `viewport_height`. Clamps at 0 so we don't scroll above the
    /// first block.
    pub(crate) fn center_viewport_on_selection(&mut self) {
        let vp = self.viewport_height.max(1) as i32;
        let target = (self.selected as i32) - vp / 2;
        self.scroll_y = target.max(0) as u16;
    }

    /// `*` / `#` — extract the word under the cursor and feed it to
    /// the workspace search. Direction is `forward = true` for `*`,
    /// `false` for `#`. No-op when the cursor isn't sitting on a word.
    pub(crate) fn search_word_under_cursor(&mut self, forward: bool) -> Result<()> {
        let text = self.current_block_text();
        let Some(word) = word_under_cursor(&text, self.cursor_col) else {
            self.status = "no word under cursor".into();
            return Ok(());
        };
        self.run_inline_search(&word, forward)
    }

    /// Build a search the same way the `/` overlay does, jump straight
    /// to the first hit, persist the rest into `last_search` so
    /// `n`/`N` can walk through them. Used by `*` / `#`.
    fn run_inline_search(&mut self, query: &str, forward: bool) -> Result<()> {
        // Reuse the overlay's machinery: open, set query, refresh,
        // accept. Cheaper than reimplementing the workspace walk.
        self.open_search();
        if let Some(crate::state::Overlay::Search(ref mut s)) = self.overlay {
            s.query = query.to_string();
        }
        self.refresh_search();
        self.accept_search()?;
        if forward {
            // `accept_search` already lands on hit 0; nothing else to do.
        } else {
            // `accept_search` set `last_search.cursor = 0`. `search_prev`
            // wraps 0 → len-1, which is exactly the last hit `#` wants.
            self.search_prev()?;
        }
        Ok(())
    }
}

/// Extract the word under `cursor` from `text`. A "word" is a
/// contiguous run of non-whitespace chars. Returns `None` on
/// whitespace / empty text.
fn word_under_cursor(text: &str, cursor: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let pos = cursor.min(chars.len().saturating_sub(1));
    if chars[pos].is_whitespace() {
        return None;
    }
    let mut start = pos;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = pos;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    if word.is_empty() {
        None
    } else {
        Some(word)
    }
}

impl App {
    /// If the cursor is sitting on a `[[ref]]`, `#tag`, or `[[YYYY-MM-DD]]`,
    /// open the corresponding page or journal. Returns `true` when an
    /// open happened so the caller can suppress the fallback (entering
    /// Insert mode on Enter).
    pub(crate) fn try_open_under_cursor(&mut self) -> Result<bool> {
        let text = self.current_block_text();
        let Some(target) = ref_at_cursor(&text, self.cursor_col) else {
            return Ok(false);
        };
        match target {
            RefTarget::Journal(date) => {
                self.view = View::Journal(date);
                self.selected = 0;
                self.cursor_col = 0;
                self.ensure_view_file_exists()?;
                self.load_current();
            }
            RefTarget::Page(name) | RefTarget::Tag(name) => {
                self.open_page_by_name(&name)?;
            }
            RefTarget::Block(handle) => {
                self.open_block_ref(&handle)?;
            }
        }
        Ok(true)
    }

    /// Open the source page of a `((blk-XXXXXX))` reference and put
    /// the selection on the referenced block.
    ///
    /// Resolution path:
    ///   1. Look the handle up in `WorkspaceIndex::resolve_block_ref`.
    ///      Orphan handles (no resolution) leave a status message and
    ///      otherwise no-op — the user keeps their current view.
    ///   2. Switch `view` to the source page (journal or regular page,
    ///      detected by the `journals/` ancestor segment in the path).
    ///   3. Load the page and translate `source_block_path` (a DFS
    ///      path) into a flat block index via
    ///      [`crate::outline_ops::index_for_path`]. Falls back to the
    ///      top of the page if the path no longer resolves (block
    ///      moved/deleted since the index was built).
    pub(crate) fn open_block_ref(&mut self, handle: &str) -> Result<()> {
        let Some(entry) = self.index.resolve_block_ref(handle) else {
            self.status = format!("ref (({handle})) does not resolve");
            return Ok(());
        };
        let source_path = entry.source_path.clone();
        let source_block_path = entry.source_block_path.clone();

        // Detect journal vs page from the path layout. Workspace
        // layout pins journals under `journals/` and pages under
        // `pages/`; everything else falls back to a page view.
        let is_journal = source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("journals");
        if is_journal {
            if let Some(stem) = source_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                    self.selected = 0;
                    self.cursor_col = 0;
                    self.load_current();
                    self.selected =
                        crate::outline_ops::index_for_path(&self.page.blocks, &source_block_path)
                            .unwrap_or(0);
                    return Ok(());
                }
            }
        }
        self.view = View::Page(source_path);
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
        self.selected =
            crate::outline_ops::index_for_path(&self.page.blocks, &source_block_path).unwrap_or(0);
        Ok(())
    }

    /// Open (or create) the page corresponding to a user-visible name.
    /// Files live under `pages/{slug}.md`; the original `name` is
    /// preserved in the page's `title::` property.
    pub(crate) fn open_page_by_name(&mut self, name: &str) -> Result<()> {
        let slug = outl_md::slug::slugify(name);
        let path = self.workspace_root.join("pages").join(format!("{slug}.md"));
        let created_new = !path.exists();
        if created_new {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // Seed with title:: <name> + a single empty bullet so the
            // editor has a cursor home.
            let seed = format!("title:: {name}\n\n- \n");
            outl_md::write_atomic(&path, seed.as_bytes())?;
            // Reconcile to establish stable IDs.
            let _ = reconcile_md(
                &mut self.workspace,
                &self.hlc,
                &path,
                Some(&self.orphans_log),
            );
        }
        self.view = View::Page(path);
        self.selected = 0;
        self.cursor_col = 0;
        self.load_current();
        self.refresh_page_list();
        if created_new {
            self.status = format!("created page \"{name}\"");
        }
        Ok(())
    }
}

#[cfg(test)]
mod word_tests {
    use super::word_under_cursor;

    #[test]
    fn returns_word_under_cursor() {
        assert_eq!(
            word_under_cursor("hello world", 2),
            Some("hello".to_string())
        );
        assert_eq!(
            word_under_cursor("hello world", 6),
            Some("world".to_string())
        );
    }

    #[test]
    fn returns_none_on_whitespace_or_empty() {
        assert_eq!(word_under_cursor("hello world", 5), None);
        assert_eq!(word_under_cursor("", 0), None);
        assert_eq!(word_under_cursor("   ", 1), None);
    }

    #[test]
    fn clamps_past_end() {
        assert_eq!(
            word_under_cursor("hello", 99),
            Some("hello".to_string()),
            "cursor past EOL still picks up the last word"
        );
    }
}
