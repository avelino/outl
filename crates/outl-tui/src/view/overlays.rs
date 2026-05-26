//! Modal overlays: quick switcher, search, slash menu, command bar,
//! error popup, help popup, and the inline autocomplete dropdown.
//!
//! Each function takes the full frame `Rect` and centers / anchors its
//! own popup inside. `render_app` in the parent module dispatches based
//! on `app.overlay`.

use crate::state::{
    App, AutocompleteKind, AutocompleteState, CommandState, ErrorState, QuickSwitchState,
    SearchState, SlashState, SwitchKind,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

pub(crate) fn render_autocomplete(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    ac: &AutocompleteState,
) {
    let height = (ac.candidates.len() as u16 + 2).min(10);
    if height < 3 {
        return;
    }
    let width = 36u16.min(full.width.saturating_sub(4));
    // Bottom-right anchor so it doesn't fight with the outline.
    let area = Rect {
        x: full.x + full.width.saturating_sub(width + 2),
        y: full.y + full.height.saturating_sub(height + 2),
        width,
        height,
    };
    f.render_widget(Clear, area);
    let title = match ac.kind {
        AutocompleteKind::PageRef => format!("[[{}]]", ac.query),
        AutocompleteKind::Tag => format!("#{}", ac.query),
        AutocompleteKind::SlashCommand => format!("/{}", ac.query),
    };
    let items: Vec<ListItem<'_>> = ac
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let style = if i == ac.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            // Decorate the candidate row according to its kind. For
            // pages/tags we prepend the page's `icon::` (display-only);
            // for slash commands we append a dim description so the
            // popup doubles as in-context help.
            match ac.kind {
                AutocompleteKind::PageRef | AutocompleteKind::Tag => {
                    let icon = match ac.kind {
                        AutocompleteKind::PageRef => {
                            app.index.by_title(c).and_then(|p| p.icon.clone())
                        }
                        AutocompleteKind::Tag => app.index.by_slug(c).and_then(|p| p.icon.clone()),
                        _ => None,
                    };
                    let label = match icon {
                        Some(ic) => format!("{ic} {c}"),
                        None => c.clone(),
                    };
                    ListItem::new(Line::from(Span::styled(label, style)))
                }
                AutocompleteKind::SlashCommand => {
                    let cmd = app.command_registry.get(c);
                    let description = cmd.as_ref().map(|cmd| cmd.description()).unwrap_or("");
                    let needs_args = cmd.as_ref().map(|cmd| cmd.needs_args()).unwrap_or(false);
                    let suffix = if needs_args { " …" } else { "" };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{c}{suffix}  "), style),
                        Span::styled(description.to_string(), app.theme.dim),
                    ]))
                }
            }
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(title, app.theme.help_title)),
        )
        .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(list, area);
}

fn centered_rect(full: Rect, w_pct: u16, h_pct: u16) -> Rect {
    let w = (full.width as u32 * w_pct as u32 / 100) as u16;
    let h = (full.height as u32 * h_pct as u32 / 100) as u16;
    Rect {
        x: full.x + (full.width.saturating_sub(w)) / 2,
        y: full.y + (full.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

pub(crate) fn render_quick_switch(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    qs: &QuickSwitchState,
) {
    let area = centered_rect(full, 60, 60);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" › ", app.theme.help_title),
        Span::raw(qs.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled("Quick Switcher", app.theme.help_title)),
    )
    .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(input, outer[0]);

    let items: Vec<ListItem<'_>> = qs
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let icon = match c.kind {
                SwitchKind::Page => "📄 ",
                SwitchKind::Journal => "📅 ",
            };
            let style = if i == qs.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::raw(icon),
                Span::styled(c.label.clone(), style),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(
                    "{} matches  ↑↓ navigate · Enter open · Esc cancel",
                    qs.candidates.len()
                )),
        )
        .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(list, outer[1]);
}

pub(crate) fn render_search_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    s: &SearchState,
) {
    let area = centered_rect(full, 75, 70);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" / ", app.theme.help_title),
        Span::raw(s.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled("Search", app.theme.help_title)),
    )
    .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(input, outer[0]);

    let lines: Vec<Line<'_>> = s
        .hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let style = if i == s.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            let icon_prefix = h
                .page_icon
                .as_deref()
                .map(|i| format!("{i} "))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!(" {icon_prefix}{} · ", h.page_label), app.theme.dim),
                Span::styled(h.snippet.clone(), style),
            ])
        })
        .collect();
    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(
                    "{} hits  ↑↓ navigate · Enter jump · Esc cancel",
                    s.hits.len()
                )),
        )
        .style(Style::default().bg(app.theme.popup_bg))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(list, outer[1]);
}

pub(crate) fn render_slash_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    s: &SlashState,
) {
    let area = centered_rect(full, 60, 60);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" / ", app.theme.help_title),
        Span::raw(s.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled("Commands", app.theme.help_title)),
    )
    .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(input, outer[0]);

    let items: Vec<ListItem<'_>> = s
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let style = if i == s.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            // Two-column-ish: name on the left, description dimmed
            // on the right. The `…` glyph hints at "this one asks
            // for args next".
            let suffix = if c.needs_args { " …" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {}{suffix}  ", c.name), style),
                Span::styled(c.description.to_string(), app.theme.dim),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(
                    "{} commands  ↑↓ navigate · Enter run · Esc cancel",
                    s.candidates.len()
                )),
        )
        .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(list, outer[1]);
}

pub(crate) fn render_error_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    err: &ErrorState,
) {
    // Auto-size: pick 80% of the viewport, capped so a tiny body
    // doesn't draw a giant empty modal.
    let body_lines = err.body.lines().count().max(1) as u16;
    let popup_w = (full.width as f32 * 0.8) as u16;
    let popup_h = (body_lines + 4).min((full.height as f32 * 0.7) as u16);
    let x = (full.width.saturating_sub(popup_w)) / 2;
    let y = (full.height.saturating_sub(popup_h)) / 2;
    let area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };
    f.render_widget(Clear, area);

    let lines: Vec<Line<'_>> = err
        .body
        .lines()
        .map(|l| Line::from(Span::raw(l.to_string())))
        .collect();

    let title = format!(" ✕ {} · press any key to dismiss ", err.title);
    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.status_message)
                .title(Span::styled(title, app.theme.status_message)),
        )
        .style(Style::default().bg(app.theme.popup_bg))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(widget, area);
}

pub(crate) fn render_command_bar(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    c: &CommandState,
) {
    let h = 3u16;
    let area = Rect {
        x: full.x,
        y: full.y + full.height.saturating_sub(h),
        width: full.width,
        height: h,
    };
    f.render_widget(Clear, area);
    let line = Line::from(vec![
        Span::styled(" : ", app.theme.help_title),
        Span::raw(c.buffer.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]);
    let bar = Paragraph::new(line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border),
        )
        .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(bar, area);
}

pub(crate) fn render_help_popup(f: &mut ratatui::Frame<'_>, full: Rect, app: &App) {
    let popup_w = (full.width as f32 * 0.7) as u16;
    let popup_h = 34u16.min(full.height.saturating_sub(2));
    let x = (full.width.saturating_sub(popup_w)) / 2;
    let y = (full.height.saturating_sub(popup_h)) / 2;
    let area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };
    let body = vec![
        Line::from(Span::styled("NORMAL mode", app.theme.help_title)),
        Line::from("  i           edit current block"),
        Line::from("  I           edit, cursor at start of block"),
        Line::from("  o / O       new block below / above"),
        Line::from("  Enter       open [[ref]] / #tag / journal under cursor"),
        Line::from("              (falls back to edit if nothing matches)"),
        Line::from("  j / k / ↑ ↓ move between blocks"),
        Line::from("  PgDn/PgUp   move one viewport down/up"),
        Line::from("  Ctrl+D / U  half-page down/up"),
        Line::from("  g g / G     first / last block"),
        Line::from("  h / l / ← → move cursor inside the current block"),
        Line::from("  w / b       cursor to next / previous word"),
        Line::from("  0 / $       cursor to start / end of block"),
        Line::from("  Tab / S-Tab indent / outdent"),
        Line::from("  K / J       move block up / down (Alt+↑/↓ too)"),
        Line::from("  dd          delete block"),
        Line::from("  yy / p / P  yank · paste after · paste before"),
        Line::from("  Ctrl+T      cycle TODO / DONE / none (Ctrl+Enter on kitty-proto terminals)"),
        Line::from("  u           undo"),
        Line::from("  Ctrl+R      redo"),
        Line::from("  Ctrl+S      force save"),
        Line::from("  Ctrl+L      refresh workspace (re-read from disk)"),
        Line::from("  t           today's journal"),
        Line::from("  [ / ]       previous / next journal"),
        Line::from("  g j         jump to today"),
        Line::from("  g x         run code block under cursor (also `:run`)"),
        Line::from("  B           toggle inline backlinks section"),
        Line::from("  ?           toggle this help"),
        Line::from("  q q         quit (chord — single `q` arms it)"),
        Line::from(""),
        Line::from(Span::styled("Overlays", app.theme.help_title)),
        Line::from("  Ctrl+P      quick switcher (pages + journals)"),
        Line::from("  /           slash commands (prop, search, run, ...)"),
        Line::from("  n / N       next / previous search hit"),
        Line::from("  :           vim-style palette (same registry as /)"),
        Line::from(""),
        Line::from(Span::styled("INSERT mode", app.theme.help_title)),
        Line::from("  Esc         commit"),
        Line::from("  Enter       commit + new block below"),
        Line::from("  Ctrl+T      cycle TODO / DONE / none (stays in Insert; Ctrl+Enter on kitty-proto terminals)"),
        Line::from("  Tab / S-Tab indent / outdent (stays in Insert)"),
        Line::from("  [[ / #      autocomplete from existing page titles"),
        Line::from(""),
        Line::from(Span::styled(
            "Date inserters (Insert mode, via /)",
            app.theme.help_title,
        )),
        Line::from("  /date-today       [[YYYY-MM-DD]]  (also /dt, /dtm, /dy)"),
        Line::from("  /date-next-monday next Monday's journal ref (and one per weekday)"),
        Line::from("  /date +3d         offset or absolute: +Nd, -Nw, +Nm, YYYY-MM-DD"),
        Line::from("  /time-now         HH:MM, plain (no brackets)"),
        Line::from("  /datetime-now     [[YYYY-MM-DD]] HH:MM  (alias /stamp)"),
        Line::from("  /iso-date-today   YYYY-MM-DD, no brackets (for `due::` etc)"),
        Line::from("  /week-num         #YYYY-Www  (ISO week as a tag)"),
        Line::from(""),
        Line::from(Span::styled(
            format!("theme: {}", app.theme.name),
            app.theme.dim,
        )),
    ];
    let popup = Paragraph::new(body)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled("Help", app.theme.help_title)),
        )
        .style(Style::default().bg(app.theme.popup_bg));
    f.render_widget(Clear, area);
    f.render_widget(popup, area);
}
