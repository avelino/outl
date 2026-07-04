//! Tag-boundary predicates over inline block text.
//!
//! `text.contains("#tag")` is the bug this module exists to delete:
//! the substring form matches `#tag-longer`, `#tagged`, and `#tag/sub`
//! as false positives. The inline tokenizer ([`crate::inline`])
//! already knows exactly where a `#tag` token starts and ends, so the
//! predicate here routes through it instead of re-deriving boundary
//! rules with a regex.
//!
//! Matching follows the tokenizer's behavior: tag names are compared
//! **case-sensitively** and `tag` is the bare name without the leading
//! `#`. Tags nested inside emphasis (`**#tag**`) still match because
//! bold / italic / strike inners are re-tokenized; a `#tag` inside a
//! `` `code` `` span is *not* a tag and does not match.

use crate::inline::{tokenize, InlineTok};

/// True when `text` mentions `#tag` as a whole tag token.
///
/// `tag` is the tag name without the leading `#` (e.g. `"project"`).
/// `#tag-longer` / `#tag/sub` do **not** match `"tag"` — the token
/// name must be equal, not a prefix.
pub fn text_contains_tag(text: &str, tag: &str) -> bool {
    toks_contain_tag(&tokenize(text), tag)
}

fn toks_contain_tag(toks: &[InlineTok<'_>], tag: &str) -> bool {
    toks.iter().any(|tok| match tok {
        InlineTok::Tag { name } => *name == tag,
        InlineTok::Bold { inner }
        | InlineTok::Italic { inner, .. }
        | InlineTok::Strike { inner } => toks_contain_tag(inner, tag),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tag_matches() {
        assert!(text_contains_tag("working on #outl today", "outl"));
    }

    #[test]
    fn longer_tag_is_not_a_prefix_match() {
        // The substring bug this predicate replaces: `#tag-longer`
        // must not satisfy a query for `tag`.
        assert!(!text_contains_tag("see #tag-longer here", "tag"));
        assert!(!text_contains_tag("see #tagged here", "tag"));
        assert!(!text_contains_tag("see #tag/sub here", "tag"));
    }

    #[test]
    fn nested_tag_still_matches_its_full_name() {
        assert!(text_contains_tag("see #tag/sub here", "tag/sub"));
    }

    #[test]
    fn tag_at_end_of_line_matches() {
        assert!(text_contains_tag("ship it #urgent", "urgent"));
    }

    #[test]
    fn tag_followed_by_punctuation_matches() {
        assert!(text_contains_tag("done (#urgent), moving on", "urgent"));
        assert!(text_contains_tag("#urgent: fix the build", "urgent"));
        assert!(text_contains_tag("really #urgent!", "urgent"));
    }

    #[test]
    fn matching_is_case_sensitive_like_the_tokenizer() {
        // The tokenizer preserves case verbatim; so does the predicate.
        assert!(!text_contains_tag("see #Urgent", "urgent"));
        assert!(text_contains_tag("see #Urgent", "Urgent"));
    }

    #[test]
    fn tag_inside_emphasis_matches() {
        assert!(text_contains_tag("**#urgent** fix", "urgent"));
        assert!(text_contains_tag("*#urgent* fix", "urgent"));
        assert!(text_contains_tag("~~#urgent~~ fix", "urgent"));
    }

    #[test]
    fn hash_inside_code_span_is_not_a_tag() {
        assert!(!text_contains_tag("run `#urgent` literally", "urgent"));
    }

    #[test]
    fn plain_word_without_hash_does_not_match() {
        assert!(!text_contains_tag("urgent but untagged", "urgent"));
    }
}
