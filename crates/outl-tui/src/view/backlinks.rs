//! Inline backlinks section — rendered *below* the outline, separated
//! by a full-width `─` rule. Each source page contributes a header
//! followed by its referencing block (with children) as a mini-outline.
//!
//! Selection of a backlink block (`Focus::Backlink { idx, sub_path }`)
//! highlights the bullet the same way the outline does — same theme
//! `selected_bullet`, same cursor styles. The TUI scrolls so the
//! selection stays visible (handled by the caller in `view::render_main`).
//!
//! The list is produced by [`crate::actions::App::backlinks_for_current`]
//! — the single source for backlinks across the TUI, shared with the
//! mobile client through `outl_actions::backlinks_for_page`.

use crate::state::{App, EditTarget, Focus, Mode};
use crate::view::outline::{emit_block_lines, RenderMode};
use outl_actions::{Backlink, OutlineNode};
use ratatui::text::{Line, Span};

/// Render the inline backlinks section.
///
/// Returns `(lines, selected_line)`. `selected_line` is the index in
/// the returned `Vec<Line>` of the bullet row of the focused backlink
/// block, or `None` when focus is on the outline. `inner_width` is the
/// drawable width of the outline panel (i.e. excluding the 2-col
/// border) so the separator rule spans the full visible width.
///
/// Empty result `(vec![], None)` when there are no backlinks for the
/// current view or the user has toggled the section off via `B`.
pub(crate) fn render_backlinks_inline(
    app: &App,
    inner_width: u16,
) -> (Vec<Line<'static>>, Option<usize>) {
    if !app.show_backlinks {
        return (Vec::new(), None);
    }
    let backlinks = app.backlinks_for_current();
    if backlinks.is_empty() {
        return (Vec::new(), None);
    }

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut selected_line: Option<usize> = None;

    // Full-width rule and section header. The blank line above the
    // rule isolates the section visually from the outline.
    out.push(Line::from(""));
    let rule = "─".repeat(inner_width.max(1) as usize);
    out.push(Line::from(Span::styled(rule, app.theme.border)));
    out.push(Line::from(Span::styled(
        format!(" Backlinks · {} ref(s)", backlinks.len()),
        app.theme.heading,
    )));
    out.push(Line::from(""));

    let mut prev_source: Option<String> = None;
    for (idx, bl) in backlinks.iter().enumerate() {
        let source_slug = bl
            .source_page
            .as_ref()
            .map(|p| p.slug.as_str())
            .unwrap_or("");
        let source_title = bl
            .source_page
            .as_ref()
            .map(|p| p.title.as_str())
            .unwrap_or("");
        let source_icon = bl.source_page.as_ref().and_then(|p| p.icon.as_deref());
        // Header per source page. Multiple backlinks from the same
        // page collapse under one header.
        if prev_source.as_deref() != Some(source_slug) {
            if prev_source.is_some() {
                out.push(Line::from(""));
            }
            let header = match source_icon {
                Some(icon) => format!("{icon}  {source_title}"),
                None => format!("📄  {source_title}"),
            };
            out.push(Line::from(Span::styled(header, app.theme.heading)));
            prev_source = Some(source_slug.to_string());
        }

        // Sub-path inside the source_block that's currently focused,
        // if any. The mini-outline highlights it the same way the
        // main outline highlights `app.selected`.
        let focus_path: Option<&[usize]> = match &app.focus {
            Focus::Backlink {
                idx: fidx,
                sub_path,
            } if *fidx == idx => Some(sub_path.as_slice()),
            _ => None,
        };

        let mut current_path: Vec<usize> = Vec::new();
        render_backlink_node(
            bl,
            &bl.source_block,
            0,
            &mut current_path,
            focus_path,
            app,
            &mut out,
            &mut selected_line,
        );
    }

    (out, selected_line)
}

/// Recursive helper: emit a backlink's source block (and its children)
/// as a mini-outline. Tracks `current_path` to know when to flag the
/// focused row, and consults `app.mode` to render an in-place edit
/// buffer when the cursor sits on the block currently being edited
/// (cross-page Insert through `EditTarget::SourcePage`).
#[allow(clippy::too_many_arguments)]
fn render_backlink_node(
    bl: &Backlink,
    node: &OutlineNode,
    indent: u32,
    current_path: &mut Vec<usize>,
    focus_path: Option<&[usize]>,
    app: &App,
    out: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
) {
    let is_focused = focus_path == Some(current_path.as_slice());
    if is_focused && selected_line.is_none() {
        *selected_line = Some(out.len());
    }

    let bullet_style = if is_focused {
        app.theme.selected_bullet
    } else {
        app.theme.bullet
    };

    // Is this exact block the one the user is editing in place?
    // `Mode::Insert.block_path` lives in the *source page* coordinate
    // system; reconstruct the equivalent absolute path here and
    // compare. When it matches, render the live buffer with a caret
    // (same UX as outline Insert).
    let editing_here = is_focused
        && match &app.mode {
            Mode::Insert {
                target: EditTarget::SourcePage { path, .. },
                block_path,
                ..
            } if Some(path.as_path()) == bl.source_path.as_deref() => {
                let prefix = &bl.source_block_path;
                block_path.len() == prefix.len() + current_path.len()
                    && block_path.starts_with(prefix)
                    && &block_path[prefix.len()..] == current_path.as_slice()
            }
            _ => false,
        };

    // Source block carries the body without the TODO/DONE prefix —
    // reattach it so the outline renderer shows the same checkbox
    // decoration the main outline uses on the source page.
    let raw_text = match node.todo {
        Some(state) => format!("{} {}", state.as_str(), node.text),
        None => node.text.clone(),
    };

    let mode = if editing_here {
        if let Mode::Insert { buffer, .. } = &app.mode {
            RenderMode::Editing {
                text: buffer.as_string(),
                cursor_char: buffer.cursor,
            }
        } else {
            unreachable!("editing_here matched but mode isn't Insert")
        }
    } else if is_focused && matches!(app.mode, Mode::Normal) {
        RenderMode::NormalCursor {
            text: raw_text.clone(),
            cursor_char: app.cursor_col,
        }
    } else {
        RenderMode::Pretty {
            text: raw_text.clone(),
        }
    };

    let has_auto_run = node.properties.iter().any(|(k, _)| k == "auto-run");
    // Backlinks render a *projection* of a source block — fold state
    // belongs to the source page's outline, not here. Always pass
    // `None` so the layout stays flush.
    emit_block_lines(
        indent,
        bullet_style,
        &mode,
        has_auto_run,
        crate::view::outline::FoldMarker::None,
        app,
        out,
    );

    for (k, v) in &node.properties {
        let mut spans: Vec<Span<'_>> = Vec::new();
        for _ in 0..indent {
            spans.push(Span::styled("│ ", app.theme.dim));
        }
        spans.push(Span::raw("  ".to_string()));
        spans.push(Span::styled(format!("{k}:: "), app.theme.property_key));
        spans.push(Span::styled(v.clone(), app.theme.property_value));
        out.push(Line::from(spans));
    }

    for (i, child) in node.children.iter().enumerate() {
        current_path.push(i);
        render_backlink_node(
            bl,
            child,
            indent + 1,
            current_path,
            focus_path,
            app,
            out,
            selected_line,
        );
        current_path.pop();
    }
}
