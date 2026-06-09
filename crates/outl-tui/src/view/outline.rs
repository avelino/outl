//! Outline rendering — turn a `ParsedPage` (current view) into a
//! flat `Vec<Line>` for ratatui, with selection / cursor / TODO
//! decoration.

use crate::outline_ops::path_for_index;
use crate::state::{App, Focus, Mode};
use crate::theme::Theme;
use crate::view::inline::{highlight_inline, render_markdown_inline, render_pretty_block_text};
use outl_md::inline::{byte_index_for_char, tokenize, InlineTok};
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::view::{block_to_rows, BlockRowKind};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Maximum AST nesting depth we'll render inside a single embed
/// expansion. Caps the size of the visual block we draw under one
/// `!((blk-XXXXXX))` — a deeply nested source subtree gets truncated
/// instead of flooding the outline.
///
/// **Not a cycle protector.** Embed-of-embed (a source block whose
/// own text is another `!((blk-Y))`) is rendered inline with the `↳ `
/// marker by `render_pretty_block_text`; it is *not* recursively
/// expanded here. So an `A → B → A` cycle never enters this recursion
/// and the cap doesn't need to defend against it. If recursive embed
/// expansion ever lands, add a `visited: &HashSet<&str>` argument and
/// short-circuit when the current handle is already in the set.
const EMBED_MAX_DEPTH: u32 = 4;

/// Render the outline into a flat list of `Line`s for ratatui, and
/// report the visual line index where the *selected* block's bullet
/// row landed. The caller uses that index to keep the selection
/// inside the scrolled viewport.
pub(crate) fn render_outline(p: &ParsedPage, app: &App) -> (Vec<Line<'static>>, Option<usize>) {
    let mut out = Vec::new();
    for (k, v) in &p.properties {
        out.push(Line::from(vec![
            Span::styled(format!("{k}:: "), app.theme.property_key),
            Span::styled(v.clone(), app.theme.property_value),
        ]));
    }
    if !p.properties.is_empty() && !p.blocks.is_empty() {
        out.push(Line::from(""));
    }
    let mut cursor = 0usize;
    let mut selected_line: Option<usize> = None;
    for block in &p.blocks {
        render_block(block, 0, &mut cursor, app, &mut out, &mut selected_line);
    }
    (out, selected_line)
}

pub(crate) fn render_block(
    b: &OutlineNode,
    indent: u32,
    cursor: &mut usize,
    app: &App,
    out: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
) {
    // Outline only owns selection/cursor decoration when focus lives
    // here. With `Focus::Backlink`, the bullet/caret belong to the
    // backlinks section — drawing them on the outline too would leave
    // a "ghost cursor" on the last outline block (the value `selected`
    // still happens to point at).
    let focused_on_outline = matches!(app.focus, Focus::Outline);
    let is_selected = focused_on_outline && *cursor == app.selected;
    // Record the visual line where this block's bullet row begins so
    // the caller can scroll the viewport to keep it visible.
    if is_selected && selected_line.is_none() {
        *selected_line = Some(out.len());
    }
    let in_visual_range = focused_on_outline
        && app
            .visual_range()
            .is_some_and(|(lo, hi)| *cursor >= lo && *cursor <= hi);
    let editing_here = focused_on_outline
        && matches!(&app.mode, Mode::Insert { block_path, .. }
            if path_for_index(&app.page.blocks, *cursor).as_deref() == Some(block_path.as_slice()));

    let bullet_style = if is_selected || in_visual_range {
        app.theme.selected_bullet
    } else {
        app.theme.bullet
    };

    // Determine which text and cursor position to render. Three cases:
    //   1. Editing here       → buffer with caret cursor.
    //   2. Selected in Normal → block text with block-style cursor.
    //   3. Anything else      → block text, no cursor, pretty render.
    let mode = if editing_here {
        if let Mode::Insert { buffer, .. } = &app.mode {
            RenderMode::Editing {
                text: buffer.as_string(),
                cursor_char: buffer.cursor,
            }
        } else {
            unreachable!("editing_here matched but mode isn't Insert")
        }
    } else if is_selected && matches!(app.mode, Mode::Normal) {
        RenderMode::NormalCursor {
            text: b.text.clone(),
            cursor_char: app.cursor_col,
        }
    } else {
        RenderMode::Pretty {
            text: b.text.clone(),
        }
    };

    // Fold indicator for the bullet row.
    //   - `▼ ` when the block has children and is expanded
    //   - `▶ ` when it has children and is collapsed
    //   - `  ` (two spaces) when it has no children — keeps column
    //     alignment with the other two cases so the bullet column
    //     never jitters across blocks on the same indent.
    let block_id = app.id_by_flat.get(*cursor).copied();
    let is_collapsed = block_id
        .map(|id| app.collapsed.contains(&id))
        .unwrap_or(false);
    let has_children = !b.children.is_empty();
    let fold_marker = match (has_children, is_collapsed) {
        (false, _) => FoldMarker::None,
        (true, false) => FoldMarker::Expanded,
        (true, true) => FoldMarker::Collapsed,
    };

    let has_auto_run = b.properties.iter().any(|(k, _)| k == "auto-run");
    emit_block_lines(
        indent,
        bullet_style,
        &mode,
        has_auto_run,
        fold_marker,
        app,
        out,
    );

    for (k, v) in &b.properties {
        let mut prop_spans: Vec<Span<'_>> = Vec::new();
        for _ in 0..indent {
            prop_spans.push(Span::styled("│ ", app.theme.dim));
        }
        prop_spans.push(Span::raw("  ".to_string()));
        prop_spans.push(Span::styled(format!("{k}:: "), app.theme.property_key));
        prop_spans.push(Span::styled(v.clone(), app.theme.property_value));
        out.push(Line::from(prop_spans));
    }

    // Expand `!((blk-XXXXXX))` embeds as a read-only subtree under
    // the carrying block. Triggered when:
    //   - the block's text resolves to a single Embed token
    //     (mixed prose keeps the inline `↳ <text>` render);
    //   - the handle resolves through the workspace index.
    // The expanded rows are visual-only — they don't move `cursor`
    // (the flat index used for navigation), so `j` / `k` cross them
    // in one step instead of paging through borrowed content. The
    // carrying block's own row keeps whatever render `mode` chose
    // (raw with cursor, raw with caret, or pretty) so column-to-byte
    // alignment is never broken by the expansion.
    if let Some(handle) = embed_only_handle(&b.text) {
        if let Some(entry) = app.index.resolve_block_ref(handle) {
            // `outer_indent` matches the carrying block's own indent so
            // the `│ ` guides line up with the outline's normal indent
            // pattern. Embed-internal nesting comes from `depth`.
            emit_embedded_children(&entry.children, indent, 1, app, out);
        }
    }

    *cursor += 1;
    if is_collapsed {
        // Children are hidden — but the flat cursor still has to
        // skip past them because `App.selected` and friends index
        // the full DFS preorder (collapsed or not). Without this
        // bump, selection bookkeeping for blocks *below* the
        // collapsed subtree would shift up by `flat_count(children)`.
        *cursor += outl_md::outline_ops::flat_count(&b.children);
    } else {
        for child in &b.children {
            render_block(child, indent + 1, cursor, app, out, selected_line);
        }
    }
}

/// Return the handle if `text` is a single `!((blk-XXXXXX))` token
/// surrounded only by whitespace; `None` otherwise.
///
/// Mixed content (`prelude !((blk-X)) postlude`) keeps the inline
/// `↳ <text>` render — we only expand when the user clearly meant
/// the whole block to *be* the embed.
fn embed_only_handle(text: &str) -> Option<&str> {
    let mut handle: Option<&str> = None;
    for tok in tokenize(text.trim()) {
        match tok {
            InlineTok::Plain(s) if s.trim().is_empty() => continue,
            InlineTok::Embed { handle: h } if handle.is_none() => handle = Some(h),
            _ => return None,
        }
    }
    handle
}

/// Emit a source block's subtree underneath the embedding block.
///
/// Each row gets the same `↳ ` prefix the embed's first row carries
/// so the whole expansion reads as one cohesive block visually. Two
/// indent layers are stacked per row:
///
/// 1. `│ ` per ancestor indent of the carrying block (matches the
///    outline's own indent guides so the embed sits under the right
///    parent at a glance);
/// 2. two spaces per embed-subtree depth, so a child of the embed's
///    root lands visually under the root's `↳ ` instead of next to
///    it (otherwise the reader can't tell whether the row is a
///    sibling of the carrying block or a child of the source).
///
/// Depth-capped at [`EMBED_MAX_DEPTH`] so an embed cycle can't run
/// forever.
fn emit_embedded_children(
    children: &[OutlineNode],
    outer_indent: u32,
    depth: u32,
    app: &App,
    out: &mut Vec<Line<'static>>,
) {
    if depth > EMBED_MAX_DEPTH {
        return;
    }
    for child in children {
        let mut spans: Vec<Span<'static>> = Vec::new();
        // 1. Outline indent guides (mirrors what `emit_block_lines`
        //    draws for a regular block at the same depth in the doc).
        for _ in 0..outer_indent {
            spans.push(Span::styled("│ ", app.theme.dim));
        }
        // 2. Embed-internal indent so children land **below the source
        //    root's text**, not alongside its `↳ `. The carrying
        //    block's first row reads `- ↳ <root-text>`: bullet + space
        //    + `↳` + space = four cells before the root text starts.
        //    A child needs to clear those four cells plus one more
        //    embed-indent step (two cells) before its own `↳ `, then
        //    another two per nested level. `(depth + 1) * 2` spaces
        //    keeps the geometry: depth 1 → 4 spaces, depth 2 → 6, etc.
        for _ in 0..(depth + 1) {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled("↳ ", app.theme.dim));
        spans.extend(render_pretty_block_text(
            &child.text,
            &app.theme,
            &app.index,
        ));
        out.push(Line::from(spans));
        emit_embedded_children(&child.children, outer_indent, depth + 1, app, out);
    }
}

/// Fold indicator drawn before the bullet on the bullet row.
///
/// `None` keeps a two-cell gap so leaf rows align with their parent
/// at the same indent — without it, a leaf's `-` would slide left
/// the moment a sibling grew children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FoldMarker {
    /// Block has no children — no marker, gap only.
    None,
    /// Block has children and they're visible. `▼ ` prefix.
    Expanded,
    /// Block has children but they're folded away. `▶ ` prefix.
    Collapsed,
}

/// Where the cursor sits on a block being rendered, and what style
/// the renderer should use for it. The UI-agnostic decomposition
/// lives in [`outl_md::view`]; this enum carries the *TUI-flavored*
/// detail of "caret vs block cursor".
pub(crate) enum RenderMode {
    /// Insert mode — show the live buffer with a thin caret at
    /// `cursor_char`. Markdown is rendered raw so columns match bytes.
    Editing { text: String, cursor_char: usize },
    /// Normal mode on the selected block — show a vim-style block
    /// cursor on the character under `cursor_char`. Raw render.
    NormalCursor { text: String, cursor_char: usize },
    /// Anything else — markdown is rendered prettily; no cursor.
    Pretty { text: String },
}

/// Emit one or more ratatui [`Line`]s for a block's text.
///
/// Decomposition into visual rows (bullet vs continuation vs code
/// fence marker vs code fence body) is delegated to
/// [`outl_md::view::block_to_rows`] so the Tauri GUI and mobile
/// clients use the same classification. This function is the
/// TUI-specific mapping: each [`outl_md::view::BlockRow`] becomes a
/// `Line` of `Span`s using the active theme.
pub(crate) fn emit_block_lines(
    indent: u32,
    bullet_style: Style,
    mode: &RenderMode,
    has_auto_run: bool,
    fold: FoldMarker,
    app: &App,
    out: &mut Vec<Line<'static>>,
) {
    let (text, cursor_char, cursor_style) = match mode {
        RenderMode::Editing { text, cursor_char } => {
            (text.as_str(), Some(*cursor_char), Some(CursorStyle::Caret))
        }
        RenderMode::NormalCursor { text, cursor_char } => {
            (text.as_str(), Some(*cursor_char), Some(CursorStyle::Block))
        }
        RenderMode::Pretty { text } => (text.as_str(), None, None),
    };
    let pretty = matches!(mode, RenderMode::Pretty { .. });
    let rows = block_to_rows(text, indent, cursor_char);

    // TODO/DONE checkbox decoration only fits on single-line bullets
    // (multi-line ones would have the icon floating above body text).
    let single_line_pretty = pretty && rows.len() == 1;

    for row in &rows {
        let mut spans: Vec<Span<'_>> = Vec::new();
        for _ in 0..row.indent {
            spans.push(Span::styled("│ ", app.theme.dim));
        }
        match row.kind {
            BlockRowKind::Bullet => {
                // Fold indicator goes first — two-cell slot whether
                // the marker is visible or not. Keeps the bullet `-`
                // column stable across siblings (leaf next to a
                // parent must line up).
                match fold {
                    FoldMarker::None => spans.push(Span::raw("  ")),
                    FoldMarker::Expanded => spans.push(Span::styled("▼ ", app.theme.dim)),
                    FoldMarker::Collapsed => spans.push(Span::styled("▶ ", app.theme.hint)),
                }
                // Blocks with `auto-run::` get a ⚡ before the bullet
                // so the user can see at a glance which cells re-run
                // themselves on page open.
                if has_auto_run {
                    spans.push(Span::styled("⚡", app.theme.hint));
                }
                spans.push(Span::styled("- ", bullet_style));
            }
            BlockRowKind::Continuation
            | BlockRowKind::CodeFenceMarker
            | BlockRowKind::CodeFenceBody => {
                // Mirror the bullet-row's pre-bullet padding so
                // continuation rows stay aligned with the bullet
                // column above them (two cells for the fold slot,
                // one extra cell when `⚡` is present).
                spans.push(Span::raw("  "));
                if has_auto_run {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::raw("  "));
            }
        }

        // If the cursor is on this row we always go raw — we want
        // bytes to line up with what the user typed, regardless of
        // fence state.
        if let (Some(col), Some(style)) = (row.cursor_col, cursor_style) {
            emit_row_with_cursor(row.text, col, style, &app.theme, &mut spans);
        } else {
            // A bullet row whose text opens a code fence (`` ```lisp ``)
            // is *both* a bullet and a fence marker — style the text
            // dimly so the fence reads visually like the rest of the
            // code block while keeping the `- ` glyph emitted above.
            let bullet_is_fence_opener = matches!(row.kind, BlockRowKind::Bullet)
                && row.text.trim_start().starts_with("```");
            match row.kind {
                _ if pretty && bullet_is_fence_opener => {
                    spans.push(Span::styled(row.text.to_string(), app.theme.dim));
                }
                BlockRowKind::CodeFenceMarker if pretty => {
                    spans.push(Span::styled(row.text.to_string(), app.theme.dim));
                }
                BlockRowKind::CodeFenceBody if pretty => {
                    spans.push(Span::styled(row.text.to_string(), app.theme.code));
                }
                BlockRowKind::Bullet if single_line_pretty => {
                    // Single owner for the bullet's pretty render: it
                    // strips TODO/DONE + `"> "` markers in either
                    // order, paints the `│ ` quote bar and the
                    // `☐`/`☑` checkbox, then tokenises the body. Same
                    // function the embed expansion uses, so the
                    // chrome stays in lockstep between bullet and
                    // embed root.
                    spans.extend(render_pretty_block_text(row.text, &app.theme, &app.index));
                }
                _ => spans.extend(render_markdown_inline(row.text, &app.theme, &app.index)),
            }
        }
        out.push(Line::from(spans));
    }
}

/// Draw one row with the cursor highlighted at `col` (a char index
/// into `text`). Splits the row in three: left of cursor, the char
/// under the cursor (or a thin caret if past-end), right of cursor.
fn emit_row_with_cursor(
    text: &str,
    col: usize,
    style: CursorStyle,
    theme: &Theme,
    spans: &mut Vec<Span<'static>>,
) {
    let byte = byte_index_for_char(text, col);
    let (left, right) = text.split_at(byte);
    spans.extend(highlight_inline(left, theme));
    let mut right_chars = right.chars();
    match (right_chars.next(), style) {
        (Some(ch), CursorStyle::Caret) => {
            // Thin caret BEFORE the next char.
            spans.push(Span::styled("▏", theme.cursor_caret));
            spans.push(Span::raw(ch.to_string()));
            let rest: String = right_chars.collect();
            spans.extend(highlight_inline(&rest, theme));
        }
        (Some(ch), CursorStyle::Block) => {
            // Inverted-color block cursor on the char under it.
            spans.push(Span::styled(ch.to_string(), theme.cursor_block));
            let rest: String = right_chars.collect();
            spans.extend(highlight_inline(&rest, theme));
        }
        (None, CursorStyle::Caret) => {
            spans.push(Span::styled("▏", theme.cursor_caret));
        }
        (None, CursorStyle::Block) => {
            spans.push(Span::styled("▏", theme.cursor_block));
        }
    }
}

/// Cursor visual style. `Caret` is the thin `▏` (Insert mode);
/// `Block` is the inverted single-char box (Normal mode on the
/// selected block).
#[derive(Debug, Clone, Copy)]
enum CursorStyle {
    Caret,
    Block,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_only_handle_detects_bare_token() {
        assert_eq!(embed_only_handle("!((blk-r6s4a1))"), Some("blk-r6s4a1"));
    }

    #[test]
    fn embed_only_handle_ignores_surrounding_whitespace() {
        assert_eq!(embed_only_handle("  !((blk-r6s4a1))  "), Some("blk-r6s4a1"));
    }

    #[test]
    fn embed_only_handle_rejects_mixed_text() {
        assert_eq!(embed_only_handle("see !((blk-r6s4a1)) context"), None);
    }

    #[test]
    fn embed_only_handle_rejects_inline_ref() {
        // `((blk-X))` (no leading `!`) is a ref, not an embed —
        // must not trigger expansion.
        assert_eq!(embed_only_handle("((blk-r6s4a1))"), None);
    }

    #[test]
    fn embed_only_handle_rejects_two_embeds_on_one_block() {
        // Two embeds in the same block is ambiguous (which one expands
        // first?) — phase 1 keeps the rule strict: exactly one token,
        // surrounded by whitespace.
        assert_eq!(embed_only_handle("!((blk-aaaaaa)) !((blk-bbbbbb))"), None);
    }
}
