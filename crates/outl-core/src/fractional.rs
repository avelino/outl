//! Fractional indexing for sibling order.
//!
//! Each sibling's position is a lexicographically sortable ASCII string
//! using the lowercase alphabet `a..=z`. Inserting between two positions
//! `left < right` always produces a position strictly greater than `left`
//! and strictly less than `right`, with finite bytes.
//!
//! Properties:
//!
//! - `between(None, None)` returns a midpoint of the full space.
//! - `between(Some(l), None)` returns a position > `l`.
//! - `between(None, Some(r))` returns a position < `r`.
//! - `between(Some(l), Some(r))` requires `l < r` and returns a position
//!   strictly between them.
//!
//! Two replicas that pick the "same gap" produce two different positions
//! (one per replica) that the HLC total order then resolves deterministically.

use serde::{Deserialize, Serialize};
use std::fmt;

const FIRST_BYTE: u8 = b'a';
const LAST_BYTE: u8 = b'z';
const BELOW_FIRST: u8 = FIRST_BYTE - 1; // 0x60 — sentinel: "before any valid byte"
const ABOVE_LAST: u8 = LAST_BYTE + 1; // 0x7b  — sentinel: "after any valid byte"

/// A lexicographically sortable position string.
///
/// Values are non-empty strings of ASCII lowercase letters (`a..=z`).
/// All constructors guarantee that invariant; consumers may rely on it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Fractional(String);

/// Errors when constructing a position from a raw string.
#[derive(Debug, thiserror::Error)]
pub enum FractionalError {
    /// Empty input.
    #[error("fractional position must be non-empty")]
    Empty,
    /// Input contains a byte outside the `a..=z` alphabet.
    #[error("fractional position byte {0:#04x} is outside a..=z")]
    InvalidByte(u8),
}

impl Fractional {
    /// Construct a `Fractional` from a raw string, validating the alphabet.
    pub fn parse(s: impl Into<String>) -> Result<Self, FractionalError> {
        let s = s.into();
        if s.is_empty() {
            return Err(FractionalError::Empty);
        }
        for b in s.bytes() {
            if !(FIRST_BYTE..=LAST_BYTE).contains(&b) {
                return Err(FractionalError::InvalidByte(b));
            }
        }
        Ok(Self(s))
    }

    /// Returns the canonical "first" position.
    pub fn first() -> Self {
        Self(String::from("a"))
    }

    /// Returns the canonical "last" position.
    pub fn last() -> Self {
        Self(String::from("z"))
    }

    /// Returns a position strictly between `left` and `right`.
    ///
    /// # Panics
    ///
    /// Panics if both bounds are `Some` and `left >= right`.
    pub fn between(left: Option<&Self>, right: Option<&Self>) -> Self {
        if let (Some(l), Some(r)) = (left, right) {
            assert!(
                l < r,
                "fractional bounds must satisfy left < right ({l:?} >= {r:?})"
            );
        }

        let left_bytes: &[u8] = left.map(|f| f.0.as_bytes()).unwrap_or(&[]);
        let right_bytes_owned: Vec<u8>;
        let right_bytes: &[u8] = match right {
            Some(r) => r.0.as_bytes(),
            None => {
                right_bytes_owned = Vec::new();
                &right_bytes_owned
            }
        };
        let right_is_infinite = right.is_none();

        let mut result = Vec::<u8>::new();
        loop {
            let i = result.len();
            let l_byte = if i < left_bytes.len() {
                left_bytes[i]
            } else {
                BELOW_FIRST
            };
            let r_byte = if i < right_bytes.len() {
                right_bytes[i]
            } else if right_is_infinite {
                ABOVE_LAST
            } else {
                // right exhausted (had finite length). Anything past its
                // length is implicitly "less than the right bound" only
                // if we already used all of right's bytes — so once
                // exhausted, we need to be > whatever's coming on the
                // left side. Setting the upper limit to ABOVE_LAST keeps
                // the bisection going.
                ABOVE_LAST
            };

            debug_assert!(r_byte >= l_byte, "left bound exceeded right bound");

            if r_byte > l_byte + 1 {
                let mid = ((l_byte as u16) + (r_byte as u16)) / 2;
                result.push(mid as u8);
                break;
            }

            // r_byte == l_byte (still in common prefix) OR
            // r_byte == l_byte + 1 (boundary tight; descend with no upper bound)
            let pushed = if l_byte == BELOW_FIRST {
                // No left constraint yet; emit a valid byte and continue.
                FIRST_BYTE
            } else {
                l_byte
            };
            result.push(pushed);

            // Safety: bound the loop so a malformed pair can't run forever.
            // In practice this caps positions at 1024 bytes; well beyond
            // anything an outline produces.
            if result.len() > 1024 {
                panic!(
                    "fractional::between: produced runaway position (>1024 bytes); \
                    bounds must be sane: left={left:?} right={right:?}"
                );
            }
        }

        Self(String::from_utf8(result).expect("ASCII a..=z by construction"))
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Fractional {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_none_none_is_middle() {
        let mid = Fractional::between(None, None);
        assert!(mid.0.bytes().all(|b| b.is_ascii_lowercase()));
    }

    #[test]
    fn between_left_and_right_is_strict() {
        let a = Fractional::parse("a").unwrap();
        let c = Fractional::parse("c").unwrap();
        let b = Fractional::between(Some(&a), Some(&c));
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn between_adjacent_chars_descends() {
        let a = Fractional::parse("a").unwrap();
        let b = Fractional::parse("b").unwrap();
        let mid = Fractional::between(Some(&a), Some(&b));
        assert!(a < mid);
        assert!(mid < b);
    }

    #[test]
    fn between_left_only_is_greater() {
        let m = Fractional::parse("m").unwrap();
        let g = Fractional::between(Some(&m), None);
        assert!(m < g);
    }

    #[test]
    fn between_right_only_is_less() {
        let m = Fractional::parse("m").unwrap();
        let l = Fractional::between(None, Some(&m));
        assert!(l < m);
    }

    #[test]
    #[should_panic(expected = "left < right")]
    fn between_invalid_bounds_panic() {
        let a = Fractional::parse("c").unwrap();
        let b = Fractional::parse("a").unwrap();
        let _ = Fractional::between(Some(&a), Some(&b));
    }

    #[test]
    fn between_deep_descent_terminates() {
        // Force a long common prefix and adjacent divergence.
        let l = Fractional::parse("aaabb").unwrap();
        let r = Fractional::parse("aaabc").unwrap();
        let m = Fractional::between(Some(&l), Some(&r));
        assert!(l < m);
        assert!(m < r);
        assert!(m.0.len() < 1024);
    }

    #[test]
    fn parse_rejects_invalid_bytes() {
        assert!(Fractional::parse("A").is_err());
        assert!(Fractional::parse("a1").is_err());
        assert!(Fractional::parse("").is_err());
    }

    #[test]
    fn many_inserts_in_same_gap_remain_distinct() {
        // 50 inserts between "a" and "z", picking the new midpoint each
        // time. Real outliner workloads do this constantly.
        let mut left = Fractional::parse("a").unwrap();
        let right = Fractional::parse("z").unwrap();
        let mut seen = std::collections::HashSet::new();
        seen.insert(left.0.clone());
        seen.insert(right.0.clone());
        for _ in 0..50 {
            let m = Fractional::between(Some(&left), Some(&right));
            assert!(left < m);
            assert!(m < right);
            assert!(seen.insert(m.0.clone()), "duplicate position generated");
            left = m;
        }
    }
}
