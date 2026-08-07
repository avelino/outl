//! Properties as they appear in a `.md`: the `key:: value` line, and
//! the contiguous run of them that opens a page.
//!
//! One reader for both positions — page properties (above the first
//! bullet) and block properties (nested under a bullet) — so the two
//! can never disagree on what a valid key is.
//!
//! Syntax only: no key is special here. Per-key semantics live with
//! their owner ([`crate::remind`] for `remind::`, `outl-actions` for
//! `collapsed::`). [`parse_property_line`] is re-exported by
//! [`crate::parse`] so `outl_md::parse::parse_property_line` keeps
//! resolving.

use crate::parse::leading_indent;

/// Read the page-property header: contiguous `key:: value` lines at
/// indent 0, ending at the first blank line **or** the first line that
/// isn't a property (typically `- block`).
///
/// `*i` is left on the first line the outline parser should look at —
/// past the terminating blank line, or on the non-property line that
/// stopped the run.
pub(crate) fn read_page_header(lines: &[&str], i: &mut usize) -> Vec<(String, String)> {
    let mut props: Vec<(String, String)> = Vec::new();
    while *i < lines.len() {
        let line = lines[*i];
        if line.trim().is_empty() {
            // Blank line ends the page-property header.
            *i += 1;
            break;
        }
        if leading_indent(line) > 0 {
            // Indented line cannot be a page property.
            break;
        }
        if let Some(kv) = parse_property_line(line.trim()) {
            props.push(kv);
            *i += 1;
        } else {
            break;
        }
    }
    props
}

/// Try to parse a single line as `key:: value` (or `key::` for empty value).
///
/// Returns `Some((key, value))` if it matches. The key may not contain
/// spaces; the value is everything after `:: `.
pub fn parse_property_line(line: &str) -> Option<(String, String)> {
    if let Some(pos) = line.find(":: ") {
        let key = line[..pos].trim();
        let value = line[pos + 3..].trim_end();
        if is_valid_key(key) {
            return Some((key.to_string(), value.to_string()));
        }
    }
    // `key::` with no value (and no trailing space).
    if let Some(rest) = line.strip_suffix("::") {
        let key = rest.trim_end();
        if is_valid_key(key) {
            return Some((key.to_string(), String::new()));
        }
    }
    None
}

fn is_valid_key(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_line_parser_handles_edge_cases() {
        assert_eq!(
            parse_property_line("priority:: high"),
            Some(("priority".into(), "high".into()))
        );
        assert_eq!(
            parse_property_line("done::"),
            Some(("done".into(), "".into()))
        );
        // Spaces in key invalidate.
        assert_eq!(parse_property_line("some key:: value"), None);
        // Missing `::`.
        assert_eq!(parse_property_line("just text"), None);
        // Empty key.
        assert_eq!(parse_property_line(":: value"), None);
    }
}
