//! Extract `(language, body)` from a block's raw text.
//!
//! A code block in outl is a multi-line bullet whose text *is* the
//! fenced markdown — opening fence, body, closing fence — preserved
//! verbatim across parse/render cycles. The block text looks like a
//! standard CommonMark fence:
//!
//! - line 1: triple-backtick + language tag (e.g. `lisp`)
//! - line 2..N: the body
//! - last line: triple-backtick closer
//!
//! This module is the boundary between "block text as it lives in the
//! AST" and "what the runtime sees" (just the body, plus the language
//! tag). Pure functions, no I/O.

/// What [`extract_fence`] returns when the block is a code fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceParts {
    /// Info-string after the opening backticks, lower-cased. Examples:
    /// `"lisp"`, `"python"`, `"js"`. Empty when the user wrote a bare
    /// `` ``` `` opener without a language tag.
    pub language: String,
    /// Body of the fence, without leading/trailing fence lines. The
    /// final newline is stripped — runtimes get exactly what the user
    /// typed between the fences.
    pub body: String,
}

/// Parse the language tag and body out of a block whose first line
/// opens a fence. Returns `None` if the text doesn't start with
/// `` ``` `` — i.e. the block isn't a code block at all.
///
/// The opening fence's info-string is taken verbatim up to the first
/// whitespace, lower-cased. Lines after the closer (if any) are
/// silently dropped; we don't expect them in our own render path but
/// outline editors might produce odd things.
pub fn extract_fence(text: &str) -> Option<FenceParts> {
    let mut lines = text.split('\n');
    let first = lines.next()?.trim_start();
    let after_ticks = first.strip_prefix("```")?;

    // Info-string is the run of non-whitespace chars after ```.
    let language = after_ticks
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut body = String::new();
    let mut first_body_line = true;
    for line in lines {
        if line.trim_start().starts_with("```") {
            break; // closing fence
        }
        if !first_body_line {
            body.push('\n');
        }
        body.push_str(line);
        first_body_line = false;
    }

    Some(FenceParts { language, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_lisp_block() {
        let parts = extract_fence("```lisp\n(+ 1 2)\n```").unwrap();
        assert_eq!(parts.language, "lisp");
        assert_eq!(parts.body, "(+ 1 2)");
    }

    #[test]
    fn extracts_multi_line_body() {
        let parts = extract_fence("```python\nfor i in range(3):\n    print(i)\n```").unwrap();
        assert_eq!(parts.language, "python");
        assert_eq!(parts.body, "for i in range(3):\n    print(i)");
    }

    #[test]
    fn language_lowercased() {
        let parts = extract_fence("```LISP\n(+ 1 2)\n```").unwrap();
        assert_eq!(parts.language, "lisp");
    }

    #[test]
    fn empty_language_when_bare_fence() {
        let parts = extract_fence("```\nplain text\n```").unwrap();
        assert_eq!(parts.language, "");
        assert_eq!(parts.body, "plain text");
    }

    #[test]
    fn returns_none_for_non_fence_block() {
        assert!(extract_fence("just regular text").is_none());
        assert!(extract_fence("- bullet content").is_none());
    }

    #[test]
    fn handles_info_string_with_extra_attrs() {
        // CommonMark allows ```lang attrs — we only care about the lang.
        let parts = extract_fence("```python {.numberLines}\nprint(1)\n```").unwrap();
        assert_eq!(parts.language, "python");
    }

    #[test]
    fn missing_closer_is_tolerated() {
        // If the closing fence got lost somehow, body is whatever's left.
        let parts = extract_fence("```lisp\n(+ 1 2)\n").unwrap();
        assert_eq!(parts.body, "(+ 1 2)\n");
    }
}
