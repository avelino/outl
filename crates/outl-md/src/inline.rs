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
//! - `**bold**` / `*italic*` / `_italic_` / `~~strike~~` / `==highlight==` / `` `code` ``.
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
    /// `==highlight==` — the target of Roam's `^^highlight^^` on import.
    Highlight {
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
    /// `![alt](url)` — markdown image / embedded asset.
    ///
    /// Mirrors [`InlineTok::Link`] with a leading `!`. `url` is either a
    /// workspace-relative `assets/<hash>.<ext>` path (imported / uploaded
    /// files, resolved against the workspace root by the client) or a
    /// remote `http(s)` URL. `alt` may be empty (`![](url)` is valid).
    /// The client decides how to render by inspecting `url` — inline
    /// `<img>` for image extensions, a viewer / chip for other files.
    Image {
        /// Alt text (may be empty).
        alt: &'a str,
        /// Image / asset target — relative `assets/…` path or remote URL.
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
    /// `==highlight==`.
    Highlight {
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
    /// `![alt](url)` image / embedded asset. `href` is a
    /// workspace-relative `assets/<hash>.<ext>` path or a remote URL;
    /// the client renders an `<img>` for image extensions and a viewer
    /// / file chip for other kinds.
    Image {
        /// Alt text (may be empty).
        alt: String,
        /// Image / asset target.
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
            InlineTok::Highlight { inner } => InlineToken::Highlight {
                inner: inner.iter().map(InlineToken::from_borrowed).collect(),
            },
            InlineTok::Code { inner } => InlineToken::Code {
                value: (*inner).to_owned(),
            },
            InlineTok::Link { text, url } => InlineToken::Link {
                value: (*text).to_owned(),
                href: (*url).to_owned(),
            },
            InlineTok::Image { alt, url } => InlineToken::Image {
                alt: (*alt).to_owned(),
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
        // The char just before `idx` decides whether a `_` here can open
        // emphasis (CommonMark forbids intra-word `_`). `None` at the
        // start of the run counts as "not alphanumeric" → can open.
        let prev = text[..idx].chars().next_back();
        if let Some((tok, consumed)) = match_one(&text[idx..], prev) {
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

// Cursor introspection (`ref_at_cursor`, `link_at_cursor`,
// `byte_index_for_char`) lives in the sibling [`crate::cursor`] module and
// is re-exported here so `outl_md::inline::{…}` paths keep resolving.
pub use crate::cursor::{byte_index_for_char, link_at_cursor, ref_at_cursor};

// --- private matchers ----------------------------------------------------

fn match_one(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
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
    // `try_image` shares the `!` opener with `try_embed` (`![` vs `!((`),
    // so it sits next to it and, crucially, before `try_md_link` — the
    // bare `[` link — so `![alt](url)` isn't split into `!` + a link.
    if let Some(out) = try_image(s) {
        return Some(out);
    }
    if let Some(out) = try_block_ref(s) {
        return Some(out);
    }
    if let Some(out) = try_bold(s) {
        return Some(out);
    }
    if let Some(out) = try_bold_under(s, prev) {
        return Some(out);
    }
    if let Some(out) = try_strike(s) {
        return Some(out);
    }
    if let Some(out) = try_highlight(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_star(s) {
        return Some(out);
    }
    if let Some(out) = try_italic_under(s, prev) {
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
            InlineTok::Highlight { inner } => {
                out.push_str("==");
                out.push_str(&inline_to_source(inner));
                out.push_str("==");
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
            InlineTok::Image { alt, url } => {
                out.push_str("![");
                out.push_str(alt);
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
fn try_bold_under(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
    // Same intra-word rule as italic: `__` preceded by an alphanumeric
    // doesn't open strong emphasis.
    if prev.is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let rest = s.strip_prefix("__")?;
    let close = closing_underscore(rest, 2)?;
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

/// `==highlight==` — the on-disk form of Roam's `^^highlight^^` after
/// import. Unlike [`try_strike`], the inner span may not begin or end
/// with a space: `^^…^^` always wraps text tightly, and the extra rule
/// keeps a stray comparison operator (`count == total == 0`) from being
/// swallowed as a highlight.
fn try_highlight(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("==")?;
    let close = rest.find("==")?;
    let inner_str = &rest[..close];
    if inner_str.is_empty()
        || inner_str.contains('\n')
        || inner_str.starts_with(' ')
        || inner_str.ends_with(' ')
    {
        return None;
    }
    Some((
        InlineTok::Highlight {
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

fn try_italic_under(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
    // CommonMark: `_` does not open emphasis intra-word. A `_`
    // immediately preceded by an alphanumeric (`inc_lag1`,
    // `prod.ml_atendimento`, `databricks_2_train`) is a literal
    // underscore, never an italic opener — `*` is the intra-word marker.
    if prev.is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let rest = s.strip_prefix('_')?;
    let close = closing_underscore(rest, 1)?;
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

/// Find the byte offset (within `rest`, the text after the opening
/// run) of a closing underscore run of `run_len` (`1` for italic, `2`
/// for bold) that is **not** intra-word — i.e. not directly followed by
/// an alphanumeric. Intra-word `_`s are skipped so `_a_b_` still closes
/// at the last underscore, and identifiers like `chamados_lag1` never
/// supply a spurious closer. Returns `None` if no valid closer exists.
fn closing_underscore(rest: &str, run_len: usize) -> Option<usize> {
    let needle = if run_len == 2 { "__" } else { "_" };
    let mut search = 0usize;
    loop {
        let rel = rest[search..].find(needle)?;
        let abs = search + rel;
        let after = &rest[abs + run_len..];
        if after.chars().next().is_some_and(|c| c.is_alphanumeric()) {
            // Intra-word underscore — not a valid closer; keep scanning.
            search = abs + 1;
            continue;
        }
        break Some(abs);
    }
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

pub(crate) fn try_md_link(s: &str) -> Option<(InlineTok<'_>, usize)> {
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

/// `![alt](url)` — markdown image / embedded asset.
///
/// Mirrors [`try_md_link`] with a leading `!`. Unlike a link, the alt
/// text may be empty (`![](url)` is valid CommonMark); the url must be
/// non-empty. `![[…]]` (Obsidian wiki-embed) does not match — the inner
/// `[` is consumed as the first alt char and the `](` shape then fails,
/// leaving that form for the wiki-link rewriter at import time.
fn try_image(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix("![")?;
    let bracket_close = rest.find(']')?;
    let alt = &rest[..bracket_close];
    let after_bracket = bracket_close + 1;
    if !rest[after_bracket..].starts_with('(') {
        return None;
    }
    let paren_rest = &rest[after_bracket + 1..];
    let paren_close = paren_rest.find(')')?;
    let url = &paren_rest[..paren_close];
    if url.is_empty() || alt.contains('\n') || url.contains('\n') {
        return None;
    }
    // Consumed: `![` + alt + `]` + `(` + url + `)`.
    let consumed = 2 + after_bracket + 1 + paren_close + 1;
    Some((InlineTok::Image { alt, url }, consumed))
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

// Tests for the owned `InlineToken` / `tokenize_owned` wire form live
// in `tests/tokenize_owned.rs` (moved out to keep this module under the
// file-size-guard).

/// Flatten inline markup into the plain text a human reads.
///
/// The single owner of "block text without the syntax". Built for
/// notification bodies — a lock screen showing
/// `ship it [[2026-12-12]] #fup` with the brackets and hash intact
/// reads like a bug — and equally right for any surface that needs the
/// prose without the markers (a11y labels, plain-text export, a search
/// snippet).
///
/// Rules, chosen so the result reads as the author meant it:
///
/// - `[[page]]` and `#tag` keep their name, drop their punctuation.
///   `[[@joão]]` becomes `@joão`: the `@` is part of how the person is
///   written, the brackets are not.
/// - Emphasis (`**bold**`, `_italic_`, `~~strike~~`, `==highlight==`)
///   keeps its contents and drops the markers, recursively.
/// - `[text](url)` keeps the anchor text; the URL is noise out loud.
///   `![alt](url)` keeps the alt for the same reason, and yields
///   nothing when the alt is empty.
/// - `:shortcode:` becomes its glyph, since that's what it renders as.
/// - `((blk-…))` / `!((blk-…))` resolve to nothing here. The handle is
///   an id, not prose, and this function has no index to look it up
///   in — a caller that wants the source text resolves the embed first.
pub fn plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    push_plain(&tokenize(raw), &mut out);
    // Collapsing runs keeps a dropped block-ref from leaving a double
    // space behind ("see ((blk-x)) later" -> "see later").
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_plain(tokens: &[InlineTok<'_>], out: &mut String) {
    for tok in tokens {
        match tok {
            InlineTok::Plain(s) => out.push_str(s),
            InlineTok::PageRef { name } => out.push_str(name),
            InlineTok::Tag { name } => out.push_str(name),
            InlineTok::Code { inner } => out.push_str(inner),
            InlineTok::Link { text, .. } => out.push_str(text),
            InlineTok::Image { alt, .. } => out.push_str(alt),
            InlineTok::Bold { inner }
            | InlineTok::Italic { inner, .. }
            | InlineTok::Strike { inner }
            | InlineTok::Highlight { inner } => push_plain(inner, out),
            InlineTok::Emoji { shortcode } => {
                match crate::emoji::shortcode_to_unicode(shortcode) {
                    Some(glyph) => out.push_str(glyph),
                    // Unreachable in practice (the matcher only emits
                    // a token the catalog knows), but falling back to
                    // the shortcode beats dropping the word.
                    None => out.push_str(shortcode),
                }
            }
            // An id, not prose, and there's no index in scope to
            // resolve it against.
            InlineTok::BlockRef { .. } | InlineTok::Embed { .. } => {}
        }
    }
}

#[cfg(test)]
mod plain_text_tests {
    use super::plain_text;

    #[test]
    fn refs_and_tags_keep_the_name_and_lose_the_punctuation() {
        assert_eq!(
            plain_text("ship it [[2026-12-12]] #fup"),
            "ship it 2026-12-12 fup"
        );
    }

    #[test]
    fn a_mention_keeps_its_at_sign() {
        // The `@` is how the person is written; the brackets are not.
        assert_eq!(plain_text("ping [[@joão]] today"), "ping @joão today");
    }

    #[test]
    fn emphasis_keeps_the_words_and_drops_the_markers() {
        assert_eq!(
            plain_text("**call** the _bank_ ~~today~~ ==now=="),
            "call the bank today now"
        );
    }

    #[test]
    fn nested_emphasis_still_flattens() {
        assert_eq!(plain_text("**see [[avelino]] now**"), "see avelino now");
    }

    #[test]
    fn a_link_keeps_its_anchor_not_its_url() {
        assert_eq!(
            plain_text("read [the RFC](https://example.com/very/long)"),
            "read the RFC"
        );
    }

    #[test]
    fn block_refs_drop_out_without_leaving_a_gap() {
        assert_eq!(plain_text("see ((blk-r6s4a1)) later"), "see later");
        assert_eq!(plain_text("see !((blk-r6s4a1)) later"), "see later");
    }

    #[test]
    fn code_keeps_its_contents() {
        assert_eq!(plain_text("run `cargo test` now"), "run cargo test now");
    }

    #[test]
    fn plain_prose_is_unchanged() {
        assert_eq!(plain_text("call the bank"), "call the bank");
    }

    #[test]
    fn an_empty_body_stays_empty() {
        assert_eq!(plain_text(""), "");
    }
}
