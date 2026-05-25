//! TUI theming — color palette + ratatui [`Style`]s the renderer applies.
//!
//! A [`Theme`] bundles every styled surface in the TUI under semantic names
//! (`ref_link`, `tag_link`, `bold`, `code`, `cursor_block`, ...) so screen
//! code never references hard-coded colors. Built-in presets cover the
//! common terminal taste palettes; users override via `.outl/config.toml`:
//!
//! ```toml
//! [theme]
//! preset = "dracula"
//! ```
//!
//! Or temporarily via the CLI: `outl --theme nord --path ~/notes`.
//!
//! Adding a preset means adding one constructor function and a string in
//! [`PRESETS`]. The render path doesn't need to change.

use ratatui::style::{Color, Modifier, Style};

/// All styled surfaces the TUI knows how to paint.
///
/// Add a field here only when there's a *semantic* surface to name. If
/// two surfaces want exactly the same style, share the field. Don't add
/// `inner_bold_in_quote` and other compound names — keep it flat.
#[derive(Debug, Clone)]
pub struct Theme {
    /// User-visible preset name.
    pub name: &'static str,
    /// Background of the alternate screen.
    pub background: Color,

    // --- bullets / outline ---
    /// Plain `-` bullet when not selected.
    pub bullet: Style,
    /// Bullet of the currently selected block.
    pub selected_bullet: Style,
    /// Single-char block cursor (vim style) in Normal mode.
    pub cursor_block: Style,
    /// Thin caret used at end-of-line and in Insert mode.
    pub cursor_caret: Style,

    // --- inline tokens (consumed by render_markdown_inline / highlight_inline) ---
    /// `[[page]]` references (pretty: name only; raw: with brackets).
    pub ref_link: Style,
    /// `#tag` references.
    pub tag_link: Style,
    /// `[text](url)` standard markdown link (text only in pretty mode).
    pub md_link: Style,
    /// `**bold**` inner text.
    pub bold: Style,
    /// `*italic*` / `_italic_` inner text.
    pub italic: Style,
    /// `~~strike~~` inner text.
    pub strike: Style,
    /// `` `code` `` inner text.
    pub code: Style,
    /// Style for unfinished `TODO` prefix on a block.
    pub todo_open: Style,
    /// Style for finished `DONE` prefix on a block.
    pub todo_done: Style,
    /// Body text of a TODO block (carries `dim` for DONE).
    pub todo_done_body: Style,

    // --- structural surfaces ---
    /// `key:: value` property lines (the key part).
    pub property_key: Style,
    /// `key:: value` property lines (the value part).
    pub property_value: Style,
    /// Page heading shown in the main header.
    pub heading: Style,
    /// Dim, used for delimiters in raw render (`**`, `~~`, etc).
    pub dim: Style,

    // --- chrome ---
    /// Borders around panels.
    pub border: Style,
    /// Hint line in the footer.
    pub hint: Style,
    /// Mode badge in Normal mode.
    pub status_normal: Style,
    /// Mode badge in Insert mode.
    pub status_insert: Style,
    /// Mode badge in Visual mode (S2.2).
    pub status_visual: Style,
    /// Transient status messages (saved, error...).
    pub status_message: Style,
    /// Section titles in the help popup.
    pub help_title: Style,
    /// Popup background (quick switcher, command palette, help).
    pub popup_bg: Color,

    // --- accent rails ---
    /// Highlight bar when an item is selected in a list popup.
    pub list_selected: Style,
}

/// List of preset names exposed to users (CLI / config / `outl theme list`).
pub const PRESETS: &[&str] = &[
    "default-dark",
    "light",
    "dracula",
    "solarized-dark",
    "nord",
    "monokai",
];

/// Look up a preset by name. Case-insensitive; dashes and underscores
/// are interchangeable so `"Solarized Dark"` and `"solarized_dark"`
/// both work.
pub fn by_name(name: &str) -> Option<Theme> {
    let norm: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            '_' | ' ' => '-',
            _ => c,
        })
        .collect();
    match norm.as_str() {
        "default-dark" | "default" | "dark" => Some(default_dark()),
        "light" => Some(light()),
        "dracula" => Some(dracula()),
        "solarized-dark" | "solarized" => Some(solarized_dark()),
        "nord" => Some(nord()),
        "monokai" => Some(monokai()),
        _ => None,
    }
}

/// Default fallback when nothing is configured.
pub fn default_theme() -> Theme {
    default_dark()
}

// --- presets -------------------------------------------------------------

/// Default dark — the original outl-tui palette.
pub fn default_dark() -> Theme {
    Theme {
        name: "default-dark",
        background: Color::Reset,
        bullet: Style::default().fg(Color::DarkGray),
        selected_bullet: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default().add_modifier(Modifier::BOLD),
        italic: Style::default().add_modifier(Modifier::ITALIC),
        strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(Color::Green),
        todo_open: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        todo_done: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(Color::DarkGray),
        property_value: Style::default().fg(Color::Gray),
        heading: Style::default().add_modifier(Modifier::BOLD),
        dim: Style::default().fg(Color::DarkGray),
        border: Style::default().fg(Color::DarkGray),
        hint: Style::default().fg(Color::Gray),
        status_normal: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(Color::Yellow),
        help_title: Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        popup_bg: Color::Black,
        list_selected: Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    }
}

/// Light theme — for high-brightness terminals.
pub fn light() -> Theme {
    Theme {
        name: "light",
        background: Color::Reset,
        bullet: Style::default().fg(Color::Gray),
        selected_bullet: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(Color::White)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        italic: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::ITALIC),
        strike: Style::default().add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(Color::Magenta),
        todo_open: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        todo_done: Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(Color::Gray),
        property_value: Style::default().fg(Color::DarkGray),
        heading: Style::default()
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
        dim: Style::default().fg(Color::Gray),
        border: Style::default().fg(Color::Gray),
        hint: Style::default().fg(Color::DarkGray),
        status_normal: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(Color::White)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(Color::White)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(Color::Yellow),
        help_title: Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        popup_bg: Color::White,
        list_selected: Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    }
}

/// Dracula — popular dark palette.
pub fn dracula() -> Theme {
    let bg = Color::Rgb(40, 42, 54);
    let fg = Color::Rgb(248, 248, 242);
    let comment = Color::Rgb(98, 114, 164);
    let cyan = Color::Rgb(139, 233, 253);
    let green = Color::Rgb(80, 250, 123);
    let orange = Color::Rgb(255, 184, 108);
    let pink = Color::Rgb(255, 121, 198);
    let purple = Color::Rgb(189, 147, 249);
    let yellow = Color::Rgb(241, 250, 140);

    Theme {
        name: "dracula",
        background: bg,
        bullet: Style::default().fg(comment),
        selected_bullet: Style::default()
            .fg(bg)
            .bg(cyan)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default().fg(bg).bg(fg).add_modifier(Modifier::BOLD),
        cursor_caret: Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ref_link: Style::default().fg(cyan).add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default().fg(pink).add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(purple)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default().fg(orange).add_modifier(Modifier::BOLD),
        italic: Style::default().fg(yellow).add_modifier(Modifier::ITALIC),
        strike: Style::default()
            .fg(comment)
            .add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(green),
        todo_open: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        todo_done: Style::default().fg(green).add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(comment)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(comment),
        property_value: Style::default().fg(purple),
        heading: Style::default().fg(pink).add_modifier(Modifier::BOLD),
        dim: Style::default().fg(comment),
        border: Style::default().fg(comment),
        hint: Style::default().fg(comment),
        status_normal: Style::default()
            .fg(bg)
            .bg(cyan)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(bg)
            .bg(green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(bg)
            .bg(pink)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(yellow),
        help_title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        popup_bg: bg,
        list_selected: Style::default()
            .fg(bg)
            .bg(purple)
            .add_modifier(Modifier::BOLD),
    }
}

/// Solarized Dark — Ethan Schoonover's classic.
pub fn solarized_dark() -> Theme {
    let base03 = Color::Rgb(0, 43, 54);
    let base01 = Color::Rgb(88, 110, 117);
    let base0 = Color::Rgb(131, 148, 150);
    let base1 = Color::Rgb(147, 161, 161);
    let yellow = Color::Rgb(181, 137, 0);
    let orange = Color::Rgb(203, 75, 22);
    let red = Color::Rgb(220, 50, 47);
    let magenta = Color::Rgb(211, 54, 130);
    let violet = Color::Rgb(108, 113, 196);
    let blue = Color::Rgb(38, 139, 210);
    let cyan = Color::Rgb(42, 161, 152);
    let green = Color::Rgb(133, 153, 0);
    let _ = (orange, red, magenta);

    Theme {
        name: "solarized-dark",
        background: base03,
        bullet: Style::default().fg(base01),
        selected_bullet: Style::default()
            .fg(base03)
            .bg(cyan)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(base03)
            .bg(base1)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default().fg(base1).add_modifier(Modifier::BOLD),
        ref_link: Style::default().fg(cyan).add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(magenta)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default().fg(blue).add_modifier(Modifier::UNDERLINED),
        bold: Style::default().fg(orange).add_modifier(Modifier::BOLD),
        italic: Style::default().fg(yellow).add_modifier(Modifier::ITALIC),
        strike: Style::default()
            .fg(base01)
            .add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(green),
        todo_open: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        todo_done: Style::default().fg(green).add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(base01)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(base01),
        property_value: Style::default().fg(violet),
        heading: Style::default().fg(blue).add_modifier(Modifier::BOLD),
        dim: Style::default().fg(base01),
        border: Style::default().fg(base01),
        hint: Style::default().fg(base0),
        status_normal: Style::default()
            .fg(base03)
            .bg(cyan)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(base03)
            .bg(green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(base03)
            .bg(magenta)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(yellow),
        help_title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        popup_bg: base03,
        list_selected: Style::default()
            .fg(base03)
            .bg(blue)
            .add_modifier(Modifier::BOLD),
    }
}

/// Nord — Arctic palette.
pub fn nord() -> Theme {
    let nord0 = Color::Rgb(46, 52, 64);
    let nord3 = Color::Rgb(76, 86, 106);
    let nord4 = Color::Rgb(216, 222, 233);
    let nord6 = Color::Rgb(236, 239, 244);
    let nord7 = Color::Rgb(143, 188, 187);
    let nord8 = Color::Rgb(136, 192, 208);
    let nord11 = Color::Rgb(191, 97, 106);
    let nord13 = Color::Rgb(235, 203, 139);
    let nord14 = Color::Rgb(163, 190, 140);
    let nord15 = Color::Rgb(180, 142, 173);
    let _ = (nord4, nord11);

    Theme {
        name: "nord",
        background: nord0,
        bullet: Style::default().fg(nord3),
        selected_bullet: Style::default()
            .fg(nord0)
            .bg(nord8)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default()
            .fg(nord0)
            .bg(nord6)
            .add_modifier(Modifier::BOLD),
        cursor_caret: Style::default().fg(nord6).add_modifier(Modifier::BOLD),
        ref_link: Style::default()
            .fg(nord8)
            .add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default()
            .fg(nord15)
            .add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(nord7)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default().fg(nord13).add_modifier(Modifier::BOLD),
        italic: Style::default().fg(nord15).add_modifier(Modifier::ITALIC),
        strike: Style::default()
            .fg(nord3)
            .add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(nord14),
        todo_open: Style::default().fg(nord13).add_modifier(Modifier::BOLD),
        todo_done: Style::default().fg(nord14).add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(nord3)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(nord3),
        property_value: Style::default().fg(nord7),
        heading: Style::default().fg(nord8).add_modifier(Modifier::BOLD),
        dim: Style::default().fg(nord3),
        border: Style::default().fg(nord3),
        hint: Style::default().fg(nord3),
        status_normal: Style::default()
            .fg(nord0)
            .bg(nord8)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(nord0)
            .bg(nord14)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(nord0)
            .bg(nord15)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(nord13),
        help_title: Style::default().fg(nord13).add_modifier(Modifier::BOLD),
        popup_bg: nord0,
        list_selected: Style::default()
            .fg(nord0)
            .bg(nord8)
            .add_modifier(Modifier::BOLD),
    }
}

/// Monokai — Wimer Hazenberg's high-contrast palette.
pub fn monokai() -> Theme {
    let bg = Color::Rgb(39, 40, 34);
    let comment = Color::Rgb(117, 113, 94);
    let fg = Color::Rgb(248, 248, 242);
    let pink = Color::Rgb(249, 38, 114);
    let orange = Color::Rgb(253, 151, 31);
    let yellow = Color::Rgb(230, 219, 116);
    let green = Color::Rgb(166, 226, 46);
    let blue = Color::Rgb(102, 217, 239);
    let purple = Color::Rgb(174, 129, 255);

    Theme {
        name: "monokai",
        background: bg,
        bullet: Style::default().fg(comment),
        selected_bullet: Style::default()
            .fg(bg)
            .bg(pink)
            .add_modifier(Modifier::BOLD),
        cursor_block: Style::default().fg(bg).bg(fg).add_modifier(Modifier::BOLD),
        cursor_caret: Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ref_link: Style::default().fg(blue).add_modifier(Modifier::UNDERLINED),
        tag_link: Style::default().fg(pink).add_modifier(Modifier::UNDERLINED),
        md_link: Style::default()
            .fg(purple)
            .add_modifier(Modifier::UNDERLINED),
        bold: Style::default().fg(orange).add_modifier(Modifier::BOLD),
        italic: Style::default().fg(yellow).add_modifier(Modifier::ITALIC),
        strike: Style::default()
            .fg(comment)
            .add_modifier(Modifier::CROSSED_OUT),
        code: Style::default().fg(green),
        todo_open: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        todo_done: Style::default().fg(green).add_modifier(Modifier::BOLD),
        todo_done_body: Style::default()
            .fg(comment)
            .add_modifier(Modifier::CROSSED_OUT),
        property_key: Style::default().fg(comment),
        property_value: Style::default().fg(purple),
        heading: Style::default().fg(pink).add_modifier(Modifier::BOLD),
        dim: Style::default().fg(comment),
        border: Style::default().fg(comment),
        hint: Style::default().fg(comment),
        status_normal: Style::default()
            .fg(bg)
            .bg(blue)
            .add_modifier(Modifier::BOLD),
        status_insert: Style::default()
            .fg(bg)
            .bg(green)
            .add_modifier(Modifier::BOLD),
        status_visual: Style::default()
            .fg(bg)
            .bg(pink)
            .add_modifier(Modifier::BOLD),
        status_message: Style::default().fg(yellow),
        help_title: Style::default().fg(yellow).add_modifier(Modifier::BOLD),
        popup_bg: bg,
        list_selected: Style::default()
            .fg(bg)
            .bg(blue)
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_preset_resolves() {
        for name in PRESETS {
            assert!(by_name(name).is_some(), "preset {name} should resolve");
        }
    }

    #[test]
    fn lookup_is_case_and_separator_insensitive() {
        assert!(by_name("Dracula").is_some());
        assert!(by_name("DRACULA").is_some());
        assert!(by_name("solarized_dark").is_some());
        assert!(by_name("Solarized Dark").is_some());
        assert!(by_name("default-dark").is_some());
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(by_name("vampire").is_none());
        assert!(by_name("").is_none());
    }

    #[test]
    fn theme_name_matches_preset_id() {
        // Catches the "shipped Monokai but typo'd its name" class of bug.
        for name in PRESETS {
            let t = by_name(name).unwrap();
            assert_eq!(
                t.name, *name,
                "preset {name} resolved to a theme with name {}",
                t.name
            );
        }
    }
}
