//! Parse outl markdown (`.md`, no IDs) into an outline AST.
//!
//! Grammar (informal):
//!
//! ```text
//! page          = page_props blank? block_list
//! page_props    = (prop_line newline)*
//! prop_line     = key "::" SPACE value
//! block_list    = (block_item)*
//! block_item    = indent? "- " content newline
//!                 (prop_line | block_item)*    // children at indent+1
//! indent        = (SPACE SPACE)*               // two spaces per level
//! ```
//!
//! See `docs/markdown-format.md` for the user-facing spec.

use serde::{Deserialize, Serialize};

/// Indent width in spaces. Two-space convention.
const INDENT_WIDTH: usize = 2;

/// One node in the outline AST. Same shape regardless of depth.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineNode {
    /// Block content (markdown inline, no `- ` prefix, no property lines).
    pub text: String,
    /// Properties attached to this block.
    pub properties: Vec<(String, String)>,
    /// Children of this block (depth-first).
    pub children: Vec<OutlineNode>,
}

/// Parsed page: top-level properties plus the outline tree.
///
/// `warnings` accumulates non-fatal grammar deviations the parser
/// recovered from — e.g. a markdown heading (`# title`) where outl
/// expects a `- bullet`, or an indent that isn't a multiple of two.
/// The parser **never** drops content: every offending line is
/// preserved as a regular block with its raw text (see
/// [`ParseWarningKind`] for the catalog of recoveries).
/// Surfaces (`outl-tui`, `outl-mobile`, `outl-desktop`, `outl doctor`)
/// render the warning list so the user can choose to clean the file
/// up — outl keeps working in the meantime.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPage {
    /// Page-level properties (the lines above the first outline item).
    pub properties: Vec<(String, String)>,
    /// Root-level outline blocks.
    pub blocks: Vec<OutlineNode>,
    /// Lines the parser preserved verbatim because they didn't match
    /// the outl dialect. Empty on a clean file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ParseWarning>,
}

/// A non-fatal recovery the parser performed while reading a `.md`.
///
/// Every warning carries the **1-based** source line number and the
/// raw line text, so a surface can highlight the exact offending row
/// without re-scanning the file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    /// 1-based line number in the source `.md`.
    pub line: usize,
    /// The offending line, verbatim (no trim).
    pub raw: String,
    /// Why the parser had to recover.
    pub kind: ParseWarningKind,
}

/// Catalog of recoveries the parser may perform.
///
/// Add a variant here when a new shape of "user wrote something the
/// dialect doesn't natively support" is detected. Keep the variant
/// name descriptive — UIs render it verbatim as a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseWarningKind {
    /// A line at the top level (or at a block's expected child slot)
    /// that doesn't start with `- ` and isn't a recognized property
    /// — typically a markdown heading (`# title`), a paragraph, an
    /// HTML snippet, or a table. The parser preserves it as a block
    /// with the raw text so a later edit + save doesn't drop content.
    UnrecognizedBlockMarker,
}

/// Parse a `.md` string into a [`ParsedPage`].
pub fn parse(md: &str) -> ParsedPage {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0usize;

    // Page-level properties: contiguous prop lines at the top, until the
    // first blank line OR the first non-property line (typically `- block`).
    let mut page_props: Vec<(String, String)> = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            // Blank line ends the page-property header.
            i += 1;
            break;
        }
        if leading_indent(line) > 0 {
            // Indented line cannot be a page property.
            break;
        }
        if let Some(kv) = parse_property_line(line.trim()) {
            page_props.push(kv);
            i += 1;
        } else {
            break;
        }
    }

    let mut cursor = i;
    let mut warnings: Vec<ParseWarning> = Vec::new();
    let blocks = parse_block_list(&lines, &mut cursor, 0, &mut warnings);

    ParsedPage {
        properties: page_props,
        blocks,
        warnings,
    }
}

fn parse_block_list(
    lines: &[&str],
    i: &mut usize,
    indent: usize,
    warnings: &mut Vec<ParseWarning>,
) -> Vec<OutlineNode> {
    let mut blocks: Vec<OutlineNode> = Vec::new();

    while *i < lines.len() {
        let raw = lines[*i];
        let stripped = raw.trim();
        if stripped.is_empty() {
            *i += 1;
            continue;
        }
        let line_indent = leading_indent(raw);
        if line_indent < indent {
            // Outdent — back to the caller's scope.
            return blocks;
        }
        if line_indent > indent {
            // Should be handled by the recursive call that owns this depth.
            // Defensive: skip to avoid infinite loop on malformed input.
            *i += 1;
            continue;
        }
        if !is_block_marker(stripped) {
            // Non-outline line at our indent. At depth 0 we recover by
            // turning it into a verbatim block (and emit a warning) so
            // a hand-written `.md` with a leading `# title`, a stray
            // paragraph, or imported markdown doesn't silently lose
            // content on the next save. At deeper levels we bail back
            // to the caller (it knows the context) — the caller's loop
            // ultimately funnels every line through this path.
            if indent != 0 {
                return blocks;
            }
            warnings.push(ParseWarning {
                line: *i + 1,
                raw: raw.to_string(),
                kind: ParseWarningKind::UnrecognizedBlockMarker,
            });
            blocks.push(OutlineNode {
                text: stripped.to_string(),
                properties: Vec::new(),
                children: Vec::new(),
            });
            *i += 1;
            continue;
        }

        // Consume a block marker line.
        let content = strip_block_marker(stripped);
        *i += 1;

        let mut node = OutlineNode {
            text: content.to_string(),
            properties: Vec::new(),
            children: Vec::new(),
        };

        // Continuation lines are only valid before any property or
        // child. Once one of those appears, plain indented text becomes
        // "unrecognized" again (skipped) — keeps the grammar
        // unambiguous.
        let mut accepting_continuation = true;

        // If the block's initial content is itself a fence opener (the
        // user wrote `- ```lisp` on one line), the opener already lives
        // in `node.text`. Pull the body and closing fence in *now*,
        // before we go looking at child/continuation lines — otherwise
        // we'd misread the closing `` ``` `` on a later line as a new
        // opener and swallow everything down to EOF.
        if node.text.trim_start().starts_with("```") {
            consume_fence_until_close(lines, i, indent + 1, &mut node.text);
            // Once a code fence has closed, the block is done — any
            // further indented text is no longer "continuation of a
            // single bullet" but a fresh thing the grammar can't see
            // safely. Properties and children still work via the loop
            // below if they appear.
            accepting_continuation = false;
        }

        // Read this block's continuation, properties and children at
        // indent + 1.
        loop {
            if *i >= lines.len() {
                break;
            }
            let next_raw = lines[*i];
            if next_raw.trim().is_empty() {
                // Blank line terminates continuation but not children
                // (children can have blank gaps between them in the
                // user's source).
                accepting_continuation = false;
                *i += 1;
                continue;
            }
            let next_indent = leading_indent(next_raw);
            if next_indent <= indent {
                break;
            }
            if next_indent == indent + 1 {
                let next_stripped = next_raw.trim();
                if is_block_marker(next_stripped) {
                    // Child block — recurse for the full sub-list.
                    accepting_continuation = false;
                    let children = parse_block_list(lines, i, indent + 1, warnings);
                    node.children.extend(children);
                } else if accepting_continuation && next_stripped.starts_with("```") {
                    // Fenced code block — consume literally until the
                    // matching closing fence at the same indent.
                    consume_fence(lines, i, indent + 1, &mut node.text);
                } else if let Some(kv) = parse_property_line(next_stripped) {
                    accepting_continuation = false;
                    node.properties.push(kv);
                    *i += 1;
                } else if accepting_continuation {
                    // Continuation of the block's text — append with a
                    // newline separator. Preserves the user's wrap
                    // intent without baking the indent into the text.
                    if !node.text.is_empty() {
                        node.text.push('\n');
                    }
                    node.text.push_str(next_stripped);
                    *i += 1;
                } else {
                    // Unrecognized line — skip to avoid hang.
                    *i += 1;
                }
            } else {
                // Over-indented; recurse so the deeper level can claim it.
                accepting_continuation = false;
                let extra = parse_block_list(lines, i, indent + 1, warnings);
                node.children.extend(extra);
            }
        }

        blocks.push(node);
    }

    blocks
}

/// Consume a fenced code block from `lines[*i]` (the opening fence)
/// up to and including the matching closing fence. The full literal
/// content — including the fences themselves — gets appended to
/// `target` so a later [`crate::render::render`] can emit it back
/// untouched.
///
/// The closing fence is recognized as a line whose trimmed content is
/// **exactly** ` ``` `, ignoring any info string on the opener. Tabs
/// inside the fence are preserved (we always read the raw line, not
/// the indent-stripped form).
///
/// `i` is advanced past the closing fence on success, or to the end
/// of the input if the closing fence is missing (graceful close).
fn consume_fence(lines: &[&str], i: &mut usize, fence_indent: usize, target: &mut String) {
    // The opening fence line itself.
    let opener = lines[*i];
    let opener_stripped = opener.trim();
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(opener_stripped);
    *i += 1;
    consume_fence_until_close(lines, i, fence_indent, target);
}

/// Same as [`consume_fence`], but the opener line is assumed to be
/// already in `target` (e.g. it was part of the block's first line:
/// `- ```lisp`). Picks up at the body, scans until the matching
/// closing fence at `fence_indent`, and advances `*i` past it.
fn consume_fence_until_close(
    lines: &[&str],
    i: &mut usize,
    fence_indent: usize,
    target: &mut String,
) {
    while *i < lines.len() {
        let raw = lines[*i];
        let stripped = raw.trim();
        // A closing fence is exactly three (or more) backticks alone.
        let is_closing = stripped == "```"
            || (stripped.starts_with("```") && stripped.chars().skip(3).all(|c| c == '`'));
        if is_closing && leading_indent(raw) == fence_indent {
            target.push('\n');
            target.push_str(stripped);
            *i += 1;
            return;
        }
        // A line outdented below the fence — e.g. a `- next block` at
        // indent 0 while the fence body lives at indent 1 — is *not*
        // part of this fence. Leave it for the outer parser to handle.
        // Without this guard a missing closer would swallow the rest
        // of the document.
        if leading_indent(raw) < fence_indent && !stripped.is_empty() {
            break;
        }
        // Inside the fence: append exactly what the user wrote,
        // minus the outer indent. Content indentation relative to
        // the fence is preserved.
        let preserved = strip_indent(raw, fence_indent);
        target.push('\n');
        target.push_str(preserved);
        *i += 1;
    }
    // Reached EOF (or an out-dented sibling) without a closing fence.
    // Leave a synthetic close so the rendered output stays well-formed
    // and the next parse round-trips.
    target.push('\n');
    target.push_str("```");
}

/// Drop `level * INDENT_WIDTH` leading spaces from a line if present.
/// Used inside fenced code blocks so we don't bake outline indent
/// into the user's literal content.
fn strip_indent(line: &str, level: usize) -> &str {
    let want = level * INDENT_WIDTH;
    let mut count = 0usize;
    let mut byte = 0usize;
    for b in line.bytes() {
        if count >= want {
            break;
        }
        if b == b' ' {
            count += 1;
            byte += 1;
        } else if b == b'\t' {
            count += INDENT_WIDTH;
            byte += 1;
        } else {
            break;
        }
    }
    &line[byte..]
}

fn leading_indent(line: &str) -> usize {
    let mut spaces = 0usize;
    for b in line.bytes() {
        if b == b' ' {
            spaces += 1;
        } else if b == b'\t' {
            // Treat one tab as one full indent level.
            spaces += INDENT_WIDTH;
        } else {
            break;
        }
    }
    spaces / INDENT_WIDTH
}

fn is_block_marker(stripped: &str) -> bool {
    stripped == "-" || stripped.starts_with("- ")
}

fn strip_block_marker(stripped: &str) -> &str {
    if stripped == "-" {
        return "";
    }
    stripped.strip_prefix("- ").unwrap_or(stripped).trim_start()
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
    fn page_properties_only() {
        let md = "title:: foo\nstatus:: active\n";
        let p = parse(md);
        assert_eq!(
            p.properties,
            vec![
                ("title".into(), "foo".into()),
                ("status".into(), "active".into()),
            ]
        );
        assert!(p.blocks.is_empty());
    }

    /// A `.md` that starts with a markdown heading (the seeded
    /// journal template was `# {{date}}\n\n- \n` before issue #55).
    /// The parser must NOT drop content — every line becomes a
    /// block — and the recovery is logged as a warning so a UI can
    /// surface it.
    #[test]
    fn permissive_recovers_top_level_heading() {
        let md = "# 2026-06-08\n\n- real bullet\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2, "heading + bullet, neither dropped");
        assert_eq!(p.blocks[0].text, "# 2026-06-08");
        assert_eq!(p.blocks[1].text, "real bullet");
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].line, 1);
        assert_eq!(p.warnings[0].raw, "# 2026-06-08");
        assert_eq!(
            p.warnings[0].kind,
            ParseWarningKind::UnrecognizedBlockMarker
        );
    }

    #[test]
    fn permissive_recovers_paragraph_at_top_level() {
        // A paragraph between bullets is preserved as a block too.
        // (At depth 0 — deeper levels still belong to their owning
        // bullet via the continuation / property machinery.)
        let md = "- first\nfree paragraph\n- second\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 3);
        assert_eq!(p.blocks[1].text, "free paragraph");
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].line, 2);
    }

    #[test]
    fn clean_file_has_no_warnings() {
        let md = "title:: foo\n\n- a\n  - b\n- c\n";
        let p = parse(md);
        assert!(p.warnings.is_empty(), "clean dialect emits zero warnings");
    }

    #[test]
    fn simple_outline() {
        let md = "- a\n- b\n- c\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 3);
        assert_eq!(p.blocks[0].text, "a");
        assert_eq!(p.blocks[2].text, "c");
    }

    #[test]
    fn nested_outline_two_levels() {
        let md = "- parent\n  - child1\n  - child2\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 1);
        assert_eq!(p.blocks[0].text, "parent");
        assert_eq!(p.blocks[0].children.len(), 2);
        assert_eq!(p.blocks[0].children[0].text, "child1");
        assert_eq!(p.blocks[0].children[1].text, "child2");
    }

    #[test]
    fn block_properties_then_children() {
        let md = "- objective\n  priority:: high\n  owner:: avelino\n  - subobjective\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 1);
        let b = &p.blocks[0];
        assert_eq!(b.text, "objective");
        assert_eq!(
            b.properties,
            vec![
                ("priority".into(), "high".into()),
                ("owner".into(), "avelino".into()),
            ]
        );
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].text, "subobjective");
    }

    #[test]
    fn page_props_then_blocks_with_blank() {
        let md = "title:: doc\n\n- one\n- two\n";
        let p = parse(md);
        assert_eq!(p.properties, vec![("title".into(), "doc".into())]);
        assert_eq!(p.blocks.len(), 2);
    }

    #[test]
    fn deep_nesting() {
        let md = "- a\n  - b\n    - c\n      - d\n";
        let p = parse(md);
        assert_eq!(p.blocks[0].text, "a");
        assert_eq!(p.blocks[0].children[0].text, "b");
        assert_eq!(p.blocks[0].children[0].children[0].text, "c");
        assert_eq!(p.blocks[0].children[0].children[0].children[0].text, "d");
    }

    #[test]
    fn empty_block_marker() {
        let md = "-\n- next\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.blocks[0].text, "");
        assert_eq!(p.blocks[1].text, "next");
    }

    #[test]
    fn empty_md_yields_empty_page() {
        let p = parse("");
        assert!(p.properties.is_empty());
        assert!(p.blocks.is_empty());
    }

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

    #[test]
    fn continuation_lines_join_into_block_text() {
        let md = "- first line\n  second line\n  third line\n- next block\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.blocks[0].text, "first line\nsecond line\nthird line");
        assert_eq!(p.blocks[1].text, "next block");
    }

    #[test]
    fn continuation_stops_at_child_block() {
        // `  - child` is a child, not continuation.
        let md = "- header\n  continuation line\n  - child block\n";
        let p = parse(md);
        assert_eq!(p.blocks[0].text, "header\ncontinuation line");
        assert_eq!(p.blocks[0].children.len(), 1);
        assert_eq!(p.blocks[0].children[0].text, "child block");
    }

    #[test]
    fn continuation_stops_at_property() {
        let md = "- header\n  continuation\n  priority:: high\n";
        let p = parse(md);
        assert_eq!(p.blocks[0].text, "header\ncontinuation");
        assert_eq!(
            p.blocks[0].properties,
            vec![("priority".to_string(), "high".to_string())]
        );
    }

    #[test]
    fn blank_line_terminates_continuation() {
        // After the blank line, `still text` is unrecognized (not
        // continuation, not a child block) and gets skipped.
        let md = "- header\n  continuation\n\n  still text\n- next\n";
        let p = parse(md);
        assert_eq!(p.blocks[0].text, "header\ncontinuation");
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.blocks[1].text, "next");
    }

    #[test]
    fn fence_preserves_literal_content() {
        let md = "- intro\n  ```lisp\n  (+ 1 2)\n  ```\n- next\n";
        let p = parse(md);
        let expected = "intro\n```lisp\n(+ 1 2)\n```";
        assert_eq!(p.blocks[0].text, expected);
        assert_eq!(p.blocks[1].text, "next");
    }

    #[test]
    fn fence_keeps_outline_markers_literal() {
        // The `- not a block` line inside the fence must NOT become a
        // child block — that's the whole reason for fence mode.
        let md = "- header\n  ```\n  - not a block\n  - me neither\n  ```\n- next\n";
        let p = parse(md);
        assert_eq!(
            p.blocks[0].text,
            "header\n```\n- not a block\n- me neither\n```"
        );
        assert!(p.blocks[0].children.is_empty());
    }

    #[test]
    fn unclosed_fence_synthesizes_close() {
        let md = "- header\n  ```\n  oh no\n";
        let p = parse(md);
        assert!(p.blocks[0].text.ends_with("```"));
    }

    #[test]
    fn fence_opened_on_bullet_line_does_not_swallow_next_block() {
        // Regression: when the bullet line itself opens the fence
        // (`- ```lisp`), the parser used to keep `consume_fence` blind
        // to whether the next bullet at a lower indent was actually a
        // new block. The closing `` ``` `` got mistaken for an opener,
        // the `- **abc** __123__` line got absorbed as fence body, and
        // a synthetic close was appended at EOF — three corruptions
        // for the price of one.
        let md = "- ```lisp\n  (+ 1 2)\n  ```\n- **abc** __123__\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2, "must keep both blocks");
        assert_eq!(p.blocks[0].text, "```lisp\n(+ 1 2)\n```");
        assert_eq!(p.blocks[1].text, "**abc** __123__");
    }

    #[test]
    fn fence_opened_on_bullet_line_with_unclosed_body_stops_at_sibling() {
        // Even with a missing closer, a sibling block at outer indent
        // ends the fence — better to synthesize a close than swallow
        // every following block.
        let md = "- ```lisp\n  oops no close\n- next block\n";
        let p = parse(md);
        assert_eq!(p.blocks.len(), 2);
        assert!(p.blocks[0].text.ends_with("```"));
        assert_eq!(p.blocks[1].text, "next block");
    }
}
