//! End-to-end tests for the inline tokenizer / `ref_at_cursor`.
//! Moved out of `src/inline.rs` to keep that module under the
//! file-size-guard. Every test here exercises only the public API.

use outl_md::inline::{byte_index_for_char, ref_at_cursor, tokenize, InlineTok, RefTarget};

#[test]
fn plain_text_is_one_token() {
    let toks = tokenize("hello world");
    assert_eq!(toks, vec![InlineTok::Plain("hello world")]);
}

#[test]
fn page_ref_is_recognized() {
    let toks = tokenize("see [[Avelino]] for more");
    assert!(toks.contains(&InlineTok::PageRef { name: "Avelino" }));
}

#[test]
fn tag_is_recognized() {
    let toks = tokenize("hot #project work");
    assert!(toks.contains(&InlineTok::Tag { name: "project" }));
}

/// Convenience for assertions: a Bold/Italic/Strike whose inner is a
/// single plain-text token (the common case in these tests).
fn plain_inner(s: &str) -> Vec<InlineTok<'_>> {
    vec![InlineTok::Plain(s)]
}

#[test]
fn bold_strips_inner() {
    let toks = tokenize("a **brave** soul");
    assert!(toks.contains(&InlineTok::Bold {
        inner: plain_inner("brave"),
    }));
}

#[test]
fn bold_under_double_underscore() {
    let toks = tokenize("look __at__ this");
    assert!(toks.contains(&InlineTok::Bold {
        inner: plain_inner("at"),
    }));
    assert!(!toks
        .iter()
        .any(|t| matches!(t, InlineTok::Italic { inner, .. } if inner == &plain_inner("at"))));
}

#[test]
fn bold_under_alongside_bold_star() {
    let toks = tokenize("**abc** __123__");
    assert!(toks.contains(&InlineTok::Bold {
        inner: plain_inner("abc"),
    }));
    assert!(toks.contains(&InlineTok::Bold {
        inner: plain_inner("123"),
    }));
}

#[test]
fn italic_star_and_under() {
    assert!(tokenize("an *italic* word").contains(&InlineTok::Italic {
        inner: plain_inner("italic"),
        marker: '*'
    }));
    assert!(tokenize("an _italic_ word").contains(&InlineTok::Italic {
        inner: plain_inner("italic"),
        marker: '_'
    }));
}

#[test]
fn strike_and_code() {
    assert!(tokenize("old ~~news~~").contains(&InlineTok::Strike {
        inner: plain_inner("news"),
    }));
    assert!(tokenize("call `fn()`").contains(&InlineTok::Code { inner: "fn()" }));
}

#[test]
fn bold_recurses_inner_refs() {
    // The bug this whole change exists to fix: `**[[avelino]]**` used
    // to surface as a single flat plain string inside Bold, which
    // meant the mobile renderer drew it as `[[avelino]]` text — the
    // ref styling was lost. With recursive inner tokenization the ref
    // emerges as its own token nested under Bold.
    let toks = tokenize("hi **[[avelino]]** there");
    let bold = toks
        .iter()
        .find_map(|t| match t {
            InlineTok::Bold { inner } => Some(inner.clone()),
            _ => None,
        })
        .expect("bold token");
    assert_eq!(bold, vec![InlineTok::PageRef { name: "avelino" }]);
}

#[test]
fn italic_recurses_inner_refs() {
    let toks = tokenize("hi *[[avelino]]* there");
    let italic_inner = toks
        .iter()
        .find_map(|t| match t {
            InlineTok::Italic { inner, .. } => Some(inner.clone()),
            _ => None,
        })
        .expect("italic token");
    assert_eq!(italic_inner, vec![InlineTok::PageRef { name: "avelino" }]);
}

#[test]
fn md_link_extracts_text_and_url() {
    let toks = tokenize("see [outl](https://outl.app) docs");
    assert!(toks.contains(&InlineTok::Link {
        text: "outl",
        url: "https://outl.app"
    }));
}

#[test]
fn unclosed_marker_falls_back_to_plain() {
    let toks = tokenize("a **brave");
    assert!(matches!(toks.first(), Some(InlineTok::Plain(_))));
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Bold { .. })));
}

#[test]
fn multibyte_text_does_not_panic() {
    let _ = tokenize("isso parece que está");
    let _ = tokenize("ação não pára aí");
    let _ = tokenize("ship it 🚀 today");
    let _ = tokenize("こんにちは world");
    let _ = tokenize("veja [[orçamento]] e #ação");
}

#[test]
fn block_ref_is_recognized() {
    let toks = tokenize("see ((blk-r6s4a1)) for context");
    assert!(toks.contains(&InlineTok::BlockRef {
        handle: "blk-r6s4a1"
    }));
}

#[test]
fn block_ref_with_seven_char_tail_is_recognized() {
    let toks = tokenize("ref ((blk-r6s4a1z)) end");
    assert!(toks.contains(&InlineTok::BlockRef {
        handle: "blk-r6s4a1z"
    }));
}

#[test]
fn double_paren_prose_does_not_tokenize_as_block_ref() {
    for bad in [
        "((really))",
        "((BLK-R6S4A1))",
        "((blk-))",
        "((blk_r6s4a1))",
        "((nothandle))",
        "(())",
    ] {
        let text = format!("see {bad} text");
        let toks = tokenize(&text);
        assert!(
            !toks.iter().any(|t| matches!(t, InlineTok::BlockRef { .. })),
            "{bad} should NOT tokenize as BlockRef; got {toks:?}"
        );
    }
}

#[test]
fn embed_is_recognized() {
    let toks = tokenize("expand !((blk-r6s4a1)) here");
    assert!(toks.contains(&InlineTok::Embed {
        handle: "blk-r6s4a1"
    }));
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Plain("!"))));
}

#[test]
fn bang_without_double_paren_does_not_tokenize_as_embed() {
    let toks = tokenize("watch out! really.");
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Embed { .. })));
}

#[test]
fn embed_with_invalid_handle_falls_through() {
    let toks = tokenize("look !((really)) here");
    assert!(!toks.iter().any(|t| matches!(t, InlineTok::Embed { .. })));
}

#[test]
fn ref_at_cursor_finds_page_ref() {
    let text = "see [[Avelino]] today";
    let idx = "see [[Av".chars().count();
    match ref_at_cursor(text, idx) {
        Some(RefTarget::Page(n)) => assert_eq!(n, "Avelino"),
        other => panic!("expected Page, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_finds_journal_date() {
    let text = "[[2026-05-24]]";
    match ref_at_cursor(text, 5) {
        Some(RefTarget::Journal(d)) => assert_eq!(d.to_string(), "2026-05-24"),
        other => panic!("expected Journal, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_finds_tag() {
    let text = "tag #foo here";
    let idx = "tag #f".chars().count();
    match ref_at_cursor(text, idx) {
        Some(RefTarget::Tag(t)) => assert_eq!(t, "foo"),
        other => panic!("expected Tag, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_outside_ref_is_none() {
    let text = "see [[Avelino]] later";
    let idx = "see [[Avelino]] la".chars().count();
    assert!(ref_at_cursor(text, idx).is_none());
}

#[test]
fn ref_at_cursor_finds_block_ref() {
    let text = "see ((blk-r6s4a1)) today";
    let idx = "see ((blk-r".chars().count();
    match ref_at_cursor(text, idx) {
        Some(RefTarget::Block(h)) => assert_eq!(h, "blk-r6s4a1"),
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_on_embed_resolves_to_block_target() {
    let text = "see !((blk-r6s4a1)) today";
    let on_bang = "see ".chars().count();
    match ref_at_cursor(text, on_bang) {
        Some(RefTarget::Block(h)) => assert_eq!(h, "blk-r6s4a1"),
        other => panic!("cursor on `!`: expected Block, got {other:?}"),
    }
    let inside = "see !((blk-r".chars().count();
    match ref_at_cursor(text, inside) {
        Some(RefTarget::Block(h)) => assert_eq!(h, "blk-r6s4a1"),
        other => panic!("cursor inside handle: expected Block, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_block_ref_ignores_invalid_handle() {
    let text = "see ((really)) today";
    let idx = "see ((re".chars().count();
    assert!(
        ref_at_cursor(text, idx).is_none(),
        "invalid handle inside ((..)) must not resolve to a RefTarget"
    );
}

#[test]
fn ref_at_cursor_handles_multibyte() {
    let text = "veja [[orçamento]] hoje";
    let idx = "veja [[orç".chars().count();
    match ref_at_cursor(text, idx) {
        Some(RefTarget::Page(n)) => assert_eq!(n, "orçamento"),
        other => panic!("expected Page, got {other:?}"),
    }
}

#[test]
fn byte_index_for_char_is_split_safe() {
    let s = "está";
    for c in 0..=s.chars().count() {
        let b = byte_index_for_char(s, c);
        let _ = s.split_at(b);
    }
    assert_eq!(byte_index_for_char(s, 0), 0);
    assert_eq!(byte_index_for_char(s, 4), 5);
}

// --- bugfix regressions (review-driven) -----------------------------

#[test]
fn ref_at_cursor_recovers_after_invalid_handle() {
    // `((((blk-r6s4a1))))` — the outer `((` captures `((blk-r6s4a1`
    // (invalid). The scanner must advance ONE byte and pick up the
    // inner valid handle at offset 2, not skip past the whole thing.
    let text = "((((blk-r6s4a1))))";
    let inside_valid = "((((blk-r".chars().count();
    match ref_at_cursor(text, inside_valid) {
        Some(RefTarget::Block(h)) => assert_eq!(h, "blk-r6s4a1"),
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn ref_at_cursor_finds_second_block_ref_after_invalid_first() {
    // Invalid handle first, then a valid one. The greedy scan used
    // to skip past the second handle when the first's `))` was
    // consumed defensively.
    let text = "look ((foo)) and then ((blk-r6s4a1)) yes";
    let inside = "look ((foo)) and then ((blk-r".chars().count();
    match ref_at_cursor(text, inside) {
        Some(RefTarget::Block(h)) => assert_eq!(h, "blk-r6s4a1"),
        other => panic!("expected Block, got {other:?}"),
    }
}
