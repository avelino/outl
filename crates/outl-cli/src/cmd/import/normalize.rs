//! Normalize free-form Logseq/Roam markdown into the strict outl
//! outliner dialect.
//!
//! outl's parser is strict: it only recognises `- ` items as blocks,
//! a block's continuation text ends at the first blank line, an
//! indented line with no parent block at the level above is dropped,
//! and the page must open with bullets (or page properties). Logseq
//! exports break all of these:
//!
//! - top-level `## headings` and loose paragraphs (no bullet),
//! - block text spread across several paragraphs with blank lines,
//! - a first block indented under nothing (`\t- x` after a page prop),
//! - tab-based indentation that skips levels.
//!
//! Left untouched, every one of those silently loses content. This
//! module rebuilds the outline into a tree and re-emits it with
//! canonical two-space indentation so the parser preserves everything:
//!
//! - non-bullet lines become bullets,
//! - continuation paragraphs are merged into the block's text (blank
//!   lines between them dropped, so the parser keeps them),
//! - block levels are clamped so no block skips a level or dangles,
//! - fenced code blocks are dedented and re-indented verbatim.

const INDENT_WIDTH: usize = 2;

/// One block plus the continuation text and properties that belong to
/// it, captured before re-indentation.
struct Block {
    /// Indentation level in the *source* (used only to rebuild the
    /// tree shape; the emitted level is clamped).
    src_level: usize,
    /// Block text. May contain `\n` for merged continuation lines or a
    /// fenced code block.
    text: String,
    /// `key:: value` block properties.
    props: Vec<String>,
}

/// Indentation level of a line, matching outl's parser: a tab counts
/// as `INDENT_WIDTH` spaces, then the leading whitespace is divided by
/// `INDENT_WIDTH`.
fn level(line: &str) -> usize {
    let mut spaces = 0usize;
    for b in line.bytes() {
        match b {
            b' ' => spaces += 1,
            b'\t' => spaces += INDENT_WIDTH,
            _ => break,
        }
    }
    spaces / INDENT_WIDTH
}

/// Count of leading whitespace *characters* (not levels). Used to
/// dedent fenced code bodies (tab = `INDENT_WIDTH` columns).
fn col_indent(line: &str) -> usize {
    let mut cols = 0usize;
    for b in line.bytes() {
        match b {
            b' ' => cols += 1,
            b'\t' => cols += INDENT_WIDTH,
            _ => break,
        }
    }
    cols
}

/// Expand the line's leading whitespace to spaces (tab → `INDENT_WIDTH`
/// spaces) then drop `n` leading space columns, keeping the rest of the
/// indentation (the code's relative structure) and the content.
fn strip_cols(line: &str, n: usize) -> String {
    let content = line.trim_start_matches([' ', '\t']);
    let cols = col_indent(line);
    let kept = cols.saturating_sub(n);
    format!("{}{}", " ".repeat(kept), content)
}

fn is_bullet(stripped: &str) -> bool {
    stripped == "-" || stripped.starts_with("- ")
}

fn strip_bullet(stripped: &str) -> &str {
    if stripped == "-" {
        ""
    } else {
        stripped.strip_prefix("- ").unwrap_or(stripped).trim_start()
    }
}

fn is_property(stripped: &str) -> bool {
    // `key:: value` — a token, `::`, then anything. Mirrors the
    // outl parser's property recognition closely enough for routing.
    if let Some(idx) = stripped.find("::") {
        let key = &stripped[..idx];
        !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    } else {
        false
    }
}

fn is_fence(stripped: &str) -> bool {
    stripped.starts_with("```")
}

/// Normalize a converted Logseq page body (already split into lines).
pub fn normalize_outline(lines: Vec<String>) -> Vec<String> {
    let (page_props, blocks) = parse_blocks(&lines);
    emit(&page_props, &blocks)
}

/// Parse the raw lines into leading page properties plus a flat list of
/// blocks (each carrying its continuations and properties). Tree shape
/// is recovered later from `src_level`.
fn parse_blocks(lines: &[String]) -> (Vec<String>, Vec<Block>) {
    let mut page_props: Vec<String> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0usize;
    let mut seen_block = false;

    while i < lines.len() {
        let line = &lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        let lvl = level(line);
        let stripped = line.trim();

        // Leading page properties (only before any block, at level 0).
        if !seen_block && lvl == 0 && !is_bullet(stripped) && is_property(stripped) {
            page_props.push(stripped.to_string());
            i += 1;
            continue;
        }

        // Start a new block. Whether the line is a bullet or loose text,
        // it becomes one block; the bullet marker is stripped if present.
        let content = if is_bullet(stripped) {
            strip_bullet(stripped)
        } else {
            stripped
        };
        let mut block = Block {
            src_level: lvl,
            text: content.to_string(),
            props: Vec::new(),
        };
        i += 1;
        seen_block = true;

        if is_fence(content.trim_start()) {
            consume_fence(lines, &mut i, line, &mut block.text);
        } else {
            consume_continuations(lines, &mut i, lvl, &mut block);
        }
        blocks.push(block);
    }

    (page_props, blocks)
}

/// Absorb the lines that belong to `block`: indented non-bullet text
/// (continuations, merged across blank gaps) and `key:: value`
/// properties, up to the next bullet or a line outdented to/under the
/// block's own level.
fn consume_continuations(lines: &[String], i: &mut usize, src_level: usize, block: &mut Block) {
    while *i < lines.len() {
        let line = &lines[*i];
        if line.trim().is_empty() {
            // Skip blank lines: in Logseq they separate paragraphs
            // inside one block, but outl would end the block here.
            *i += 1;
            continue;
        }
        let lvl = level(line);
        let stripped = line.trim();
        if lvl <= src_level || is_bullet(stripped) {
            // Next sibling/child block, or an outdent: stop.
            break;
        }
        if is_property(stripped) {
            block.props.push(stripped.to_string());
        } else if is_fence(stripped) {
            // A fenced code block opened on a continuation line.
            block.text.push('\n');
            block.text.push_str(stripped);
            *i += 1;
            consume_fence_body(lines, i, &mut block.text);
            continue;
        } else {
            if !block.text.is_empty() {
                block.text.push('\n');
            }
            block.text.push_str(stripped);
        }
        *i += 1;
    }
}

/// Consume a fenced code block whose opener is on the bullet line
/// itself (`- ```lang`). The opener already sits in `text`; this picks
/// up at the body.
fn consume_fence(lines: &[String], i: &mut usize, _opener_line: &str, text: &mut String) {
    consume_fence_body(lines, i, text);
}

/// Collect a fenced code body up to its closing fence, dedent it by the
/// smallest indentation among its non-blank lines (drops the base
/// alignment, keeps the code's relative structure), and append the
/// dedented body plus a bare closing fence to `text`. Re-indentation to
/// the block's canonical level happens at emit time.
fn consume_fence_body(lines: &[String], i: &mut usize, text: &mut String) {
    let mut body: Vec<String> = Vec::new();
    while *i < lines.len() {
        let raw = &lines[*i];
        let stripped = raw.trim();
        *i += 1;
        if stripped.starts_with("```") {
            break;
        }
        body.push(raw.clone());
    }
    let min_col = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| col_indent(l))
        .min()
        .unwrap_or(0);
    for line in &body {
        text.push('\n');
        if !line.trim().is_empty() {
            text.push_str(&strip_cols(line, min_col));
        }
    }
    text.push('\n');
    text.push_str("```");
}

/// Re-emit page properties and blocks with canonical two-space
/// indentation, clamping levels so the tree never skips a level.
fn emit(page_props: &[String], blocks: &[Block]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in page_props {
        out.push(p.clone());
    }
    if !page_props.is_empty() {
        out.push(String::new());
    }

    // Stack of source levels of the open ancestors; its length is the
    // canonical depth of the next block.
    let mut stack: Vec<usize> = Vec::new();
    for block in blocks {
        while stack.last().is_some_and(|&l| l >= block.src_level) {
            stack.pop();
        }
        let canon = stack.len();
        stack.push(block.src_level);

        let indent = "  ".repeat(canon);
        let cont_indent = "  ".repeat(canon + 1);
        let mut text_lines = block.text.split('\n');
        let first = text_lines.next().unwrap_or("");
        out.push(format!("{indent}- {first}"));
        for line in text_lines {
            // Keep blank lines (e.g. inside a code fence) truly empty
            // rather than emitting trailing indentation.
            if line.is_empty() {
                out.push(String::new());
            } else {
                out.push(format!("{cont_indent}{line}"));
            }
        }
        for prop in &block.props {
            out.push(format!("{cont_indent}{prop}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(input: &str) -> String {
        normalize_outline(input.lines().map(str::to_string).collect()).join("\n")
    }

    #[test]
    fn heading_at_root_becomes_a_bullet() {
        assert_eq!(norm("## TODOs\n- DONE alpha"), "- ## TODOs\n- DONE alpha");
    }

    #[test]
    fn continuation_paragraphs_merge_dropping_blank_lines() {
        // The "Power" bug: multi-paragraph block body with blank lines.
        let input = "- Power\n  supino\n\n  arrumar isso\n\n  20kg x 10";
        assert_eq!(
            norm(input),
            "- Power\n  supino\n  arrumar isso\n  20kg x 10"
        );
    }

    #[test]
    fn orphan_indented_first_block_is_clamped_to_root() {
        // `\t- preciso` after a page property, with no level-0 parent.
        let input = "trained:: strength\n\t- preciso falar com alguem\n- outro";
        assert_eq!(
            norm(input),
            "trained:: strength\n\n- preciso falar com alguem\n- outro"
        );
    }

    #[test]
    fn nested_bullets_keep_relative_nesting_after_clamp() {
        // Source starts at level 1/2; output should be 0/1.
        let input = "\t- parent\n\t\t- child\n\t\t\t- grandchild";
        assert_eq!(norm(input), "- parent\n  - child\n    - grandchild");
    }

    #[test]
    fn block_properties_are_kept_under_their_block() {
        let input = "- task\n  collapsed:: true";
        assert_eq!(norm(input), "- task\n  collapsed:: true");
    }

    #[test]
    fn leading_page_property_is_preserved() {
        let input = "title:: Foo\n\n- body";
        assert_eq!(norm(input), "title:: Foo\n\n- body");
    }

    #[test]
    fn fenced_code_block_on_bullet_line_is_preserved() {
        let input = "- ```clojure\n  (def x\n    1)\n  ```";
        // Body keeps relative indent of `1)`; base indent dropped then
        // re-applied at the block's canonical level (0 here → 2 spaces).
        assert_eq!(norm(input), "- ```clojure\n  (def x\n    1)\n  ```");
    }

    #[test]
    fn fence_nested_under_a_bullet_reindents() {
        let input = "\t- code:\n\t\t- ```\n\t\t  line1\n\t\t  line2\n\t\t  ```";
        // parent at level 0, the fence bullet becomes its child at lvl 1.
        assert_eq!(
            norm(input),
            "- code:\n  - ```\n    line1\n    line2\n    ```"
        );
    }

    #[test]
    fn continuation_then_property_then_child_all_survive() {
        let input = "- root\n  more text\n  key:: val\n  - child";
        assert_eq!(norm(input), "- root\n  more text\n  key:: val\n  - child");
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(norm(""), "");
    }
}
