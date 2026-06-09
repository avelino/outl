//! Quote state, encoded as a prefix on a block's text.
//!
//! Mirrors the TODO/DONE convention (see [`crate::todo`]): the marker
//! lives **in the block's text** as a `"> "` prefix instead of a new
//! field on the AST. Consumers (TUI, mobile, desktop) decide how to
//! render it; the wire format stays the same and round-trips through
//! CommonMark `.md` as `> body` continuation lines.
//!
//! Rules:
//!
//! 1. A block is a quote when its text starts with `"> "` — one space
//!    after `>`, matching CommonMark.
//! 2. Quote state is **per block**: children of a quoted block are not
//!    implicitly quoted (same policy as TODO/DONE).
//! 3. Nested quotes (`">> "`) are out of scope for the first cut — the
//!    body is whatever follows the single `"> "` prefix, verbatim, so a
//!    user who types `"> > foo"` gets a quoted block whose body
//!    *starts* with `"> foo"` but no extra unwrapping happens.

/// Wire prefix for a quoted block. Two characters including the
/// trailing space; consumers can rely on `chars().count() == 2` for
/// cursor math.
pub const QUOTE_PREFIX: &str = "> ";

/// Does the block's raw text carry the quote marker?
pub fn is_quote(text: &str) -> bool {
    text.starts_with(QUOTE_PREFIX)
}

/// Split a block's raw text into `(quoted, body)`. The body never
/// includes the prefix or its trailing space when `quoted` is true.
/// When the prefix is absent, `quoted` is `false` and `body` is the
/// untouched input.
pub fn split_quote(raw: &str) -> (bool, &str) {
    match raw.strip_prefix(QUOTE_PREFIX) {
        Some(rest) => (true, rest),
        None => (false, raw),
    }
}

/// Toggle the quote prefix on a block's raw text. Adds `"> "` when
/// absent, removes it when present. Mirrors
/// [`crate::todo::cycle_todo`] but with a two-state cycle (the quote
/// marker is binary, unlike TODO ↔ DONE ↔ none).
pub fn toggle_quote(raw: &str) -> String {
    let (quoted, body) = split_quote(raw);
    if quoted {
        body.to_string()
    } else {
        format!("{QUOTE_PREFIX}{raw}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_recognises_the_marker() {
        assert_eq!(split_quote("> a quote"), (true, "a quote"));
        assert_eq!(split_quote("plain block"), (false, "plain block"));
    }

    #[test]
    fn marker_requires_trailing_space() {
        // ">foo" without the space stays plain (CommonMark requires
        // the space too).
        assert_eq!(split_quote(">foo"), (false, ">foo"));
        // A bare ">" is also not a quote (no body, no trailing space).
        assert_eq!(split_quote(">"), (false, ">"));
        assert!(!is_quote(">foo"));
        assert!(is_quote("> "));
    }

    #[test]
    fn toggle_walks_through_two_states() {
        let s0 = "ship the feature";
        let s1 = toggle_quote(s0);
        let s2 = toggle_quote(&s1);
        assert_eq!(s1, "> ship the feature");
        assert_eq!(s2, "ship the feature");
    }

    #[test]
    fn toggle_preserves_inner_text_verbatim_including_inline_tokens() {
        // Inline tokens inside a quote stay untouched — the wrapper
        // is transparent to the inline tokenizer.
        let s = toggle_quote("**bold** [[ref]] #tag");
        assert_eq!(s, "> **bold** [[ref]] #tag");

        let (quoted, body) = split_quote(&s);
        assert!(quoted);
        assert_eq!(body, "**bold** [[ref]] #tag");
    }

    #[test]
    fn empty_body_is_legal() {
        // "> " alone is a quoted block with an empty body — we treat
        // it as legal so the user can type the marker and then the
        // body without an intermediate "not a quote" state.
        assert_eq!(split_quote("> "), (true, ""));
        assert!(is_quote("> "));
    }
}
