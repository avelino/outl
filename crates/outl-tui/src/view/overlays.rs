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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

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
        AutocompleteKind::BlockRef => format!("(({}))", ac.query),
        AutocompleteKind::SlashCommand => format!("/{}", ac.query),
        AutocompleteKind::Mention => format!("@{}", ac.query),
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
                AutocompleteKind::PageRef | AutocompleteKind::Tag | AutocompleteKind::Mention => {
                    // Both PageRef and Mention list candidates by
                    // **title** (`by_title`); Tag lists by slug.
                    let icon = match ac.kind {
                        AutocompleteKind::PageRef | AutocompleteKind::Mention => {
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
                AutocompleteKind::BlockRef => {
                    // `c` is the handle. Resolve to the block's text
                    // for display — that's what the user is hunting
                    // for; the raw handle would be unreadable.
                    let text = app
                        .index
                        .resolve_block_ref(c)
                        .map(|b| b.text.clone())
                        .unwrap_or_else(|| c.clone());
                    ListItem::new(Line::from(vec![
                        Span::styled(text, style),
                        Span::styled(format!("  {c}"), app.theme.dim),
                    ]))
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
        .style(app.theme.popup_style());
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
    // Wider overlay (80%) so the preview pane has room to show real
    // outline context, not 5-char truncations.
    let area = centered_rect(full, 80, 70);
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
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    // Telescope-style split: list on the left, preview on the right.
    // The preview re-reads the highlighted page from disk per frame —
    // cheap for a single page, and avoids leaking a page cache into
    // App state for a feature that's open for ~5 seconds at a time.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(outer[1]);

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
                .title(format!("{} matches  ↑↓ Enter Esc", qs.candidates.len())),
        )
        .style(app.theme.popup_style());
    f.render_widget(list, cols[0]);

    render_preview_pane(f, cols[1], app, qs);
}

/// Right-hand preview pane for the quick switcher. Shows the first
/// ~N blocks of the highlighted candidate, or a placeholder when the
/// candidate isn't indexed yet (cold-start race) / has no body.
fn render_preview_pane(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, qs: &QuickSwitchState) {
    let Some(candidate) = qs.candidates.get(qs.selected) else {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  (type to search)",
            app.theme.dim,
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(" preview ", app.theme.help_title)),
        )
        .style(app.theme.popup_style());
        f.render_widget(empty, area);
        return;
    };

    // One-slot cache: the renderer is called on every poll tick, but
    // the underlying file changes only when the user touches j/k (or
    // edits a page elsewhere). Re-read only when the cached key
    // doesn't match the current candidate.
    let path = app.index.by_slug(&candidate.key).map(|e| e.path.clone());
    let cached_text: Option<String> = {
        let mut slot = qs.preview_cache.borrow_mut();
        let hit = matches!(slot.as_ref(), Some((k, _)) if k == &candidate.key);
        if !hit {
            *slot = path
                .as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|text| (candidate.key.clone(), text));
        }
        slot.as_ref().map(|(_, text)| text.clone())
    };

    let body_lines: Vec<Line<'static>> = match (path.is_some(), cached_text) {
        (true, Some(text)) => preview_lines(&text, area.height.saturating_sub(2) as usize, app),
        (true, None) => vec![Line::from(Span::styled(
            "  (couldn't read file)",
            app.theme.dim,
        ))],
        (false, _) => vec![Line::from(Span::styled(
            "  (not yet indexed)",
            app.theme.dim,
        ))],
    };

    let title_prefix = match candidate.kind {
        SwitchKind::Page => "📄 ",
        SwitchKind::Journal => "📅 ",
    };
    let preview = Paragraph::new(body_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(
                    format!(" {title_prefix}{} ", candidate.label),
                    app.theme.help_title,
                )),
        )
        .style(app.theme.popup_style())
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(preview, area);
}

/// Cheap markdown → preview lines. Doesn't reuse the outline renderer
/// (we don't want cursors, TODO checkboxes, etc.) — just enough to
/// give the user a sense of what they're about to open.
fn preview_lines(text: &str, max: usize, app: &App) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for line in text.lines() {
        if out.len() >= max {
            break;
        }
        let trimmed_start = line.trim_start();
        if trimmed_start.is_empty() {
            out.push(Line::raw(""));
            continue;
        }
        if let Some(rest) = trimmed_start.strip_prefix("- ") {
            let indent_chars = line.len() - trimmed_start.len();
            let indent = " ".repeat(indent_chars);
            out.push(Line::from(vec![
                Span::raw(indent),
                Span::styled("• ", app.theme.bullet),
                Span::raw(rest.to_string()),
            ]));
        } else if trimmed_start.contains("::") {
            // property line — show dimmer to de-emphasize.
            out.push(Line::from(Span::styled(line.to_string(), app.theme.dim)));
        } else {
            out.push(Line::raw(line.to_string()));
        }
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled("  (empty page)", app.theme.dim)));
    }
    out
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
    .style(app.theme.popup_style());
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
        .style(app.theme.popup_style())
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(list, outer[1]);
}

pub(crate) fn render_slash_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    s: &SlashState,
) {
    let area = centered_rect(full, 65, 65);
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
            .title(Span::styled(" Command palette ", app.theme.help_title)),
    )
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    // Group candidates by category, then render section headers
    // inline. We need the original index (into `s.candidates`) to
    // keep the highlight in sync with the selection, so we walk the
    // list once and stash `(original_index, command, category)`
    // tuples bucketed by category.
    let mut buckets: Vec<(&str, Vec<(usize, &crate::state::SlashCommand)>)> = Vec::new();
    for (i, c) in s.candidates.iter().enumerate() {
        let cat = category_for(c.name);
        // Insert preserving the canonical category order rather than
        // first-seen, so the palette layout is stable as the user
        // types and the candidate set shifts.
        if let Some(b) = buckets.iter_mut().find(|(k, _)| *k == cat) {
            b.1.push((i, c));
        } else {
            buckets.push((cat, vec![(i, c)]));
        }
    }
    buckets.sort_by_key(|(k, _)| category_order(k));

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Track which visual row the highlighted command landed on so
    // we can scroll the Paragraph to keep it in view when category
    // headers + blank rows push it past the visible height.
    let mut highlighted_row: Option<usize> = None;
    for (cat, items) in &buckets {
        lines.push(Line::from(Span::styled(
            format!(" {} {} ", category_icon(cat), cat),
            app.theme.help_title,
        )));
        for (orig_idx, c) in items {
            if *orig_idx == s.selected {
                highlighted_row = Some(lines.len());
            }
            let style = if *orig_idx == s.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            let suffix = if c.needs_args { " …" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {}  {}{suffix}  ", command_icon(c.name), c.name),
                    style,
                ),
                Span::styled(c.description.to_string(), app.theme.dim),
            ]));
        }
        lines.push(Line::raw(""));
    }

    // Viewport height inside the bordered block.
    let inner_h = outer[1].height.saturating_sub(2) as usize;
    let total = lines.len();
    // Auto-scroll: when the highlighted row would render past the
    // bottom of the viewport, push the scroll offset just enough to
    // bring it back into view. Clamps so the bottom of the list
    // doesn't scroll past the last row.
    let scroll: u16 = match highlighted_row {
        Some(row) if inner_h > 0 && row >= inner_h => {
            let max = total.saturating_sub(inner_h);
            ((row + 1).saturating_sub(inner_h)).min(max) as u16
        }
        _ => 0,
    };

    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(" {} commands · ↑↓ Enter Esc ", s.candidates.len())),
        )
        .style(app.theme.popup_style())
        .scroll((scroll, 0));
    f.render_widget(list, outer[1]);
}

/// Bucket a command name into a coarse category. Names follow loose
/// prefix conventions (`date-*`, `time-*`, `iso-*`, `week-*`, …) so
/// we can group without each command having to declare its category.
fn category_for(name: &str) -> &'static str {
    let n = name;
    if n.starts_with("date")
        || n.starts_with("time")
        || n.starts_with("iso")
        || n.starts_with("week")
        || n == "stamp"
        || n == "dt"
        || n == "dtm"
        || n == "dy"
    {
        "Dates & time"
    } else if n == "search" || n == "find" {
        "Search"
    } else if n == "theme" || n == "set" || n == "config" {
        "Settings"
    } else if n == "open" || n == "switch" || n == "quit" || n == "q" {
        "Navigation"
    } else {
        "Actions"
    }
}

/// Canonical sort order — Actions first (most common), Dates last
/// (long list, scrolls off).
fn category_order(cat: &str) -> u8 {
    match cat {
        "Actions" => 0,
        "Navigation" => 1,
        "Search" => 2,
        "Settings" => 3,
        "Dates & time" => 4,
        _ => 5,
    }
}

fn category_icon(cat: &str) -> &'static str {
    match cat {
        "Actions" => "⚡",
        "Navigation" => "↪",
        "Search" => "🔎",
        "Settings" => "⚙",
        "Dates & time" => "📅",
        _ => "•",
    }
}

/// Per-command leading glyph. Falls back to a dot for anything we
/// haven't curated.
fn command_icon(name: &str) -> &'static str {
    match name {
        "run" => "▶",
        "prop" => "≡",
        "search" | "find" => "🔎",
        "theme" => "🎨",
        "open" | "switch" => "↪",
        "quit" | "q" => "✕",
        n if n.starts_with("date") || n == "dt" || n == "dy" || n == "dtm" => "📅",
        n if n.starts_with("time") => "🕐",
        n if n.starts_with("iso") => "🔢",
        n if n.starts_with("week") => "📆",
        "stamp" => "🕒",
        _ => "·",
    }
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
        .style(app.theme.popup_style())
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
        .style(app.theme.popup_style());
    f.render_widget(bar, area);
}

/// Tab titles for the help popup, in the order they appear. The
/// `App.help_tab` index points into this slice (saturating, so an
/// out-of-range value clamps to the last tab).
pub(crate) const HELP_TABS: &[&str] =
    &["Normal", "Insert", "Visual", "Sidebar", "Overlays", "Dates"];

pub(crate) fn render_help_popup(f: &mut ratatui::Frame<'_>, full: Rect, app: &App) {
    let popup_w = (full.width as f32 * 0.7) as u16;
    let popup_h = 28u16.min(full.height.saturating_sub(2));
    let x = (full.width.saturating_sub(popup_w)) / 2;
    let y = (full.height.saturating_sub(popup_h)) / 2;
    let area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let tab = app.help_tab.min(HELP_TABS.len() - 1);
    let tabs = Tabs::new(
        HELP_TABS
            .iter()
            .map(|t| Line::from(format!(" {t} ")))
            .collect::<Vec<_>>(),
    )
    .select(tab)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled(
                " Help · h/l tabs · j/k scroll · PgUp/PgDn page · g/G top/end · ? close ",
                app.theme.help_title,
            )),
    )
    .style(
        app.theme
            .popup_style()
            .fg(app.theme.dim.fg.unwrap_or(ratatui::style::Color::Gray)),
    )
    .highlight_style(app.theme.list_selected)
    .divider(Span::styled("│", app.theme.dim));
    f.render_widget(tabs, chunks[0]);

    let body = help_tab_body(tab, app);
    let body_len = body.len() as u16;
    // Inner height = block area minus the 2 border rows.
    let inner_h = chunks[1].height.saturating_sub(2);
    // Clamp the requested scroll against the actual body so `G` /
    // PgDn don't park the user past the end of the content.
    let max_scroll = body_len.saturating_sub(inner_h);
    let scroll = app.help_scroll.min(max_scroll);

    // Title carries a scroll indicator when the content overflows —
    // gives the user a visual cue that there's more below / above.
    let title = if body_len > inner_h {
        format!(" {} · ↕ {}/{} ", HELP_TABS[tab], scroll + 1, max_scroll + 1)
    } else {
        format!(" {} ", HELP_TABS[tab])
    };
    let popup = Paragraph::new(body)
        .block(
            Block::default()
                .borders(Borders::ALL)
                // Highlight the border so it reads as "this owns focus"
                // — the dim outline border behind would otherwise
                // suggest the popup is informational.
                .border_style(app.theme.heading)
                .title(Span::styled(title, app.theme.help_title)),
        )
        .style(app.theme.popup_style())
        .scroll((scroll, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(popup, chunks[1]);
}

fn help_tab_body(tab: usize, app: &App) -> Vec<Line<'static>> {
    match HELP_TABS.get(tab).copied().unwrap_or("Normal") {
        "Normal" => vec![
            Line::from(Span::styled("Editing", app.theme.help_title)),
            Line::from("  i           edit current block"),
            Line::from("  I           edit, cursor at start of block"),
            Line::from("  o / O       new block below / above"),
            Line::from("  Tab / S-Tab indent / outdent"),
            Line::from("  K / J       move block up / down (Alt+↑/↓ too)"),
            Line::from("  dd          delete block"),
            Line::from("  yy / p / P  yank · paste after · paste before"),
            Line::from("  Ctrl+T      cycle TODO / DONE / none"),
            Line::from("  c           fold / unfold the selected block"),
            Line::from("              (▼ expanded · ▶ collapsed · synced via op log)"),
            Line::from("  u / Ctrl+R  undo / redo"),
            Line::from("  g p         toggle pinned:: on this page (chord)"),
            Line::from(""),
            Line::from(Span::styled("Navigation", app.theme.help_title)),
            Line::from("  j/k ↑↓      move between blocks"),
            Line::from("  PgDn/PgUp   one viewport"),
            Line::from("  Ctrl+D / U  half-page"),
            Line::from("  g g / G     first / last block"),
            Line::from("  h/l ←→      cursor inside the current block"),
            Line::from("  w / b       next / previous word"),
            Line::from("  0 / $       start / end of block"),
            Line::from("  Enter       open [[ref]] / #tag / journal under cursor"),
            Line::from(""),
            Line::from(Span::styled("Journal & workspace", app.theme.help_title)),
            Line::from("  t           today's journal"),
            Line::from("  [ / ]       previous / next journal"),
            Line::from("  g j         jump to today"),
            Line::from("  g x         run code block under cursor (also `:run`)"),
            Line::from("  Ctrl+S      force save"),
            Line::from("  Ctrl+L      reload workspace from disk"),
            Line::from("  B           toggle inline backlinks"),
            Line::from("  \\           toggle left sidebar (opens with focus on Pinned)"),
            Line::from("  q q         quit (chord)"),
        ],
        "Insert" => vec![
            Line::from(Span::styled("Commit / cancel", app.theme.help_title)),
            Line::from("  Esc         commit (write buffer → AST → disk)"),
            Line::from("  Enter       commit + new block below"),
            Line::from(""),
            Line::from(Span::styled(
                "Block ops (stay in Insert)",
                app.theme.help_title,
            )),
            Line::from("  Tab / S-Tab indent / outdent"),
            Line::from("  Ctrl+T      cycle TODO / DONE / none"),
            Line::from(""),
            Line::from(Span::styled("Text editing", app.theme.help_title)),
            Line::from("  chars       insert at cursor"),
            Line::from("  Backspace   delete previous (deletes block if empty)"),
            Line::from("  arrows/home/end   move cursor"),
            Line::from("  ( [ {       auto-pair with matching close"),
            Line::from(""),
            Line::from(Span::styled("Autocomplete", app.theme.help_title)),
            Line::from("  [[          page-ref picker"),
            Line::from("  #           tag picker"),
            Line::from("  /           slash command picker"),
        ],
        "Visual" => vec![
            Line::from(Span::styled("Selection", app.theme.help_title)),
            Line::from("  V           enter Visual (Normal mode)"),
            Line::from("  j / k       extend selection"),
            Line::from("  Esc         cancel"),
            Line::from(""),
            Line::from(Span::styled("Batch ops on the range", app.theme.help_title)),
            Line::from("  d / x       delete selected blocks"),
            Line::from("  y           yank selected blocks"),
            Line::from("  Tab / S-Tab indent / outdent the range"),
        ],
        "Sidebar" => vec![
            Line::from(Span::styled("Open / close", app.theme.help_title)),
            Line::from("  \\           toggle sidebar (opens with focus on Pinned)"),
            Line::from("  Esc         return focus to the outline (sidebar stays open)"),
            Line::from(""),
            Line::from(Span::styled("Inside the sidebar", app.theme.help_title)),
            Line::from("  j / k ↑↓    move between items in the focused section"),
            Line::from("  g / G       first / last item"),
            Line::from("  Tab / S-Tab cycle sections (Pinned → Recent → Calendar)"),
            Line::from("  Enter       open the highlighted page or journal"),
            Line::from(""),
            Line::from(Span::styled("Sections", app.theme.help_title)),
            Line::from("  📅 Calendar  current month — journals marked with ●"),
            Line::from("  ⭐ Pinned    pages with `pinned:: true` property"),
            Line::from("              (toggle with `gp` chord in Normal, or `/pin`)"),
            Line::from("  🕘 Recent    pages opened this session (LRU, cap 20)"),
        ],
        "Overlays" => vec![
            Line::from(Span::styled("Open", app.theme.help_title)),
            Line::from("  Ctrl+P      quick switcher (pages + journals, with preview)"),
            Line::from("  /           slash command menu (Notion-style)"),
            Line::from("  :           vim-style palette (same registry as /)"),
            Line::from("  ?           toggle this help"),
            Line::from(""),
            Line::from(Span::styled("Inside an overlay", app.theme.help_title)),
            Line::from("  ↑↓ j k      navigate candidates"),
            Line::from("  Enter       accept / run / open"),
            Line::from("  Esc         dismiss"),
            Line::from(""),
            Line::from(Span::styled("Search hits", app.theme.help_title)),
            Line::from("  n / N       next / previous hit (after `/` is closed)"),
        ],
        "Dates" => vec![
            Line::from(Span::styled(
                "Insert-mode slash commands",
                app.theme.help_title,
            )),
            Line::from("  /date-today          [[YYYY-MM-DD]]  (also /dt, /dtm, /dy)"),
            Line::from("  /date-next-monday    next Monday's journal ref"),
            Line::from("                       (one alias per weekday)"),
            Line::from("  /date +3d            offset: +Nd, -Nw, +Nm  or absolute YYYY-MM-DD"),
            Line::from("  /time-now            HH:MM, plain (no brackets)"),
            Line::from("  /datetime-now        [[YYYY-MM-DD]] HH:MM  (alias /stamp)"),
            Line::from("  /iso-date-today      YYYY-MM-DD, no brackets (for `due::` etc)"),
            Line::from("  /week-num            #YYYY-Www  (ISO week as a tag)"),
            Line::from(""),
            Line::from(Span::styled(
                format!("theme: {}", app.theme.name),
                app.theme.dim,
            )),
        ],
        _ => vec![Line::from("  (no content for this tab)")],
    }
}
