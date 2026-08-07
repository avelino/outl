//! Delimiter-pair spans: `**bold**`, `__bold__`, `*italic*`,
//! `_italic_`, `~~strike~~`, `==highlight==`, `` `code` ``.
//!
//! One matcher per marker pair, each with the same contract as every
//! other matcher in the tokenizer: given the remainder of the source
//! starting at the candidate opener, return the token plus the number
//! of **bytes** consumed, or `None` to let the next matcher (and
//! ultimately `Plain`) have it. [`crate::inline::match_one`] owns the
//! order they are tried in.
//!
//! All of these re-tokenize their inner span (so `**[[avelino]]**`
//! keeps the ref recognizable) except `` `code` ``, whose payload is
//! opaque by definition but which is otherwise the same shape.
//!
//! The subtle rule that lives here is CommonMark's intra-word
//! underscore restriction — see [`closing_underscore`].

use crate::inline::tokenize;
use crate::token::InlineTok;

pub(crate) fn try_bold(s: &str) -> Option<(InlineTok<'_>, usize)> {
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
pub(crate) fn try_bold_under(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
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

pub(crate) fn try_strike(s: &str) -> Option<(InlineTok<'_>, usize)> {
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
pub(crate) fn try_highlight(s: &str) -> Option<(InlineTok<'_>, usize)> {
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

pub(crate) fn try_italic_star(s: &str) -> Option<(InlineTok<'_>, usize)> {
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

pub(crate) fn try_italic_under(s: &str, prev: Option<char>) -> Option<(InlineTok<'_>, usize)> {
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

pub(crate) fn try_code(s: &str) -> Option<(InlineTok<'_>, usize)> {
    let rest = s.strip_prefix('`')?;
    let close = rest.find('`')?;
    let inner = &rest[..close];
    if inner.is_empty() || inner.contains('\n') {
        return None;
    }
    Some((InlineTok::Code { inner }, 1 + close + 1))
}
