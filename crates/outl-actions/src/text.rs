//! Small text helpers shared across block operations.
//!
//! These are pure string utilities with no workspace or op-log
//! involvement — they live here so `paste` (caret-anchored insert) and
//! `block::split` (split a block at the caret) share one implementation
//! instead of each carrying its own `chars().take(caret)` slice.

/// Split `s` into `(head, tail)` at a **character** offset (not a byte
/// offset). `caret` past the end saturates: `head` is all of `s` and
/// `tail` is empty.
pub(crate) fn split_at_char(s: &str, caret: usize) -> (String, String) {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(caret).collect();
    let tail: String = chars.collect();
    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_in_the_middle() {
        assert_eq!(
            split_at_char("hello world", 5),
            ("hello".to_string(), " world".to_string())
        );
    }

    #[test]
    fn caret_past_end_saturates() {
        assert_eq!(split_at_char("abc", 99), ("abc".to_string(), String::new()));
    }

    #[test]
    fn respects_utf8_char_boundaries() {
        // 'é' is multi-byte; caret 4 is a char index, so "café" stays whole.
        assert_eq!(
            split_at_char("café au lait", 4),
            ("café".to_string(), " au lait".to_string())
        );
    }
}
