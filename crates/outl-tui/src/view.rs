//! ratatui rendering: turn the current `App` into a frame.
//!
//! This file is the orchestrator — it composes the panels but delegates
//! the actual painting to siblings:
//!
//! - [`overlays`] — every modal popup (quick switcher, search, slash,
//!   command bar, error, help, inline autocomplete).
//! - [`outline`] — the current page's block tree.
//! - [`backlinks`] — the inline backlinks section below the outline.
//! - [`inline`] — span-level markdown (used by `outline` and
//!   `backlinks`).
//!
//! Only `render_app` is callable from outside the module — the rest is
//! `pub(crate)` for cross-file reuse inside `view/`.

mod backlinks;
mod inline;
mod outline;
mod overlays;

use crate::state::{
    App, Focus, Mode, Overlay, View, HELP_HINT_INSERT, HELP_HINT_NORMAL, HELP_HINT_VISUAL,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

// Inline-rendering helpers used by tests in `app.rs` — keep them
// reachable via `crate::view::…` for backwards compat with the
// pre-split layout.
#[cfg(test)]
pub(crate) use inline::{highlight_inline, render_markdown_inline, split_todo_prefix};

pub(crate) fn render_app(f: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = f.area();
    render_main(f, area, app);

    // Overlays draw on top of everything else.
    match &app.overlay {
        Some(Overlay::QuickSwitch(qs)) => overlays::render_quick_switch(f, area, app, qs),
        Some(Overlay::Search(s)) => overlays::render_search_overlay(f, area, app, s),
        Some(Overlay::Command(c)) => overlays::render_command_bar(f, area, app, c),
        Some(Overlay::Error(e)) => overlays::render_error_overlay(f, area, app, e),
        Some(Overlay::Slash(s)) => overlays::render_slash_overlay(f, area, app, s),
        None => {}
    }

    if let Some(ac) = &app.autocomplete {
        overlays::render_autocomplete(f, area, app, ac);
    }

    if app.show_help {
        overlays::render_help_popup(f, area, app);
    }
}

fn render_main(f: &mut ratatui::Frame<'_>, area: Rect, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    // Header: page title on the left, workspace/index info on the right.
    let workspace_label = app
        .workspace_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();
    let stats = format!(
        "  ws:{workspace_label}  pages:{}  blocks:{}",
        app.index.page_count(),
        app.flat_len
    );
    let header = Paragraph::new(Line::from(vec![
        Span::styled(app.current_title(), app.theme.heading),
        Span::styled(stats, app.theme.dim),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled(
                format!(" outl · {} ", app.theme.name),
                app.theme.hint,
            )),
    );
    f.render_widget(header, outer[0]);

    // Build the single scrollable region: outline lines, then the
    // inline backlinks section. The `─` separator and headers live
    // inside `backlinks::render_backlinks_inline`.
    let (mut all_lines, sel_outline) = outline::render_outline(&app.page, app);
    let outline_len = all_lines.len();
    let inner_width = outer[1].width.saturating_sub(2);
    let (bl_lines, sel_bl) = backlinks::render_backlinks_inline(app, inner_width);
    let bl_offset = all_lines.len();
    all_lines.extend(bl_lines);

    let title = match &app.view {
        View::Journal(_) | View::Page(_) => "Outline",
    };

    // Pick the selected line based on where the focus currently is.
    let selected_line = match &app.focus {
        Focus::Outline => sel_outline,
        Focus::Backlink { .. } => sel_bl.map(|n| n + bl_offset),
    };
    let _ = outline_len; // kept for future scroll heuristics

    // Viewport math: outer[1] is the outline area (borders included).
    // Subtract 2 for top + bottom border lines to get the actually
    // drawable region.
    let viewport_h = outer[1].height.saturating_sub(2);
    app.viewport_height = viewport_h;

    // Auto-scroll: keep the selection visible. If it scrolled off the
    // top, drop the offset down to it; if it scrolled off the bottom,
    // push the offset up so the bullet sits on the last row.
    if let Some(sel) = selected_line {
        let sel = sel as u16;
        if sel < app.scroll_y {
            app.scroll_y = sel;
        } else if viewport_h > 0 && sel >= app.scroll_y + viewport_h {
            app.scroll_y = sel + 1 - viewport_h;
        }
    }
    // Clamp: never scroll past `last_line - viewport_h + 1`.
    let total = all_lines.len() as u16;
    if total > viewport_h {
        let max_scroll = total - viewport_h;
        if app.scroll_y > max_scroll {
            app.scroll_y = max_scroll;
        }
    } else {
        app.scroll_y = 0;
    }

    let scroll_indicator = if total > viewport_h && viewport_h > 0 {
        format!(
            " ({}/{})",
            app.scroll_y + 1,
            total.saturating_sub(viewport_h) + 1
        )
    } else {
        String::new()
    };
    let body = Paragraph::new(all_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!("{title}{scroll_indicator}")),
        )
        .scroll((app.scroll_y, 0));
    // NB: no `.wrap(...)`. Wrap turns one logical line into N visual
    // lines whose count depends on width, which would invalidate our
    // `selected_line` index. We trade off-screen long lines (rare,
    // and you can horizontal-scroll later) for a correct vertical
    // scroll today.
    f.render_widget(body, outer[1]);

    let (mode_label, mode_style) = match app.mode {
        Mode::Normal => (" NORMAL ", app.theme.status_normal),
        Mode::Insert { .. } => (" INSERT ", app.theme.status_insert),
        Mode::Visual { .. } => (" VISUAL ", app.theme.status_visual),
    };
    let hint = match app.mode {
        Mode::Insert { .. } => HELP_HINT_INSERT,
        Mode::Visual { .. } => HELP_HINT_VISUAL,
        Mode::Normal => HELP_HINT_NORMAL,
    };
    // Backlink count for this view (when it matters).
    let bl_count = app.index.backlinks(&app.current_slug()).len();
    let bl_label = if bl_count == 0 {
        String::new()
    } else {
        format!(
            "  ⇇ {bl_count} backlink{}",
            if bl_count == 1 { "" } else { "s" }
        )
    };
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(mode_label, mode_style),
        Span::raw("  "),
        Span::styled(hint, app.theme.hint),
        Span::styled(bl_label, app.theme.dim),
        Span::raw("  "),
        Span::styled(&app.status, app.theme.status_message),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border),
    );
    f.render_widget(footer, outer[2]);
}
