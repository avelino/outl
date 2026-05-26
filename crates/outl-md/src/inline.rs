//! Inline tokenization and cursor introspection — agnostic of any UI.
//!
//! This module exists so the TUI, a future Tauri/desktop GUI, and the
//! mobile (uniffi-bridged) clients can all share the same understanding
//! of what's inside a block:
//!
//! - **TUI** maps each [`InlineTok`] to a `ratatui::Span` with style.
//! - **Tauri / web** maps tokens to HTML / React fragments.
//! - **iOS / Android** maps tokens to `AttributedString` /
//!   `AnnotatedString`.
//!
//! The recognized constructs:
//!
//! - `[[name]]` — outl page reference (lives in `pages/{slugify(name)}.md`).
//! - `[[YYYY-MM-DD]]` — journal date reference.
//! - `#tag` — tag (resolves to a page when opened).
//! - `**bold**` / `*italic*` / `_italic_` / `~~strike~~` / `` `code` ``.
//! - `[text](url)` — standard markdown link.
//! - Anything else: [`InlineTok::Plain`].
//!
//! Multi-byte UTF-8 (accents, emoji, CJK) is handled correctly — we
//! always advance by `ch.len_utf8()`, never by raw byte.

use chrono::NaiveDate;

/// A token recognized in inline block content.
///
/// Lifetimes reference the source string; clone with `to_owned()` if
/// the consumer needs to outlive the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineTok<'a> {
    /// Bare text with no formatting.
    Plain(&'a str),
    /// `[[name]]` — outl page reference.
    PageRef {
        /// Display name (preserved verbatim; the filename is the
        /// slugified form).
        name: &'a str,
    },
    /// `#tag`.
    Tag {
        /// Tag identifier without the leading `#`.
        name: &'a str,
    },
    /// `**bold**`.
    Bold {
        /// Inner text between the markers.
        inner: &'a str,
    },
    /// `*italic*` or `_italic_`. `marker` is the literal delimiter used.
    Italic {
        /// Inner text between the markers.
        inner: &'a str,
        /// Either `'*'` or `'_'`.
        marker: char,
    },
    /// `~~strike~~`.
    Strike {
        /// Inner text between the markers.
        inner: &'a str,
    },
    /// `` `code` ``.
    Code {
        /// Inner text between the backticks.
        inner: &'a str,
    },
    /// `[text](url)` — standard markdown link.
    Link {
        /// Anchor text shown to the user.
        text: &'a str,
        /// URL target.
        url: &'a str,
    },
    /// `((blk-XXXXXX))` — inline reference to another block.
    ///
    /// The `handle` is the short, stable id persisted in the sidecar
    /// (see [`crate::sidecar::derive_ref_handle`]). The token carries
    /// the full handle including the `blk-` prefix so UI consumers can
    /// trust it as the lookup key without re-parsing.
    BlockRef {
        /// Full handle, e.g. `"blk-r6s4a1"`.
        handle: &'a str,
    },
    /// `!((blk-XXXXXX))` — embed: render the referenced block expanded
    /// (its `text` plus subtree) inline instead of as a link.
    ///
    /// Mirrors markdown image syntax (`![alt](url)`) where `!` means
    /// "expand". UI consumers render an Embed by resolving `handle`
    /// through [`crate::index::WorkspaceIndex::resolve_block_ref`]
    /// and drawing the result's `text` + `children`.
    Embed {
        /// Full handle, e.g. `"blk-r6s4a1"`.
        handle: &'a str,
    },
}

/// What `ref_at_cursor` resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    /// `[[name]]` — page reference (the disk path is `slugify(name)`).
    Page(String),
    /// `[[YYYY-MM-DD]]` — journal date reference.
    Journal(NaiveDate),
    /// `#name` — tag (resolves to a page with same name).
    Tag(String),
    /// `((blk-XXXXXX))` — block reference (lookup key into
    /// [`crate::index::WorkspaceIndex`]).
    Block(String),
}

/// Tokenize inline block content.
///
/// Greedy left-to-right scan. Plain text accumulates between recognized
/// constructs and emerges as a single [`InlineTok::Plain`] run.
pub fn tokenize(text: &str) -> Vec<InlineTok<'_>> {
    let mut out = Vec::new();
    let mut plain_start = 0usize;
    let mut idx = 0usize;

    while idx < text.len() {
        if let Some((tok, consumed)) = match_one(&text[idx..]) {
            if idx > plain_start {
                out.push(InlineTok::Plain(&text[plain_start..idx]));
            }
            out.push(tok);
            idx += consumed;
            plain_start = idx;
        } else {
            let ch = text[idx..]
                .chars()
                .next()
                .expect("idx < text.len() implies a next char");
            idx += ch.len_utf8();
        }
    }
    if plain_start < text.len() {
        out.push(InlineTok::Plain(&text[plain_start..]));
    }
    out
}

/// If `char_index` falls inside a `[[ref]]`, `#tag`, or `[[date]]` token
/// in `text`, return the corresponding [`RefTarget`]. Otherwise `None`.
pub fn ref_at_cursor(text: &str, char_index: usize) -> Option<RefTarget> {
    let cursor_byte = byte_index_for_char(text, char_index);

    // Scan `[[...]]` ranges.
    let mut search = 0usize;
    while let Some(rel_open) = text[search..].find("[[") {
        let abs_open = search + rel_open;
        let inner_start = abs_open + 2;
        let Some(rel_close) = text[inner_start..].find("]]") else {
            break;
        };
        let inner_end = inner_start + rel_close;
        let abs_close_end = inner_end + 2;
        if cursor_byte >= abs_open && cursor_byte <= abs_close_end {
            let inner = &text[inner_start..inner_end];
            if let Ok(date) = NaiveDate::parse_from_str(inner, "%Y-%m-%d") {
                return Some(RefTarget::Journal(date));
            }
            return Some(RefTarget::Page(inner.to_string()));
        }
        search = abs_close_end;
    }

    // Scan `((blk-...))` ranges. A preceding `!` (embed form) widens
    // the match by one byte so a cursor sitting on `!` still resolves
    // to the same target.
    //
    // Bug fix: when the candidate handle fails validation we advance
    // by ONE byte (not past the closing `))`) so an overlapping valid
    // handle still gets a chance. Example: `((((blk-x))))` — the
    // outer `((` captures `((blk-x` (invalid). Skipping to the first
    // `))` would step past the real `((blk-x))` at offset 2.
    let mut search = 0usize;
    while let Some(rel_open) = text[search..].find("((") {
        let abs_open = search + rel_open;
        let inner_start = abs_open + 2;
        let Some(rel_close) = text[inner_start..].find("))") else {
            break;
        };
        let inner_end = inner_start + rel_close;
        let abs_close_end = inner_end + 2;
        let handle = &text[inner_start..inner_end];
        if !is_valid_block_handle(handle) {
            search = abs_open + 1;
            continue;
        }
        let starts_at = if abs_open > 0 && text.as_bytes()[abs_open - 1] == b'!' {
            abs_open - 1
        } else {
            abs_open
        };
        if cursor_byte >= starts_at && cursor_byte <= abs_close_end {
            return Some(RefTarget::Block(handle.to_string()));
        }
        search = abs_close_end;
    }

    // Scan `#tag` ranges.
    let mut idx = 0usize;
    while idx < text.len() {
        if text[idx..].starts_with('#') {
            let after = &text[idx + 1..];
            let mut tag_byte_end = 0usize;
            for (rel, ch) in after.char_indices() {
                if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
                    tag_byte_end = rel + ch.len_utf8();
                } else {
                    break;
                }
            }
            if tag_byte_end > 0 {
                let abs_end = idx + 1 + tag_byte_end;
                if cursor_byte >= idx && cursor_byte <= abs_end {
                    let name = &text[idx + 1..abs_end];
                    return Some(RefTarget::Tag(name.to_string()));
                }
                idx = abs_end;
                continue;
            }
        }
        let ch = text[idx..].chars().next()?;
        idx += ch.len_utf8();
    }

    None
}

/// Convert a char index (0-based) into the corresponding byte offset.
///
/// Returns `s.len()` when the char index is at or past the end. Always
/// safe to pass into `s.split_at(...)`.
pub fn byte_index_for_char(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

// --- private matchers ----------------------------------------------------

fn match_one(s: &str) -> Option<(InlineTok<'_>, usize)> {
    if let Some(out) = try_page_ref(s) {
        return Some(out);
    }
    // `try_embed` MUST be checked before `try_block_ref`: the embed
    // form starts with `!` and contains a `((handle))` inside, and we
    // want the whole `!((handle))` consumed as one token instead of
    // a stray `Plain("!")` followed by a `BlockRef`.
    if let Some(out) = try_embed(s) {
        return Some(out);
    }
    if let Some(out) = try_block_ref(s) {
        return Some(out);
    }
    if let Some(out) = try_bold(s) {
        return Some(out);
    }
    if let Some(out) = try_bold_under(s) {
        return Some(out);
    }
    if let Some(out) = try_strike(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_star(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_under(s) {
        return Some(out);
    }
    if let Some(out) = try_code(s) {
        return Some(out);
    }
    if let Some(out) = try_md_link(s) {
        return Some(out);
    }
    if let Some(out) = try_tag(s) {
        return Some(out);
    }
    None
}

fn try_page_ref(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("[[")?;
    let close = rest.find("]]")?;
    let name = &rest[..close];
    if name.is_empty() || name.contains('\n') {
        return None;
    }
    Some((InlineTok::PageRef { name }, 2 + close + 2))
}

/// `!((blk-XXXXXX))` — block embed.
///
/// Markdown-image-shaped (`!((handle))` mirrors `![alt](url)`).
/// Strict on the inner handle for the same reason
/// [`try_block_ref`] is: arbitrary `!((..))` in prose must not be
/// silently rewritten as an embed.
fn try_embed(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("!((")?;
    let close = rest.find("))")?;
    let handle = &rest[..close];
    if !is_valid_block_handle(handle) {
        return None;
    }
    // Consumed: `!` (1) + `((` (2) + handle + `))` (2).
    Some((InlineTok::Embed { handle }, 1 + 2 + close + 2))
}

/// `((blk-XXXXXX))` — Roam-style block reference.
///
/// The handle must look like a valid one: starts with `blk-`, followed
/// by 1 or more ASCII-alphanumeric lowercase characters. Anything else
/// falls back to `Plain` so plain prose using `((..))` for parentheticals
/// is not silently rewritten.
fn try_block_ref(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("((")?;
    let close = rest.find("))")?;
    let handle = &rest[..close];
    if !is_valid_block_handle(handle) {
        return None;
    }
    Some((InlineTok::BlockRef { handle }, 2 + close + 2))
}

/// Validate a `((..))` payload as a block ref handle.
///
/// Conservative on purpose: the moment we accept arbitrary content
/// between `((` and `))`, prose like "look here ((really))" gets
/// rewritten as a broken reference. Loose validation is worse than no
/// recognition. Keep this aligned with [`crate::sidecar::derive_ref_handle`].
pub fn is_valid_block_handle(handle: &str) -> bool {
    let Some(tail) = handle.strip_prefix(crate::sidecar::REF_HANDLE_PREFIX) else {
        return false;
    };
    if tail.is_empty() {
        return false;
    }
    tail.chars()
        .all(|c| c.is_ascii_alphanumeric() && !c.is_ascii_uppercase())
}

fn try_bold(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("**")?;
    let close = rest.find("**")?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') || inner.starts_with('*') {
        return None;
    }
    Some((InlineTok::Bold { inner }, 2 + close + 2))
}

/// `__bold__` — CommonMark treats double-underscore the same as `**`:
/// strong emphasis (bold), not italic. Must be checked **before**
/// [`try_italic_under`] so the double form wins.
fn try_bold_under(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("__")?;
    let close = rest.find("__")?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') || inner.starts_with('_') {
        return None;
    }
    Some((InlineTok::Bold { inner }, 2 + close + 2))
}

fn try_strike(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("~~")?;
    let close = rest.find("~~")?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    Some((InlineTok::Strike { inner }, 2 + close + 2))
}

fn try_italic_star(s: &str) -> Option<(InlineTok<'_>, usize)> {
    if s.starts_with("**") {
        return None;
    }
    let rest = s.strip_prefix('*')?;
    let mut iter = rest.char_indices().peekable();
    let close = loop {
        let (i, c) = iter.next()?;
        if c == '*' {
            if iter.peek().map(|(_, c2)| *c2) == Some('*') {
                return None;
            }
            break i;
        }
    };
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    Some((InlineTok::Italic { inner, marker: '*' }, 1 + close + 1))
}

fn try_italic_under(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('_')?;
    let close = rest.find('_')?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    Some((InlineTok::Italic { inner, marker: '_' }, 1 + close + 1))
}

fn try_code(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('`')?;
    let close = rest.find('`')?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    Some((InlineTok::Code { inner }, 1 + close + 1))
}

fn try_md_link(s: &str) -> Option<(InlineTok<'_>, usize)> {
    if s.starts_with("[[") {
        return None;
    }
    let rest = s.strip_prefix('[')?;
    let bracket_close = rest.find(']')?;
    let text = &rest[..bracket_close];
    let after_bracket = bracket_close + 1;
    if !rest[after_bracket..].starts_with('(') {
        return None;
    }
    let paren_rest = &rest[after_bracket + 1..];
    let paren_close = paren_rest.find(')')?;
    let url = &paren_rest[..paren_close];
    if text.is_empty() || text.contains('\n') || url.contains('\n') {
        return None;
    }
    let consumed = 1 + after_bracket + 1 + paren_close + 1;
    Some((InlineTok::Link { text, url }, consumed))
}

fn try_tag(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('#')?;
    let mut tag_byte_end = 0usize;
    for (rel, ch) in rest.char_indices() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '/' {
            tag_byte_end = rel + ch.len_utf8();
        } else {
            break;
        }
    }
    if tag_byte_end == 0 {
        return None;
    }
    Some((
        InlineTok::Tag {
            name: &rest[..tag_byte_end],
        },
        1 + tag_byte_end,
    ))
}
