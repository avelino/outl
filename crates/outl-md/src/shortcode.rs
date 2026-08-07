//! The `:shortcode:` matcher — the one inline construct whose payload
//! is validated against an external catalog instead of by shape alone.
//!
//! The catalog itself (`crate::emoji`) owns "which shortcodes exist";
//! this module owns "when does a `:`…`:` run in prose count as one".
//! Both gates are load-bearing, and the rationale for each lives on
//! [`try_emoji`] — read it before loosening either.

use crate::token::InlineTok;

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
pub(crate) fn try_emoji(s: &str) -> Option<(InlineTok<'_>, usize)> {
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
