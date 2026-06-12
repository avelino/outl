//! Emoji shortcode resolution and search.
//!
//! Source-of-truth rule (matches invariant #2 — markdown stays clean):
//! the `.md` file always stores `:shortcode:` literal; the renderer
//! translates to the unicode glyph at display time. Two consequences
//! every consumer in the repo must respect:
//!
//! 1. **Lookup is one-way.** We resolve `shortcode → glyph` for
//!    rendering; we do **not** retro-translate `glyph → shortcode` on
//!    paste — multiple shortcodes can map to the same codepoint
//!    (`:+1:` and `:thumbsup:` both → 👍) and the disk form would
//!    become lossy.
//! 2. **Catalog is GitHub Gemoji.** Backed by the [`emojis`] crate
//!    (Unicode CLDR + GitHub aliases). The parser only tokenizes
//!    `:foo:` when `foo` is a known shortcode; unknown input stays
//!    plain so prose like `meeting at 14:00 :` doesn't get butchered.
//!
//! The autocomplete in every client (TUI, mobile, desktop) calls
//! [`search`] through one shared Tauri command — the catalog and the
//! parser cannot drift on what `:foo:` means.

use serde::{Deserialize, Serialize};

/// Resolve a GitHub-style shortcode (without the surrounding `:`s) to
/// the unicode glyph it stands for.
///
/// Returns `None` for unknown shortcodes — the inline tokenizer relies
/// on this to skip `:notanemoji:` and leave it as plain text.
pub fn shortcode_to_unicode(shortcode: &str) -> Option<&'static str> {
    emojis::get_by_shortcode(shortcode).map(|e| e.as_str())
}

/// Is `s` a syntactically valid shortcode shape (`[a-z0-9_+-]+`)?
///
/// **Does not** check the catalog — that's [`shortcode_to_unicode`].
/// Used by the inline tokenizer to walk forward from a `:` candidate.
pub fn is_valid_shortcode(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_valid_shortcode_char)
}

/// One char of a shortcode — lowercase ASCII letters, digits, `_`,
/// `+`, `-`. Pinned by the GitHub gemoji syntax (`:+1:`, `:-1:`,
/// `:smile_cat:`, `:100:`). Not exported because the matcher in
/// `inline.rs` reaches it through [`is_valid_shortcode`].
pub(crate) fn is_valid_shortcode_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '+' || c == '-'
}

/// One match returned by [`search`]: the shortcode that matched, the
/// glyph it stands for, and a score (higher is better).
///
/// `Serialize + Deserialize` because the autocomplete Tauri command
/// ships these directly to mobile + desktop frontends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiHit {
    /// GitHub-style shortcode, e.g. `"rocket"` (without `:`s).
    pub shortcode: String,
    /// Unicode glyph the shortcode resolves to, e.g. `"🚀"`.
    pub glyph: String,
    /// Match score: higher = better. Stable enough for the autocomplete
    /// popup to sort on; not a public ranking guarantee.
    pub score: u32,
}

/// Search the gemoji catalog for shortcodes matching `query`.
///
/// Ranking (highest first):
/// - exact shortcode match
/// - shortcode starts with the query
/// - shortcode contains the query (substring)
///
/// Shorter shortcodes win ties (e.g. `:smile:` ranks above
/// `:smile_cat:` for query `"smi"`). `limit == 0` short-circuits to an
/// empty vec; empty / whitespace-only query also returns empty.
///
/// `query` is matched case-insensitively (lowered before scoring).
pub fn search(query: &str, limit: usize) -> Vec<EmojiHit> {
    if limit == 0 {
        return Vec::new();
    }
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let q_lower = q.to_ascii_lowercase();

    let mut hits: Vec<EmojiHit> = Vec::new();
    for emoji in emojis::iter() {
        let Some(shortcode) = emoji.shortcode() else {
            continue;
        };
        let Some(score) = score_match(shortcode, &q_lower) else {
            continue;
        };
        hits.push(EmojiHit {
            shortcode: shortcode.to_string(),
            glyph: emoji.as_str().to_string(),
            score,
        });
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.shortcode.len().cmp(&b.shortcode.len()))
            .then_with(|| a.shortcode.cmp(&b.shortcode))
    });
    hits.truncate(limit);
    hits
}

/// Score `shortcode` against a lowercased `query`. `None` means no match.
///
/// The bias toward shorter shortcodes is intentional: when the user
/// types `:smi`, `:smile:` should land above `:smile_cat:` /
/// `:smiling_face_with_tear:`. The length penalty is small enough that
/// it never beats the kind (exact > prefix > substring).
fn score_match(shortcode: &str, query: &str) -> Option<u32> {
    let len_penalty = (shortcode.len().min(64)) as u32;
    if shortcode == query {
        return Some(10_000);
    }
    if shortcode.starts_with(query) {
        return Some(5_000 - len_penalty);
    }
    if shortcode.contains(query) {
        return Some(2_000 - len_penalty);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shortcode_resolves_to_glyph() {
        assert_eq!(shortcode_to_unicode("tada"), Some("🎉"));
        assert_eq!(shortcode_to_unicode("rocket"), Some("🚀"));
        assert_eq!(shortcode_to_unicode("fire"), Some("🔥"));
    }

    #[test]
    fn unknown_shortcode_is_none() {
        assert!(shortcode_to_unicode("notarealemoji").is_none());
        assert!(shortcode_to_unicode("").is_none());
    }

    #[test]
    fn digit_only_shortcode_works() {
        // `:100:` is gemoji.
        assert_eq!(shortcode_to_unicode("100"), Some("💯"));
    }

    #[test]
    fn shortcode_with_underscore_works() {
        // `:smile_cat:` pins the `_` separator decision (cat with
        // grinning face and smiling eyes — not to be confused with
        // `:smiley_cat:` which is the open-mouthed 😺).
        assert_eq!(shortcode_to_unicode("smile_cat"), Some("😸"));
    }

    #[test]
    fn aliases_collapse_to_same_glyph() {
        // Both `thumbsup` and `+1` are in the gemoji catalog and
        // resolve to 👍. We don't canonicalize on disk — each shortcode
        // round-trips with its own literal — but they must both render.
        let a = shortcode_to_unicode("thumbsup");
        let b = shortcode_to_unicode("+1");
        assert!(a.is_some(), "thumbsup should resolve");
        assert!(b.is_some(), "+1 should resolve");
        assert_eq!(a, b, "both aliases should land on 👍");
    }

    #[test]
    fn is_valid_shortcode_accepts_gemoji_alphabet() {
        assert!(is_valid_shortcode("tada"));
        assert!(is_valid_shortcode("smile_cat"));
        assert!(is_valid_shortcode("+1"));
        assert!(is_valid_shortcode("-1"));
        assert!(is_valid_shortcode("100"));
    }

    #[test]
    fn is_valid_shortcode_rejects_invalid() {
        assert!(!is_valid_shortcode(""));
        assert!(!is_valid_shortcode("Tada")); // uppercase
        assert!(!is_valid_shortcode("foo bar")); // space
        assert!(!is_valid_shortcode("foo:bar")); // colon
        assert!(!is_valid_shortcode("foo.bar")); // dot
        assert!(!is_valid_shortcode("foo/bar")); // slash
    }

    #[test]
    fn search_empty_returns_empty() {
        assert!(search("", 10).is_empty());
        assert!(search("   ", 10).is_empty());
    }

    #[test]
    fn search_zero_limit_returns_empty() {
        assert!(search("smile", 0).is_empty());
    }

    #[test]
    fn search_exact_match_wins() {
        let hits = search("smile", 5);
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(
            hits[0].shortcode,
            "smile",
            "exact match should rank first; got {:?}",
            hits.iter().map(|h| &h.shortcode).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_prefix_beats_substring() {
        // `:rocket:` (prefix) should beat any shortcode that only
        // *contains* "rock" elsewhere.
        let hits = search("rock", 10);
        assert!(!hits.is_empty());
        // Top hit starts with the query.
        assert!(
            hits[0].shortcode.starts_with("rock"),
            "expected prefix winner, got {}",
            hits[0].shortcode
        );
    }

    #[test]
    fn search_case_insensitive() {
        let lower = search("rock", 5);
        let upper = search("ROCK", 5);
        assert_eq!(
            lower.iter().map(|h| &h.shortcode).collect::<Vec<_>>(),
            upper.iter().map(|h| &h.shortcode).collect::<Vec<_>>(),
            "query case should not change ranking"
        );
    }

    #[test]
    fn search_respects_limit() {
        let hits = search("a", 3);
        assert!(hits.len() <= 3);
    }
}
