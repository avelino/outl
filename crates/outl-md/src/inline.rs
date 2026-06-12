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
use serde::{Deserialize, Serialize};

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
    /// `**bold**`. The inner span is re-tokenized so refs / tags /
    /// block-refs nested inside the markers render with their own
    /// styling instead of falling through as plain text. Same for
    /// `Italic` and `Strike` below.
    Bold {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
    },
    /// `*italic*` or `_italic_`. `marker` is the literal delimiter used.
    Italic {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
        /// Either `'*'` or `'_'`.
        marker: char,
    },
    /// `~~strike~~`.
    Strike {
        /// Recursively-tokenized contents between the markers.
        inner: Vec<InlineTok<'a>>,
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
    /// `:shortcode:` — GitHub gemoji shortcode.
    ///
    /// The borrowed form carries only the shortcode (without the `:`s);
    /// the glyph is resolved at conversion time by
    /// [`crate::emoji::shortcode_to_unicode`]. The matcher only emits
    /// this token when the catalog recognizes the shortcode — unknown
    /// `:foo:` runs stay [`InlineTok::Plain`] so prose like
    /// `meeting at 14:00 : ok?` is not silently rewritten.
    Emoji {
        /// Shortcode without the surrounding `:`s (e.g. `"tada"`).
        shortcode: &'a str,
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

/// Owned, serializable mirror of [`InlineTok`].
///
/// `InlineTok` borrows from the source string and is cheap to use
/// inside Rust. Anything that has to cross a serialization boundary
/// (a Tauri command's return value, a `BlockNode` DTO sent to a
/// frontend, a `Backlink.block_tokens` payload) needs owned strings
/// and a Serde-friendly shape. `InlineToken` is that shape.
///
/// The JSON form matches the schema mobile's TypeScript renderer
/// consumes one-for-one. Adding a variant in `InlineTok` requires
/// adding the same variant here plus the conversion in
/// [`InlineToken::from_borrowed`] in the same change — otherwise the
/// new variant silently degrades to `Plain` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InlineToken {
    /// Bare text with no formatting.
    Plain {
        /// Verbatim text.
        value: String,
    },
    /// `**bold**`. The inner span is re-tokenized so wiki-refs,
    /// tags, and block-refs nested inside the markers stay
    /// recognizable (e.g. `**[[avelino]]**` renders the bold `**` and
    /// the ref `[[avelino]]` as separate styled tokens, not a single
    /// flat string).
    Bold {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `*italic*` or `_italic_`. The TS renderer collapses both
    /// markers into one variant since the literal delimiter is
    /// purely cosmetic on the browser side; Rust consumers that need
    /// the marker keep using [`InlineTok`] directly.
    Italic {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `~~strike~~`.
    Strike {
        /// Tokens of the inner span.
        inner: Vec<InlineToken>,
    },
    /// `` `code` ``.
    Code {
        /// Inner text between the backticks.
        value: String,
    },
    /// `[text](url)` link.
    Link {
        /// Anchor text shown to the user.
        value: String,
        /// URL target.
        href: String,
    },
    /// `[[name]]` page reference.
    Ref {
        /// Page name (display form, kept verbatim).
        value: String,
    },
    /// `#tag`. `value` includes the leading `#` so the frontend can
    /// render it as a single token without re-prefixing.
    Tag {
        /// Tag string including the `#` prefix (e.g. `"#project"`).
        value: String,
    },
    /// `((blk-XXXXXX))` block reference.
    #[serde(rename = "blockref")]
    BlockRef {
        /// Full handle including the `blk-` prefix.
        value: String,
    },
    /// `!((blk-XXXXXX))` block embed.
    Embed {
        /// Full handle including the `blk-` prefix.
        value: String,
    },
    /// `:shortcode:` GitHub gemoji shortcode.
    ///
    /// `shortcode` is the literal text between the `:`s
    /// (e.g. `"tada"`); `glyph` is the resolved unicode codepoint
    /// (e.g. `"🎉"`). Clients render `glyph` and surface `shortcode`
    /// for hover / `aria-label`. If the catalog ever misses (should
    /// not happen — the tokenizer pre-validates) the client should
    /// fall back to rendering `:${shortcode}:` literal.
    Emoji {
        /// Shortcode without the surrounding `:`s (e.g. `"tada"`).
        shortcode: String,
        /// Resolved unicode glyph (e.g. `"🎉"`).
        glyph: String,
    },
}

impl InlineToken {
    /// Convert a borrowed [`InlineTok`] into the owned, serializable
    /// form. The conversion is total — every variant maps 1:1.
    pub fn from_borrowed(tok: &InlineTok<'_>) -> Self {
        match tok {
            InlineTok::Plain(s) => InlineToken::Plain {
                value: (*s).to_owned(),
            },
            InlineTok::Bold { inner } => InlineToken::Bold {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Italic { inner, .. } => InlineToken::Italic {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Strike { inner } => InlineToken::Strike {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Code { inner } => InlineToken::Code {
                value: (*inner).to_owned(),
            },
            InlineTok::Link { text, url } => InlineToken::Link {
                value: (*text).to_owned(),
                href: (*url).to_owned(),
            },
            InlineTok::PageRef { name } => InlineToken::Ref {
                value: (*name).to_owned(),
            },
            InlineTok::Tag { name } => InlineToken::Tag {
                value: format!("#{name}"),
            },
            InlineTok::BlockRef { handle } => InlineToken::BlockRef {
                value: (*handle).to_owned(),
            },
            InlineTok::Embed { handle } => InlineToken::Embed {
                value: (*handle).to_owned(),
            },
            InlineTok::Emoji { shortcode } => InlineToken::Emoji {
                shortcode: (*shortcode).to_owned(),
                // The tokenizer only emits `Emoji` when the catalog
                // resolves — so this `unwrap_or("")` is a defensive
                // landing pad, not a code path we expect to hit.
                // Empty `glyph` lets the frontend fall back to the
                // literal `:shortcode:` form without crashing.
                glyph: crate::emoji::shortcode_to_unicode(shortcode)
                    .unwrap_or("")
                    .to_owned(),
            },
        }
    }
}

/// Tokenize `text` directly into the owned, serializable form. This
/// is the call backend DTOs use when they need to ship tokens to a
/// frontend — single source of truth for inline markdown parsing
/// across every client, no parallel TS / Swift / Kotlin tokenizer
/// to keep in sync.
pub fn tokenize_owned(text: &str) -> Vec<InlineToken> {
    tokenize(text)
        .iter()
        .map(InlineToken::from_borrowed)
        .collect()
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
    // `try_emoji` sits between `try_md_link` and `try_tag` — `:` does
    // not overlap with any other matcher's opener, so the slot is
    // chosen for readability, not precedence.
    if let Some(out) = try_emoji(s) {
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
/// recognition. Keep this aligned with [`crate::sidecar::derive_ref_handle`]
/// — handles are `blk-` plus at least [`crate::sidecar::REF_HANDLE_TAIL_LEN`]
/// lowercase ASCII alphanumerics; collision expansion only ever makes
/// them longer, never shorter.
pub fn is_valid_block_handle(handle: &str) -> bool {
    let Some(tail) = handle.strip_prefix(crate::sidecar::REF_HANDLE_PREFIX) else {
        return false;
    };
    if tail.chars().count() < crate::sidecar::REF_HANDLE_TAIL_LEN {
        return false;
    }
    tail.chars()
        .all(|c| c.is_ascii_alphanumeric() && !c.is_ascii_uppercase())
}

/// Re-emit a tokenized inline span back as the markdown source it
/// came from.
///
/// Bold / italic / strike now carry recursively-tokenized inners
/// (`Vec<InlineTok>`), so consumers that used to call
/// `inner.to_string()` on a `&str` need a small helper to reconstruct
/// the literal source. Renderers that already iterate
/// `Vec<InlineTok>` to dispatch per-variant styling don't need this —
/// it's specifically for surfaces that want the whole inner span as
/// one styled string.
pub fn inline_to_source(toks: &[InlineTok<'_>]) -> String {
    let mut out = String::new();
    for tok in toks {
        match tok {
            InlineTok::Plain(s) => out.push_str(s),
            InlineTok::PageRef { name } => {
                out.push_str("[[");
                out.push_str(name);
                out.push_str("]]");
            }
            InlineTok::Tag { name } => {
                out.push('#');
                out.push_str(name);
            }
            InlineTok::Bold { inner } => {
                out.push_str("**");
                out.push_str(&inline_to_source(inner));
                out.push_str("**");
            }
            InlineTok::Italic { inner, marker } => {
                out.push(*marker);
                out.push_str(&inline_to_source(inner));
                out.push(*marker);
            }
            InlineTok::Strike { inner } => {
                out.push_str("~~");
                out.push_str(&inline_to_source(inner));
                out.push_str("~~");
            }
            InlineTok::Code { inner } => {
                out.push('`');
                out.push_str(inner);
                out.push('`');
            }
            InlineTok::Link { text, url } => {
                out.push('[');
                out.push_str(text);
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            InlineTok::BlockRef { handle } => {
                out.push_str("((");
                out.push_str(handle);
                out.push_str("))");
            }
            InlineTok::Embed { handle } => {
                out.push_str("!((");
                out.push_str(handle);
                out.push_str("))");
            }
            InlineTok::Emoji { shortcode } => {
                out.push(':');
                out.push_str(shortcode);
                out.push(':');
            }
        }
    }
    out
}

fn try_bold(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("**")?;
    let close = rest.find("**")?;
    let inner_str = &rest[..close];
    if inner_str.is_empty() || inner_str.contains('\n') || inner_str.starts_with('*') {
        return None;
    }
    Some((
        InlineTok::Bold {
            inner: tokenize(inner_str),
        },
        2 + close + 2,
    ))
}

/// `__bold__` — CommonMark treats double-underscore the same as `**`:
/// strong emphasis (bold), not italic. Must be checked **before**
/// [`try_italic_under`] so the double form wins.
fn try_bold_under(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("__")?;
    let close = rest.find("__")?;
    let inner_str = &rest[..close];
    if inner_str.is_empty() || inner_str.contains('\n') || inner_str.starts_with('_') {
        return None;
    }
    Some((
        InlineTok::Bold {
            inner: tokenize(inner_str),
        },
        2 + close + 2,
    ))
}

fn try_strike(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("~~")?;
    let close = rest.find("~~")?;
    let inner_str = &rest[..close];
    if inner_str.is_empty() || inner_str.contains('\n') {
        return None;
    }
    Some((
        InlineTok::Strike {
            inner: tokenize(inner_str),
        },
        2 + close + 2,
    ))
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
    let inner_str = &rest[..close];
    if inner_str.is_empty() || inner_str.contains('\n') {
        return None;
    }
    Some((
        InlineTok::Italic {
            inner: tokenize(inner_str),
            marker: '*',
        },
        1 + close + 1,
    ))
}

fn try_italic_under(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('_')?;
    let close = rest.find('_')?;
    let inner_str = &rest[..close];
    if inner_str.is_empty() || inner_str.contains('\n') {
        return None;
    }
    Some((
        InlineTok::Italic {
            inner: tokenize(inner_str),
            marker: '_',
        },
        1 + close + 1,
    ))
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

/// `:shortcode:` — GitHub gemoji shortcode.
///
/// Strict on both ends:
/// - the shape `[a-z0-9_+-]+` is pinned to gemoji syntax (covers `:+1:`,
///   `:-1:`, `:smile_cat:`, `:100:`) — any non-shortcode char (incl.
///   uppercase, whitespace, `/`, `.`, `:`) terminates the run and forces
///   a closing `:` to be next, otherwise we bail.
/// - the catalog gate (`shortcode_to_unicode`) means we never tokenize
///   `:foo:` unless `foo` is a known emoji. Prose like
///   `meeting at 14:00 :` stays plain.
///
/// URL boundary fall-out: `https://example.com:8080/api`, `ftp://host:21`,
/// `mailto:foo@bar.com`, `git@github.com:avelino/outl.git` all fail this
/// matcher naturally — either the inner run contains an invalid char
/// (`/`, `.`, `@`) or there is no closing `:`. No look-behind needed.
fn try_emoji(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix(':')?;
    let mut shortcode_byte_end = 0usize;
    for (rel, ch) in rest.char_indices() {
        if crate::emoji::is_valid_shortcode_char(ch) {
            shortcode_byte_end = rel + ch.len_utf8();
        } else {
            break;
        }
    }
    if shortcode_byte_end == 0 {
        return None;
    }
    let after = &rest[shortcode_byte_end..];
    if !after.starts_with(':') {
        return None;
    }
    let shortcode = &rest[..shortcode_byte_end];
    // Catalog gate: unknown shortcodes degrade to plain text.
    crate::emoji::shortcode_to_unicode(shortcode)?;
    // Consumed: opening `:` + shortcode + closing `:`.
    Some((InlineTok::Emoji { shortcode }, 1 + shortcode_byte_end + 1))
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

#[cfg(test)]
mod tokenize_owned_tests {
    use super::*;

    #[test]
    fn round_trips_every_variant_into_serializable_form() {
        // Block-ref handles need `REF_HANDLE_TAIL_LEN` chars after
        // `blk-` (6 today) to pass `is_valid_block_handle`; shorter
        // handles correctly degrade to plain text.
        let toks = tokenize_owned(
            "**b** *i* ~~s~~ `c` [t](u) [[p]] #tag ((blk-aaaaaa)) !((blk-bbbbbb)) :tada: tail",
        );
        // Spot-check shape (kind discriminant) and the `tag` prefixing,
        // since that's the one place `from_borrowed` does more than a
        // string copy.
        let kinds: Vec<&str> = toks
            .iter()
            .map(|t| match t {
                InlineToken::Plain { .. } => "plain",
                InlineToken::Bold { .. } => "bold",
                InlineToken::Italic { .. } => "italic",
                InlineToken::Strike { .. } => "strike",
                InlineToken::Code { .. } => "code",
                InlineToken::Link { .. } => "link",
                InlineToken::Ref { .. } => "ref",
                InlineToken::Tag { .. } => "tag",
                InlineToken::BlockRef { .. } => "blockref",
                InlineToken::Embed { .. } => "embed",
                InlineToken::Emoji { .. } => "emoji",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "bold", "plain", "italic", "plain", "strike", "plain", "code", "plain", "link",
                "plain", "ref", "plain", "tag", "plain", "blockref", "plain", "embed", "plain",
                "emoji", "plain",
            ],
        );
        // Tag value carries the leading `#` so the mobile renderer
        // doesn't have to re-prefix.
        let tag = toks
            .iter()
            .find_map(|t| match t {
                InlineToken::Tag { value } => Some(value.clone()),
                _ => None,
            })
            .expect("tokenize_owned should emit one Tag");
        assert_eq!(tag, "#tag");
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        // Replaces coverage from the deleted mobile `markdown.test.ts`
        // — the old TS tokenizer used to push a phantom `plain` run on
        // empty input. Pin the Rust behaviour so a future refactor
        // doesn't reintroduce it.
        assert!(tokenize_owned("").is_empty());
    }

    #[test]
    fn bare_text_is_one_plain_run() {
        let toks = tokenize_owned("tail");
        assert_eq!(toks.len(), 1);
        assert!(matches!(
            &toks[0],
            InlineToken::Plain { value } if value == "tail"
        ));
    }

    #[test]
    fn plain_text_after_last_match_survives() {
        // The deleted TS test "preserves trailing text after the last
        // match" guarded a tokenizer bug where the tail run got
        // dropped. Pin the same invariant on the Rust side.
        let toks = tokenize_owned("**bold** tail");
        let trailing = toks.last().expect("at least one token");
        assert!(
            matches!(trailing, InlineToken::Plain { value } if value == " tail"),
            "expected trailing Plain(\" tail\"), got {trailing:?}",
        );
    }

    /// The `> ` blockquote marker is block-level, not inline. It must
    /// not become a token: the inline tokenizer is called on a body
    /// that already had the prefix stripped by
    /// `outl_actions::quote::split_quote`. If the marker ever shows up
    /// inside the body (the user typed `"> > foo"` — single split, the
    /// inner `"> foo"` is the body), it stays Plain so the inline
    /// surface doesn't accidentally double-style it.
    #[test]
    fn quote_prefix_is_not_tokenized_as_an_inline() {
        let toks = tokenize_owned("> still plain");
        assert_eq!(toks.len(), 1);
        assert!(
            matches!(&toks[0], InlineToken::Plain { value } if value == "> still plain"),
            "expected the whole string as Plain, got {toks:?}"
        );
    }

    #[test]
    fn serde_json_kind_field_matches_mobile_dto() {
        // Mobile reads `kind` lowercase via Serde's
        // `rename_all = "lowercase"`. If we ever change the rename
        // policy, the iOS client silently goes to plain — this test
        // pins the wire shape.
        let toks = vec![
            InlineToken::Plain { value: "hi".into() },
            InlineToken::BlockRef {
                value: "blk-x1".into(),
            },
        ];
        let json = serde_json::to_string(&toks).unwrap();
        assert!(json.contains(r#""kind":"plain""#));
        assert!(json.contains(r#""kind":"blockref""#));
    }
}
