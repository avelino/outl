//! `==highlight==` inline token — the on-disk form of Roam's
//! `^^highlight^^` after import. Pins tokenization, source round-trip,
//! and the "don't swallow a comparison operator" guard.

use outl_md::inline::{inline_to_source, tokenize, InlineTok};

#[test]
fn highlight_tokenizes() {
    let toks = tokenize("mark ==this== well");
    assert!(
        toks.iter()
            .any(|t| matches!(t, InlineTok::Highlight { .. })),
        "expected a Highlight token, got {toks:?}"
    );
}

#[test]
fn highlight_round_trips_to_source() {
    // render(parse(x)) preserves the `==…==` marker verbatim.
    let src = "a ==foo bar== b";
    assert_eq!(inline_to_source(&tokenize(src)), src);
}

#[test]
fn highlight_nests_a_ref() {
    // The inner span is re-tokenized, so `[[page]]` inside a highlight
    // stays a page ref instead of flattening to text.
    let toks = tokenize("==see [[buser]]==");
    let inner = toks
        .iter()
        .find_map(|t| match t {
            InlineTok::Highlight { inner } => Some(inner),
            _ => None,
        })
        .expect("one highlight");
    assert!(inner.iter().any(|t| matches!(t, InlineTok::PageRef { .. })));
}

#[test]
fn comparison_operator_is_not_a_highlight() {
    // `count == total == 0` — the spaces around `==` keep it out of the
    // highlight matcher (unlike `~~strike~~`, which has no such rule).
    let toks = tokenize("count == total == 0");
    assert!(
        !toks
            .iter()
            .any(|t| matches!(t, InlineTok::Highlight { .. })),
        "spaced `==` must stay plain, got {toks:?}"
    );
}

#[test]
fn empty_highlight_stays_plain() {
    let toks = tokenize("==== nope");
    assert!(!toks
        .iter()
        .any(|t| matches!(t, InlineTok::Highlight { .. })));
}

#[test]
fn newline_inside_is_not_a_highlight() {
    let toks = tokenize("==line one\nline two==");
    assert!(!toks
        .iter()
        .any(|t| matches!(t, InlineTok::Highlight { .. })));
}

#[test]
fn multibyte_inner_round_trips() {
    // The matcher works on byte offsets, so multi-byte inner content
    // must survive intact (issue #52 acceptance).
    for src in ["==café==", "==日本語==", "==🔥 urgent=="] {
        let toks = tokenize(src);
        assert!(
            toks.iter()
                .any(|t| matches!(t, InlineTok::Highlight { .. })),
            "{src} should tokenize as a highlight"
        );
        assert_eq!(inline_to_source(&toks), src, "{src} should round-trip");
    }
}
