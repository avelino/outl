//! Inline autocomplete popup for Insert mode — the `[[ref]]`, `#tag`,
//! `((blk-…))`, `/command`, and `@mention` triggers.
//!
//! Split out of `overlay.rs` so each file owns one concern: this
//! module covers the in-place popup that closes on a keystroke; the
//! sibling `overlay` module covers full-screen modals (quick switcher,
//! workspace search, command palette).
//!
//! `detect_trigger` is the pure walk-back over the caret context that
//! tells the impl-side handler which popup to open. `App::*` methods
//! own the workspace-aware candidate ranking and the accept gesture.
//! Both run on every keystroke, so the trigger detection caps its
//! lookback at 64 chars and never allocates beyond the matched span.

use crate::state::{App, AutocompleteKind, AutocompleteState, CommandState, Mode, Overlay};

impl App {
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
            Some((AutocompleteKind::Mention, query)) => {
                let candidates = self.candidates_for_mention(&query);
                // No candidates → close the popup. The user typed past
                // the matchable prefix (e.g. `@Thiago xyz`), so silently
                // dropping the popup is the right move — the next
                // matching keystroke reopens it.
                if candidates.is_empty() {
                    self.autocomplete = None;
                } else {
                    self.autocomplete = Some(AutocompleteState {
                        kind: AutocompleteKind::Mention,
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

    /// Mention candidates — page titles filtered to `type:: person`,
    /// fuzzy-matched against the query.
    ///
    /// Empty query returns the first 8 persons in title order so the
    /// popup is never blank right after the `@` keypress.
    ///
    /// **Create-new affordance.** When `q` is non-empty and no existing
    /// person matches it exactly (case-insensitive), the query itself
    /// is appended to the candidate list as a "create" entry. Accepting
    /// inserts `[[@<query>]]`; the person page is materialised lazily
    /// when the user opens that ref (via `open_or_create_by_ref`, which
    /// already strips the `@` and sets `type:: person`).
    /// Matches Notion / Slack / Linear's `@` convention: the popup
    /// always has at least one row when the user is typing.
    fn candidates_for_mention(&self, q: &str) -> Vec<String> {
        let mut scored: Vec<(i32, String)> = self
            .index
            .pages_by_type(outl_actions::PERSON_TYPE)
            .filter_map(|p| {
                let title = p.title.clone();
                if q.is_empty() {
                    // Synthetic score so empty-query path keeps stable
                    // ordering by title.
                    return Some((0, title));
                }
                crate::fuzzy::fuzzy_score(q, &title).map(|s| (s, title))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut result: Vec<String> = scored.into_iter().take(8).map(|(_, n)| n).collect();
        // Append the query as a create-new candidate when it doesn't
        // already match an existing person (case-insensitive). The
        // typed casing is preserved so `@Vini` → `[[@Vini]]` and
        // `@vini` → `[[@vini]]`.
        if !q.is_empty()
            && !result
                .iter()
                .any(|title| title.to_lowercase() == q.to_lowercase())
        {
            result.push(q.to_string());
        }
        result
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

        {
            let Mode::Insert { buffer, .. } = &mut self.mode else {
                return;
            };
            // Replace from the trigger position to the cursor with the
            // completed token (closing the brackets for refs).
            let trigger_len = match ac.kind {
                AutocompleteKind::PageRef => 2 + ac.query.chars().count(), // `[[query`
                AutocompleteKind::BlockRef => 2 + ac.query.chars().count(), // `((query`
                AutocompleteKind::Tag => 1 + ac.query.chars().count(),     // `#query`
                AutocompleteKind::Mention => 1 + ac.query.chars().count(), // `@query` (spaces OK)
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
                AutocompleteKind::Mention => {
                    // `choice` is the person's title (no `@`). The `@`
                    // belongs to the link affordance, not the page identity.
                    buffer.insert_str(&format!("[[@{choice}]]"));
                }
                AutocompleteKind::SlashCommand => unreachable!("handled above"),
            }
        }
        // Mention sugar: materialise the person page so the inserted
        // `[[@title]]` link doesn't dangle. `open_or_create_by_ref`
        // is idempotent — returns the existing page when present, or
        // creates a fresh one tagged `type:: person` when missing.
        //
        // Three steps after the resolve:
        // 1. Project the page's `.md` + sidecar to disk via
        //    `apply_page_md_with_sidecar`. Otherwise the page exists
        //    only in the op log and the next `WorkspaceIndex` query
        //    won't see it.
        // 2. Re-parse the rendered `.md` and `WorkspaceIndex::patch_page`
        //    so `pages_by_type(PERSON_TYPE)` includes the new person
        //    on the next keystroke. Without this, `@sam` types in
        //    again would still only offer "sam" as a create-new
        //    even though we just minted the page.
        // 3. Errors fall back to a status-bar message — the op log
        //    mutation already landed, and the next save / orphan scan
        //    will retry the projection.
        //
        // Same gesture desktop + mobile apply on accept (with a Tauri
        // event for the failure case there).
        if let AutocompleteKind::Mention = ac.kind {
            let target = format!("@{choice}");
            match outl_actions::open_or_create_by_ref(&mut self.workspace, &self.hlc, &target) {
                Err(e) => {
                    self.status = format!("create person {choice} failed: {e}");
                }
                Ok(id) => {
                    let workspace_root = self.workspace_root.clone();
                    match outl_actions::apply_page_md_with_sidecar(
                        &self.workspace,
                        &workspace_root,
                        id,
                    ) {
                        Ok(path) => {
                            if let Ok(text) = std::fs::read_to_string(&path) {
                                let parsed = outl_md::parse::parse(&text);
                                self.index.patch_page(&path, &parsed);
                            }
                        }
                        Err(e) => {
                            self.status =
                                format!("create person {choice}: md projection failed: {e}");
                        }
                    }
                }
            }
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
/// `/` / `@` trigger plus the query typed after it. Returns `None` if
/// no trigger is active (cursor outside any open token, or query
/// contains chars that close it).
pub(crate) fn detect_trigger(chars: &[char], cursor: usize) -> Option<(AutocompleteKind, String)> {
    if cursor == 0 {
        return None;
    }
    // Mention pre-pass — walk back looking for a word-initial `@`,
    // allowing spaces in the query so composite names work
    // (`@Thiago Avelino`). The main walk-back below stops at the first
    // whitespace, so without this pre-pass `@Thiago ` would never
    // trigger Mention. Caps at 64 chars to avoid scanning the whole
    // buffer on a stray `@` followed by lots of prose.
    {
        let lower_bound = cursor.saturating_sub(64);
        let mut j = cursor;
        while j > lower_bound {
            let ch = chars[j - 1];
            if matches!(ch, '\n' | '[' | ']' | '(' | ')') {
                break;
            }
            if ch == '@' {
                let at_start = j == 1;
                let preceded_by_space = j >= 2 && chars[j - 2].is_whitespace();
                if at_start || preceded_by_space {
                    let query: String = chars[j..cursor].iter().collect();
                    return Some((AutocompleteKind::Mention, query));
                }
                // Mid-word `@` (email, social handle, etc.) is not a
                // trigger — bail out so the main walk-back doesn't
                // accidentally swallow it as part of another token.
                break;
            }
            j -= 1;
        }
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

#[cfg(test)]
mod tests {
    use crate::state::AutocompleteKind;

    #[test]
    fn detect_trigger_mention_at_buffer_start() {
        let chars: Vec<char> = "@av".chars().collect();
        let trigger = super::detect_trigger(&chars, chars.len());
        assert_eq!(trigger, Some((AutocompleteKind::Mention, "av".to_string())));
    }

    #[test]
    fn detect_trigger_mention_after_whitespace() {
        // Word-initial: `@` preceded by space counts.
        let chars: Vec<char> = "hi @av".chars().collect();
        let trigger = super::detect_trigger(&chars, chars.len());
        assert_eq!(trigger, Some((AutocompleteKind::Mention, "av".to_string())));
    }

    #[test]
    fn detect_trigger_mention_keeps_spaces_in_query() {
        // Composite name: query goes past spaces, all the way back to
        // the opening `@`.
        let chars: Vec<char> = "@Thiago Av".chars().collect();
        let trigger = super::detect_trigger(&chars, chars.len());
        assert_eq!(
            trigger,
            Some((AutocompleteKind::Mention, "Thiago Av".to_string()))
        );
    }

    #[test]
    fn detect_trigger_mention_ignores_mid_word_at() {
        // `a@b.com` (email-shaped) must NOT trigger mention — the
        // `@` is not word-initial.
        let chars: Vec<char> = "a@b".chars().collect();
        let trigger = super::detect_trigger(&chars, chars.len());
        assert!(
            !matches!(trigger, Some((AutocompleteKind::Mention, _))),
            "mid-word `@` must not produce a mention trigger"
        );
    }

    #[test]
    fn detect_trigger_mention_stops_at_brackets() {
        // The `[[` opener for a normal page-ref must win over a stray
        // `@` that sits before the bracket (because the user's caret
        // is inside a `[[foo...` token, not a mention).
        let chars: Vec<char> = "@x [[av".chars().collect();
        let trigger = super::detect_trigger(&chars, chars.len());
        // Should be PageRef, not Mention.
        assert_eq!(trigger, Some((AutocompleteKind::PageRef, "av".to_string())));
    }
}
