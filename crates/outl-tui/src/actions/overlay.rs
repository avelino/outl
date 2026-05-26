//! Modal overlays: quick switcher, workspace search, command palette,
//! and the Insert-mode inline `[[`/`#` autocomplete popup.
//!
//! Each overlay has the same shape — `open_*` initialises it,
//! `refresh_*` updates the candidate list as the user types,
//! `accept_*` commits the selection.

use crate::state::{
    hit_count, App, AutocompleteKind, AutocompleteState, CommandState, ErrorState, LastSearch,
    Mode, Overlay, QuickSwitchState, SearchHit, SearchState, SlashCommand, SlashState,
    SwitchCandidate, SwitchKind, View,
};
use anyhow::Result;
use outl_md::parse::OutlineNode;
use std::path::{Path, PathBuf};

impl App {
    /// Build the universe of switchable items: every page in `pages/`,
    /// every existing journal in `journals/`, plus today's date even if
    /// the journal file doesn't exist yet.
    fn collect_switch_candidates(&self) -> Vec<SwitchCandidate> {
        let mut out = Vec::new();

        // Pages.
        for path in &self.page_list {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Page title comes from the `title::` property if set, else slug.
            // Prefer the index — it already parsed everything once at
            // load time and reading from disk on every keystroke would
            // make the switcher noticeably laggy on big workspaces.
            let (title, icon) = self
                .index
                .by_slug(&stem)
                .map(|p| (p.title.clone(), p.icon.clone()))
                .unwrap_or_else(|| (read_page_title(path).unwrap_or_else(|| stem.clone()), None));
            let label = match icon {
                Some(i) => format!("{i} {title}"),
                None => title,
            };
            out.push(SwitchCandidate {
                label,
                key: stem,
                kind: SwitchKind::Page,
                score: 0,
            });
        }

        // Journals.
        let journals_dir = self.workspace_root.join("journals");
        if journals_dir.is_dir() {
            let mut journals: Vec<PathBuf> = walkdir::WalkDir::new(&journals_dir)
                .max_depth(1)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path().extension().is_some_and(|x| x == "md")
                        && !e.file_name().to_string_lossy().starts_with('.')
                })
                .map(|e| e.path().to_path_buf())
                .collect();
            journals.sort_by(|a, b| b.cmp(a)); // newest first
            for p in journals {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(SwitchCandidate {
                        label: stem.to_string(),
                        key: stem.to_string(),
                        kind: SwitchKind::Journal,
                        score: 0,
                    });
                }
            }
        }

        // Always include today even if no file exists yet — typing
        // "today" always lands somewhere.
        let today_str = chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        if !out
            .iter()
            .any(|c| c.kind == SwitchKind::Journal && c.key == today_str)
        {
            out.push(SwitchCandidate {
                label: format!("{today_str} (new)"),
                key: today_str,
                kind: SwitchKind::Journal,
                score: 0,
            });
        }

        out
    }

    pub(crate) fn open_quick_switch(&mut self) {
        let candidates = self.collect_switch_candidates();
        self.overlay = Some(Overlay::QuickSwitch(QuickSwitchState {
            query: String::new(),
            candidates,
            selected: 0,
            preview_cache: std::cell::RefCell::new(None),
        }));
    }

    pub(crate) fn refresh_quick_switch(&mut self) {
        if let Some(Overlay::QuickSwitch(ref mut qs)) = self.overlay {
            // Score every candidate by the current query; drop misses.
            let mut filtered: Vec<SwitchCandidate> = qs
                .candidates
                .iter()
                .filter_map(|c| {
                    let primary = crate::fuzzy::fuzzy_score(&qs.query, &c.label);
                    let secondary = crate::fuzzy::fuzzy_score(&qs.query, &c.key);
                    let best = match (primary, secondary) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    };
                    best.map(|s| SwitchCandidate {
                        score: s,
                        ..c.clone()
                    })
                })
                .collect();
            filtered.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
            qs.candidates = filtered;
            qs.selected = qs.selected.min(qs.candidates.len().saturating_sub(1));
        }
    }

    pub(crate) fn accept_quick_switch(&mut self) -> Result<()> {
        let pick = if let Some(Overlay::QuickSwitch(ref qs)) = self.overlay {
            qs.candidates.get(qs.selected).cloned()
        } else {
            None
        };
        self.overlay = None;
        let Some(c) = pick else {
            return Ok(());
        };
        match c.kind {
            SwitchKind::Page => self.open_page_by_name(&c.key)?,
            SwitchKind::Journal => {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(&c.key, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                    self.selected = 0;
                    self.cursor_col = 0;
                    self.ensure_view_file_exists()?;
                    self.load_current();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn open_search(&mut self) {
        self.overlay = Some(Overlay::Search(SearchState {
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
        }));
    }

    pub(crate) fn refresh_search(&mut self) {
        let Some(Overlay::Search(ref state)) = self.overlay else {
            return;
        };
        let query = state.query.clone();
        if query.trim().is_empty() {
            if let Some(Overlay::Search(ref mut s)) = self.overlay {
                s.hits.clear();
            }
            return;
        }
        // Scan all .md files in pages/ and journals/. For each block,
        // run fuzzy match; keep top 30 hits by score.
        let mut hits = Vec::new();
        for dir in ["pages", "journals"] {
            let base = self.workspace_root.join(dir);
            if !base.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&base).max_depth(1) {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.path().extension().and_then(|x| x.to_str()) != Some("md") {
                    continue;
                }
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let parsed = outl_md::parse::parse(&text);
                let page_label = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "title")
                    .map(|(_, v)| v.clone())
                    .or_else(|| {
                        entry
                            .path()
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(String::from)
                    })
                    .unwrap_or_default();
                let page_icon = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "icon")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty());
                let mut block_idx = 0usize;
                collect_block_hits(
                    &parsed.blocks,
                    &mut block_idx,
                    entry.path(),
                    &page_label,
                    page_icon.as_deref(),
                    &query,
                    &mut hits,
                );
            }
        }
        hits.sort_by_key(|b| std::cmp::Reverse(b.score));
        hits.truncate(30);
        if let Some(Overlay::Search(ref mut s)) = self.overlay {
            s.hits = hits;
            s.selected = s.selected.min(s.hits.len().saturating_sub(1));
        }
    }

    pub(crate) fn accept_search(&mut self) -> Result<()> {
        // Snapshot the entire result list onto the App so `n`/`N`
        // can walk through them after the overlay closes.
        let (hits, selected) = if let Some(Overlay::Search(ref s)) = self.overlay {
            (s.hits.clone(), s.selected)
        } else {
            return Ok(());
        };
        self.overlay = None;
        self.last_search = Some(LastSearch {
            hits: hits.clone(),
            cursor: selected,
        });
        let Some(h) = hits.get(selected).cloned() else {
            return Ok(());
        };
        self.jump_to_search_hit(&h)
    }

    /// Move to the n-th hit in the persisted last-search list.
    fn jump_to_search_hit(&mut self, h: &SearchHit) -> Result<()> {
        if h.md_path.starts_with(self.workspace_root.join("journals")) {
            if let Some(stem) = h.md_path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                    self.view = View::Journal(date);
                }
            }
        } else {
            self.view = View::Page(h.md_path.clone());
        }
        self.selected = h.block_index;
        self.cursor_col = 0;
        self.ensure_view_file_exists()?;
        self.load_current();
        Ok(())
    }

    /// Jump to the next hit in the last `/` search. No-op if there's
    /// no saved search or the cursor is at the end.
    pub(crate) fn search_next(&mut self) -> Result<()> {
        let Some(ref ls) = self.last_search else {
            self.status = "no previous search".into();
            return Ok(());
        };
        if ls.hits.is_empty() {
            return Ok(());
        }
        let next = (ls.cursor + 1) % ls.hits.len();
        let hit = ls.hits[next].clone();
        if let Some(ls_mut) = self.last_search.as_mut() {
            ls_mut.cursor = next;
        }
        self.status = format!("hit {}/{}", next + 1, hit_count(&self.last_search));
        self.jump_to_search_hit(&hit)
    }

    /// Jump to the previous hit. Wraps to the last on underflow.
    pub(crate) fn search_prev(&mut self) -> Result<()> {
        let Some(ref ls) = self.last_search else {
            self.status = "no previous search".into();
            return Ok(());
        };
        if ls.hits.is_empty() {
            return Ok(());
        }
        let prev = if ls.cursor == 0 {
            ls.hits.len() - 1
        } else {
            ls.cursor - 1
        };
        let hit = ls.hits[prev].clone();
        if let Some(ls_mut) = self.last_search.as_mut() {
            ls_mut.cursor = prev;
        }
        self.status = format!("hit {}/{}", prev + 1, hit_count(&self.last_search));
        self.jump_to_search_hit(&hit)
    }

    /// Pop a modal error/warning over everything. Use for failures
    /// the status line can't fit — compile errors, multi-line traps,
    /// "rustc not found" toolchain hints. Any key dismisses.
    pub(crate) fn show_error(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.overlay = Some(Overlay::Error(ErrorState {
            title: title.into(),
            body: body.into(),
        }));
    }

    /// Open the Notion-style slash command menu.
    ///
    /// Bound to `/` in Normal mode. Replaces the old `/` =
    /// workspace-search binding — search is now a *command* inside
    /// the menu (one extra keystroke, full discoverability and
    /// future plugin commands appear here automatically).
    pub(crate) fn open_slash(&mut self) {
        let candidates = self
            .command_registry
            .all()
            .map(|c| SlashCommand {
                name: c.name(),
                description: c.description(),
                needs_args: c.needs_args(),
            })
            .collect();
        self.overlay = Some(Overlay::Slash(SlashState {
            query: String::new(),
            candidates,
            selected: 0,
        }));
    }

    /// Recompute the slash overlay's candidate list against the
    /// current query (fuzzy match on `name`).
    pub(crate) fn refresh_slash(&mut self) {
        let Some(Overlay::Slash(ref state)) = self.overlay else {
            return;
        };
        let query = state.query.clone();
        let mut filtered: Vec<(i32, SlashCommand)> = self
            .command_registry
            .all()
            .filter_map(|c| {
                let entry = SlashCommand {
                    name: c.name(),
                    description: c.description(),
                    needs_args: c.needs_args(),
                };
                if query.is_empty() {
                    Some((0, entry))
                } else {
                    crate::fuzzy::fuzzy_score(&query, c.name()).map(|score| (score, entry))
                }
            })
            .collect();
        filtered.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(b.1.name)));
        if let Some(Overlay::Slash(ref mut s)) = self.overlay {
            s.candidates = filtered.into_iter().map(|(_, c)| c).collect();
            s.selected = s.selected.min(s.candidates.len().saturating_sub(1));
        }
    }

    /// Accept the highlighted slash command. Commands with `needs_args`
    /// hand off to the vim palette pre-filled for arg entry; the
    /// arg-less ones dispatch through the registry directly.
    pub(crate) fn accept_slash(&mut self) -> Result<bool> {
        let pick = match &self.overlay {
            Some(Overlay::Slash(s)) => s.candidates.get(s.selected).cloned(),
            _ => None,
        };
        self.overlay = None;
        let Some(cmd) = pick else {
            return Ok(false);
        };
        if cmd.needs_args {
            self.overlay = Some(Overlay::Command(CommandState {
                buffer: format!("{} ", cmd.name),
            }));
            return Ok(false);
        }
        let registry = self.command_registry.clone();
        registry.dispatch(self, cmd.name)
    }

    pub(crate) fn open_command(&mut self) {
        self.overlay = Some(Overlay::Command(CommandState {
            buffer: String::new(),
        }));
    }

    // --- inline autocomplete --------------------------------------------

    /// Inspect the Insert buffer after a keystroke and toggle the
    /// autocomplete popup when the trigger conditions are met.
    pub(crate) fn maybe_update_autocomplete(&mut self) {
        let Mode::Insert { buffer, .. } = &self.mode else {
            self.autocomplete = None;
            return;
        };
        // Find any open `[[` or `#` immediately before the cursor.
        let text = buffer.as_string();
        let cursor = buffer.cursor;
        let chars: Vec<char> = text.chars().collect();

        // Walk back from cursor to find a trigger or whitespace/limit.
        let trigger = detect_trigger(&chars, cursor);
        match trigger {
            Some((AutocompleteKind::PageRef, query)) => {
                let candidates = self.candidates_for_pageref(&query);
                self.autocomplete = Some(AutocompleteState {
                    kind: AutocompleteKind::PageRef,
                    query,
                    candidates,
                    selected: 0,
                });
            }
            Some((AutocompleteKind::Tag, query)) => {
                let candidates = self.candidates_for_tag(&query);
                self.autocomplete = Some(AutocompleteState {
                    kind: AutocompleteKind::Tag,
                    query,
                    candidates,
                    selected: 0,
                });
            }
            Some((AutocompleteKind::BlockRef, query)) => {
                let candidates = self.candidates_for_blockref(&query);
                self.autocomplete = Some(AutocompleteState {
                    kind: AutocompleteKind::BlockRef,
                    query,
                    candidates,
                    selected: 0,
                });
            }
            Some((AutocompleteKind::SlashCommand, query)) => {
                let candidates = self.candidates_for_slash(&query);
                // No candidates → no popup. Keeps `/` typed in random
                // mid-word contexts from showing an empty box.
                if candidates.is_empty() {
                    self.autocomplete = None;
                } else {
                    self.autocomplete = Some(AutocompleteState {
                        kind: AutocompleteKind::SlashCommand,
                        query,
                        candidates,
                        selected: 0,
                    });
                }
            }
            None => {
                self.autocomplete = None;
            }
        }
    }

    /// Fuzzy-match slash command names against `q`. Empty `q` shows
    /// every command (in registry order).
    fn candidates_for_slash(&self, q: &str) -> Vec<String> {
        if q.is_empty() {
            return self
                .command_registry
                .all()
                .map(|c| c.name().to_string())
                .collect();
        }
        let mut scored: Vec<(i32, String)> = self
            .command_registry
            .all()
            .filter_map(|c| {
                let name = c.name();
                crate::fuzzy::fuzzy_score(q, name).map(|s| (s, name.to_string()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(8).map(|(_, n)| n).collect()
    }

    /// Sorted page-title candidates matching the query (fuzzy, top 8).
    fn candidates_for_pageref(&self, q: &str) -> Vec<String> {
        let mut scored: Vec<(i32, String)> = self
            .index
            .pages()
            .filter_map(|p| {
                let title = p.title.clone();
                crate::fuzzy::fuzzy_score(q, &title).map(|s| (s, title))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(8).map(|(_, n)| n).collect()
    }

    /// Block-ref candidates for the `((` autocomplete.
    ///
    /// Returns **handles** (`blk-XXXXXX`) — the popup looks each one
    /// up in the index for its display text. Empty query shows the
    /// most-recent blocks (ULID-sorted descending, so newest first),
    /// which is deterministic across rebuilds. Non-empty query
    /// delegates to `WorkspaceIndex::search_block_text` which is
    /// case-insensitive substring + prefix-first ranking.
    fn candidates_for_blockref(&self, q: &str) -> Vec<String> {
        if q.is_empty() {
            // HashMap iteration order is unstable; sort descending by
            // NodeId (ULIDs are lexicographically time-sortable) so
            // the popup shows the same eight rows every keystroke and
            // newest-edited blocks surface at the top.
            let mut entries: Vec<_> = self.index.iter_blocks().collect();
            entries.sort_by_key(|b| std::cmp::Reverse(b.id));
            return entries
                .into_iter()
                .take(8)
                .map(|b| b.ref_handle.clone())
                .collect();
        }
        self.index
            .search_block_text(q, 8)
            .into_iter()
            .map(|b| b.ref_handle.clone())
            .collect()
    }

    /// Tag candidates — for now, every page title (tags resolve to pages).
    /// Filtered by fuzzy match on the query.
    fn candidates_for_tag(&self, q: &str) -> Vec<String> {
        let mut scored: Vec<(i32, String)> = self
            .index
            .pages()
            .filter_map(|p| {
                let slug = p.slug.clone();
                crate::fuzzy::fuzzy_score(q, &slug).map(|s| (s, slug))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(8).map(|(_, n)| n).collect()
    }

    /// Accept the highlighted autocomplete candidate.
    ///
    /// For `[[` / `#` triggers: the trigger + query are replaced in
    /// the buffer by the completed token.
    ///
    /// For `/` slash triggers: the trigger + query are *deleted*
    /// from the buffer, the current Insert is committed, and the
    /// command runs (or pops the `:` palette for arg entry). Failure
    /// to dispatch shows up as a status-line message; never breaks
    /// the user's editing flow.
    pub(crate) fn accept_autocomplete(&mut self) {
        let Some(ac) = self.autocomplete.take() else {
            return;
        };
        let Some(choice) = ac.candidates.get(ac.selected).cloned() else {
            return;
        };
        // Slash commands take a different path: they remove the
        // trigger from the buffer and *execute* — not insert.
        if let AutocompleteKind::SlashCommand = ac.kind {
            self.accept_slash_inline(&ac, &choice);
            return;
        }

        let Mode::Insert { buffer, .. } = &mut self.mode else {
            return;
        };
        // Replace from the trigger position to the cursor with the
        // completed token (closing the brackets for refs).
        let trigger_len = match ac.kind {
            AutocompleteKind::PageRef => 2 + ac.query.chars().count(), // `[[query`
            AutocompleteKind::BlockRef => 2 + ac.query.chars().count(), // `((query`
            AutocompleteKind::Tag => 1 + ac.query.chars().count(),     // `#query`
            AutocompleteKind::SlashCommand => unreachable!("handled above"),
        };
        // Delete the trigger + query characters before the cursor.
        for _ in 0..trigger_len {
            buffer.delete_back();
        }
        match ac.kind {
            AutocompleteKind::PageRef => {
                buffer.insert_str(&format!("[[{choice}]]"));
            }
            AutocompleteKind::BlockRef => {
                // `choice` is the handle (e.g. `blk-r6s4a1`).
                buffer.insert_str(&format!("(({choice}))"));
            }
            AutocompleteKind::Tag => {
                buffer.insert_str(&format!("#{choice}"));
            }
            AutocompleteKind::SlashCommand => unreachable!("handled above"),
        }
    }

    /// Slash branch of [`accept_autocomplete`]: erase `/<query>` from
    /// the buffer, then dispatch.
    ///
    /// Commands that **don't** insert text into the buffer
    /// (`inserts_inline() == false`, i.e. the default) get the Insert
    /// committed first — we don't want the in-flight edit alive
    /// alongside an overlay-opening command. Inline-insert commands
    /// (`/date-today` and friends) skip the commit so they can call
    /// `buffer.insert_str(...)` at the cursor.
    fn accept_slash_inline(&mut self, ac: &AutocompleteState, choice: &str) {
        let trigger_len = 1 + ac.query.chars().count();
        if let Mode::Insert { buffer, .. } = &mut self.mode {
            for _ in 0..trigger_len {
                buffer.delete_back();
            }
        }

        let registry = self.command_registry.clone();
        let Some(cmd) = registry.get(choice) else {
            self.commit_insert();
            self.status = format!("unknown command: {choice}");
            return;
        };
        if !cmd.inserts_inline() {
            self.commit_insert();
        }
        if cmd.needs_args() {
            self.overlay = Some(Overlay::Command(CommandState {
                buffer: format!("{choice} "),
            }));
            return;
        }
        if let Err(e) = registry.dispatch(self, choice) {
            self.status = format!("/{choice} failed: {e}");
        }
    }
}

/// Look back from `cursor` over `chars` to find an open `[[` / `#` /
/// `/` trigger plus the query typed after it. Returns `None` if no
/// trigger is active (cursor outside any open token, or query contains
/// chars that close it).
pub(crate) fn detect_trigger(chars: &[char], cursor: usize) -> Option<(AutocompleteKind, String)> {
    if cursor == 0 {
        return None;
    }
    // Walk back until we hit a trigger boundary.
    let mut i = cursor;
    while i > 0 {
        let prev = chars[i - 1];
        // `[[ref` — look for two `[` in a row.
        if prev == '[' && i >= 2 && chars[i - 2] == '[' {
            let query: String = chars[i..cursor].iter().collect();
            // The query must not have started closing the token.
            if query.contains(']') || query.contains('\n') {
                return None;
            }
            return Some((AutocompleteKind::PageRef, query));
        }
        // `((blk-` — look for two `(` in a row. Same shape as PageRef
        // but with parens — the trigger fires immediately after `((`
        // so the popup is ready to filter by block text.
        if prev == '(' && i >= 2 && chars[i - 2] == '(' {
            let query: String = chars[i..cursor].iter().collect();
            if query.contains(')') || query.contains('\n') {
                return None;
            }
            return Some((AutocompleteKind::BlockRef, query));
        }
        if prev == '#' {
            // Tag must be word-initial: preceded by start-of-buffer or
            // whitespace.
            let at_start = i == 1;
            let preceded_by_space = i >= 2 && chars[i - 2].is_whitespace();
            if !(at_start || preceded_by_space) {
                return None;
            }
            let query: String = chars[i..cursor].iter().collect();
            // Tag identifier ends on the first non-tag char.
            if query
                .chars()
                .any(|c| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '/'))
            {
                return None;
            }
            return Some((AutocompleteKind::Tag, query));
        }
        if prev == '/' {
            // Slash command — same word-initial rule as `#`. Lets us
            // not fire on URLs (`http://...`) or paths (`a/b`).
            let at_start = i == 1;
            let preceded_by_space = i >= 2 && chars[i - 2].is_whitespace();
            if !(at_start || preceded_by_space) {
                return None;
            }
            let query: String = chars[i..cursor].iter().collect();
            // Command names are `[a-z0-9-]` only. Any other char and
            // we close the trigger silently — the user clearly wanted
            // a literal `/...` (path, URL fragment, etc).
            if query
                .chars()
                .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
            {
                return None;
            }
            return Some((AutocompleteKind::SlashCommand, query));
        }
        // Whitespace or `]` ends the search without finding a trigger.
        if prev.is_whitespace() || prev == ']' {
            return None;
        }
        i -= 1;
    }
    None
}

/// Recursively walk an outline scoring each block's text against `query`.
pub(crate) fn collect_block_hits(
    blocks: &[OutlineNode],
    cursor: &mut usize,
    md_path: &Path,
    page_label: &str,
    page_icon: Option<&str>,
    query: &str,
    hits: &mut Vec<SearchHit>,
) {
    for b in blocks {
        if let Some(score) = crate::fuzzy::fuzzy_score(query, &b.text) {
            hits.push(SearchHit {
                page_label: page_label.to_string(),
                page_icon: page_icon.map(String::from),
                md_path: md_path.to_path_buf(),
                snippet: truncate_for_snippet(&b.text, 80),
                block_index: *cursor,
                score,
            });
        }
        *cursor += 1;
        collect_block_hits(
            &b.children,
            cursor,
            md_path,
            page_label,
            page_icon,
            query,
            hits,
        );
    }
}

pub(crate) fn truncate_for_snippet(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

/// Best-effort read of `title::` from a `.md` file. Returns the
/// (possibly trimmed) value or `None` if the file is unreadable or has
/// no title.
pub(crate) fn read_page_title(md: &Path) -> Option<String> {
    let text = std::fs::read_to_string(md).ok()?;
    let parsed = outl_md::parse::parse(&text);
    parsed
        .properties
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty())
}
