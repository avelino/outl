//! Low-level scanners shared by the markdown-dialect adapters.
//!
//! These are syntax-agnostic primitives (balanced-delimiter scans, UID
//! shape checks); what a matched region *means* stays in each
//! adapter's `inline.rs` — dialect knowledge never lives here.

/// Scan a balanced two-char-delimited region. `s` must start with
/// `open`; returns the total length including the closing delimiter,
/// or `None` when unbalanced.
pub(crate) fn balanced(s: &str, open: &str, close: &str) -> Option<usize> {
    debug_assert!(s.starts_with(open));
    let mut depth = 0usize;
    let mut j = 0usize;
    while j < s.len() {
        let r = &s[j..];
        if r.starts_with(open) {
            depth += 1;
            j += open.len();
        } else if r.starts_with(close) {
            depth -= 1;
            j += close.len();
            if depth == 0 {
                return Some(j);
            }
        } else {
            j += r.chars().next()?.len_utf8();
        }
    }
    None
}

/// Char-depth brace scan for `{{…}}` components whose body may nest
/// single-brace clauses (`{{[[query]]: {and: …}}}`). `s` must start
/// with `{{`; returns the total length through the brace that closes
/// the outer pair, or `None` when unbalanced.
pub(crate) fn balanced_braces(s: &str) -> Option<usize> {
    debug_assert!(s.starts_with("{{"));
    let mut depth = 0i32;
    for (j, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Source block UIDs are short alnum/`-`/`_` runs (Roam's 9-char ids,
/// Logseq's UUIDs). Rejecting anything else keeps prose
/// double-parentheticals (`((aside, note))`) as text.
pub(crate) fn is_uid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `[alias](target)` scanner with balanced brackets/parens. Returns
/// `(alias, target, total_len)`.
pub(crate) fn alias_link(s: &str) -> Option<(&str, &str, usize)> {
    debug_assert!(s.starts_with('['));
    let mut depth = 0usize;
    let mut close_bracket = None;
    for (j, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close_bracket = Some(j);
                    break;
                }
            }
            _ => {}
        }
    }
    let cb = close_bracket?;
    let after = &s[cb + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let mut pdepth = 0usize;
    for (j, c) in after.char_indices() {
        match c {
            '(' => pdepth += 1,
            ')' => {
                pdepth -= 1;
                if pdepth == 0 {
                    let alias = &s[1..cb];
                    let target = &after[1..j];
                    return Some((alias, target, cb + 1 + j + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Truncate for warning messages.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// `key:: value` → `(key, value)`, lower-cased key. Accepts both
/// `key:: value` and a bare `key::` with no value and no trailing space
/// (empty value), matching outl's own `parse_property_line`. The key
/// must be a bare word (alnum / `-` / `_` / `.`), so a key with spaces
/// or a `::` buried in prose (`foo::bar`, `see below::`) stays text.
///
/// Shared by the Logseq and Roam adapters: both surface properties as
/// `key:: value` lines and must classify them identically, or the same
/// note imported from two tools would diverge.
pub(crate) fn parse_prop_line(trimmed: &str) -> Option<(String, String)> {
    if let Some((k, v)) = trimmed.split_once(":: ") {
        let key = k.trim();
        if is_prop_key(key) {
            return Some((key.to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    // `key::` with no value and no trailing space (outl accepts this too).
    if let Some(key) = trimmed.strip_suffix("::") {
        let key = key.trim_end();
        if is_prop_key(key) {
            return Some((key.to_ascii_lowercase(), String::new()));
        }
    }
    None
}

/// A property key is a non-empty bare word: alnum / `-` / `_` / `.`.
fn is_prop_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// A block whose whole text is one fenced code block →
/// `(info-string, body)`. `None` when the text isn't a single
/// complete fence.
pub(crate) fn parse_whole_fence(text: &str) -> Option<(Option<String>, String)> {
    let t = text.trim();
    let first_line = t.lines().next()?;
    if !first_line.starts_with("```") {
        return None;
    }
    let rest: Vec<&str> = t.lines().skip(1).collect();
    let last = rest.last()?;
    if last.trim() != "```" {
        return None;
    }
    let lang_str = first_line[3..].trim();
    let lang = (!lang_str.is_empty()).then(|| lang_str.to_string());
    let body = rest[..rest.len() - 1].join("\n");
    Some((lang, body))
}
