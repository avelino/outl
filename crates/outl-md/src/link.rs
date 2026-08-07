//! The CommonMark bracket-paren family: `[text](url)` and
//! `![alt](url)`.
//!
//! Both scan the same shape (`[`…`]` immediately followed by `(`…`)`),
//! differ only in the leading `!`, and are the two tokens whose target
//! can leave the workspace (a remote URL) or point at an imported file
//! (`assets/<hash>.<ext>`). `inline::match_one` owns the order they are
//! tried in relative to `((blk-…))` / `!((blk-…))`.
//!
//! [`try_md_link`] is also the matcher `cursor::link_at_cursor` reuses,
//! so "what is a link" stays owned in exactly one place.

use crate::token::InlineTok;

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
pub(crate) fn try_image(s: &str) -> Option<(InlineTok<'_>, usize)> {
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
