//! Built-in palette presets.
//!
//! Each `fn` returns a [`Palette`] with every field populated. Hex
//! values are sourced from the TUI's pre-crate `theme.rs` so the
//! shipping look-and-feel is byte-identical after the refactor.
//!
//! When you add a preset, also extend
//! [`crate::PRESETS`] and [`crate::by_name`].

use crate::Palette;

/// outl brand palette — deep purple with lavender accent and lemon
/// highlight. Default for every client.
pub fn outl() -> Palette {
    let bg = "#0c0814";
    let bg_elev = "#15101f";
    let fg = "#f4f1fa";
    let fg_dim = "#b4adc7";
    let fg_dimmer = "#7b7390";
    let border = "#382c54";
    let accent = "#a78bfa";
    let accent_soft = "#c4b5fd";
    let accent_alt = "#d6ff47";
    let warn = "#fbbf24";
    let blue = "#7dd3fc";
    let magenta = "#f0abfc";

    Palette {
        name: "outl".into(),
        bg: bg.into(),
        bg_elev: bg_elev.into(),
        fg: fg.into(),
        fg_dim: fg_dim.into(),
        fg_dimmer: fg_dimmer.into(),
        border: border.into(),
        hint: fg_dim.into(),
        accent: accent.into(),
        accent_soft: accent_soft.into(),
        accent_alt: accent_alt.into(),
        warn: warn.into(),
        ref_link_fg: accent.into(),
        tag_link_fg: magenta.into(),
        md_link_fg: blue.into(),
        bold_fg: fg.into(),
        italic_fg: accent_soft.into(),
        strike_fg: fg_dimmer.into(),
        code_fg: accent_alt.into(),
        todo_open_fg: warn.into(),
        todo_done_fg: accent_alt.into(),
        todo_done_body_fg: fg_dimmer.into(),
        property_key_fg: fg_dimmer.into(),
        property_value_fg: accent_soft.into(),
        heading_fg: fg.into(),
        dim_fg: fg_dimmer.into(),
        selected_bullet_bg: accent.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: accent_soft.into(),
        status_normal_bg: accent.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: accent_alt.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: magenta.into(),
        status_visual_fg: bg.into(),
        status_message_fg: warn.into(),
        list_selected_bg: accent.into(),
        list_selected_fg: bg.into(),
        help_title_fg: accent_soft.into(),
    }
}

/// Default dark — ANSI 16-color equivalents in hex so the desktop
/// has something to paint. The TUI rendering of this preset on a
/// real terminal continues to use the terminal's native ANSI
/// palette (see `outl-tui/src/theme.rs::default_dark` which builds
/// from explicit `Color::*` values, not from this palette).
pub fn default_dark() -> Palette {
    let bg = "#000000";
    let fg = "#cccccc";
    let dark_gray = "#555555";
    let gray = "#aaaaaa";
    let cyan = "#00aaaa";
    let magenta = "#aa00aa";
    let blue = "#0066ff";
    let green = "#00aa00";
    let yellow = "#ffff55";

    Palette {
        name: "default-dark".into(),
        bg: bg.into(),
        bg_elev: "#101010".into(),
        fg: fg.into(),
        fg_dim: gray.into(),
        fg_dimmer: dark_gray.into(),
        border: dark_gray.into(),
        hint: gray.into(),
        accent: cyan.into(),
        accent_soft: "#55ffff".into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        ref_link_fg: cyan.into(),
        tag_link_fg: magenta.into(),
        md_link_fg: blue.into(),
        bold_fg: fg.into(),
        italic_fg: fg.into(),
        strike_fg: dark_gray.into(),
        code_fg: green.into(),
        todo_open_fg: yellow.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: dark_gray.into(),
        property_key_fg: dark_gray.into(),
        property_value_fg: gray.into(),
        heading_fg: fg.into(),
        dim_fg: dark_gray.into(),
        selected_bullet_bg: cyan.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: "#ffffff".into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: "#ffffff".into(),
        status_normal_bg: cyan.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: magenta.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: cyan.into(),
        list_selected_fg: bg.into(),
        help_title_fg: yellow.into(),
    }
}

/// Light palette for high-brightness environments.
pub fn light() -> Palette {
    let bg = "#ffffff";
    let fg = "#1a1a1a";
    let dark_gray = "#555555";
    let gray = "#888888";
    let blue = "#0066ff";
    let magenta = "#aa00aa";
    let red = "#cc0033";
    let green = "#007733";
    let yellow = "#aa6600";

    Palette {
        name: "light".into(),
        bg: bg.into(),
        bg_elev: "#f4f4f4".into(),
        fg: fg.into(),
        fg_dim: gray.into(),
        fg_dimmer: "#bbbbbb".into(),
        border: gray.into(),
        hint: dark_gray.into(),
        accent: blue.into(),
        accent_soft: "#3399ff".into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        ref_link_fg: blue.into(),
        tag_link_fg: red.into(),
        md_link_fg: blue.into(),
        bold_fg: fg.into(),
        italic_fg: fg.into(),
        strike_fg: gray.into(),
        code_fg: magenta.into(),
        todo_open_fg: red.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: gray.into(),
        property_key_fg: gray.into(),
        property_value_fg: dark_gray.into(),
        heading_fg: fg.into(),
        dim_fg: gray.into(),
        selected_bullet_bg: blue.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg.into(),
        status_normal_bg: blue.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: magenta.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: blue.into(),
        list_selected_fg: bg.into(),
        help_title_fg: blue.into(),
    }
}

/// Dracula — popular dark palette (zenorocha).
pub fn dracula() -> Palette {
    let bg = "#282a36";
    let fg = "#f8f8f2";
    let comment = "#6272a4";
    let cyan = "#8be9fd";
    let green = "#50fa7b";
    let orange = "#ffb86c";
    let pink = "#ff79c6";
    let purple = "#bd93f9";
    let yellow = "#f1fa8c";

    Palette {
        name: "dracula".into(),
        bg: bg.into(),
        bg_elev: "#21222c".into(),
        fg: fg.into(),
        fg_dim: comment.into(),
        fg_dimmer: comment.into(),
        border: comment.into(),
        hint: comment.into(),
        accent: cyan.into(),
        accent_soft: purple.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        ref_link_fg: cyan.into(),
        tag_link_fg: pink.into(),
        md_link_fg: purple.into(),
        bold_fg: orange.into(),
        italic_fg: yellow.into(),
        strike_fg: comment.into(),
        code_fg: green.into(),
        todo_open_fg: yellow.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: comment.into(),
        property_key_fg: comment.into(),
        property_value_fg: purple.into(),
        heading_fg: pink.into(),
        dim_fg: comment.into(),
        selected_bullet_bg: cyan.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg.into(),
        status_normal_bg: cyan.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: pink.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: cyan.into(),
        list_selected_fg: bg.into(),
        help_title_fg: yellow.into(),
    }
}

/// Solarized Dark — Ethan Schoonover's classic muted palette.
pub fn solarized_dark() -> Palette {
    let base03 = "#002b36";
    let base02 = "#073642";
    let base01 = "#586e75";
    let base0 = "#839496";
    let base1 = "#93a1a1";
    let yellow = "#b58900";
    let orange = "#cb4b16";
    let red = "#dc322f";
    let magenta = "#d33682";
    let violet = "#6c71c4";
    let blue = "#268bd2";
    let cyan = "#2aa198";
    let green = "#859900";

    Palette {
        name: "solarized-dark".into(),
        bg: base03.into(),
        bg_elev: base02.into(),
        fg: base0.into(),
        fg_dim: base01.into(),
        fg_dimmer: base01.into(),
        border: base02.into(),
        hint: base1.into(),
        accent: cyan.into(),
        accent_soft: violet.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        ref_link_fg: blue.into(),
        tag_link_fg: magenta.into(),
        md_link_fg: violet.into(),
        bold_fg: orange.into(),
        italic_fg: violet.into(),
        strike_fg: base01.into(),
        code_fg: green.into(),
        todo_open_fg: red.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: base01.into(),
        property_key_fg: base01.into(),
        property_value_fg: cyan.into(),
        heading_fg: base1.into(),
        dim_fg: base01.into(),
        selected_bullet_bg: cyan.into(),
        selected_bullet_fg: base03.into(),
        cursor_block_bg: base1.into(),
        cursor_block_fg: base03.into(),
        cursor_caret_fg: base1.into(),
        status_normal_bg: cyan.into(),
        status_normal_fg: base03.into(),
        status_insert_bg: green.into(),
        status_insert_fg: base03.into(),
        status_visual_bg: magenta.into(),
        status_visual_fg: base03.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: cyan.into(),
        list_selected_fg: base03.into(),
        help_title_fg: yellow.into(),
    }
}

/// Nord — arctic blue-greys, low contrast.
pub fn nord() -> Palette {
    let polar0 = "#2e3440";
    let polar1 = "#3b4252";
    let snow0 = "#d8dee9";
    let snow1 = "#e5e9f0";
    let snow2 = "#eceff4";
    let frost1 = "#88c0d0";
    let frost2 = "#81a1c1";
    let frost3 = "#5e81ac";
    let aurora_red = "#bf616a";
    let aurora_orange = "#d08770";
    let aurora_yellow = "#ebcb8b";
    let aurora_green = "#a3be8c";
    let aurora_purple = "#b48ead";

    Palette {
        name: "nord".into(),
        bg: polar0.into(),
        bg_elev: polar1.into(),
        fg: snow1.into(),
        fg_dim: snow0.into(),
        fg_dimmer: "#4c566a".into(),
        border: polar1.into(),
        hint: snow0.into(),
        accent: frost1.into(),
        accent_soft: frost2.into(),
        accent_alt: aurora_green.into(),
        warn: aurora_yellow.into(),
        ref_link_fg: frost1.into(),
        tag_link_fg: aurora_purple.into(),
        md_link_fg: frost3.into(),
        bold_fg: aurora_orange.into(),
        italic_fg: frost2.into(),
        strike_fg: "#4c566a".into(),
        code_fg: aurora_green.into(),
        todo_open_fg: aurora_yellow.into(),
        todo_done_fg: aurora_green.into(),
        todo_done_body_fg: "#4c566a".into(),
        property_key_fg: "#4c566a".into(),
        property_value_fg: frost2.into(),
        heading_fg: snow2.into(),
        dim_fg: "#4c566a".into(),
        selected_bullet_bg: frost1.into(),
        selected_bullet_fg: polar0.into(),
        cursor_block_bg: snow1.into(),
        cursor_block_fg: polar0.into(),
        cursor_caret_fg: snow1.into(),
        status_normal_bg: frost1.into(),
        status_normal_fg: polar0.into(),
        status_insert_bg: aurora_green.into(),
        status_insert_fg: polar0.into(),
        status_visual_bg: aurora_purple.into(),
        status_visual_fg: polar0.into(),
        status_message_fg: aurora_red.into(),
        list_selected_bg: frost1.into(),
        list_selected_fg: polar0.into(),
        help_title_fg: aurora_yellow.into(),
    }
}

/// Monokai — Wimer Hazenberg's high-contrast palette.
pub fn monokai() -> Palette {
    let bg = "#272822";
    let fg = "#f8f8f2";
    let comment = "#75715e";
    let pink = "#f92672";
    let orange = "#fd971f";
    let yellow = "#e6db74";
    let green = "#a6e22e";
    let blue = "#66d9ef";
    let purple = "#ae81ff";

    Palette {
        name: "monokai".into(),
        bg: bg.into(),
        bg_elev: "#1e1f1c".into(),
        fg: fg.into(),
        fg_dim: comment.into(),
        fg_dimmer: comment.into(),
        border: comment.into(),
        hint: comment.into(),
        accent: pink.into(),
        accent_soft: purple.into(),
        accent_alt: green.into(),
        warn: yellow.into(),
        ref_link_fg: blue.into(),
        tag_link_fg: pink.into(),
        md_link_fg: blue.into(),
        bold_fg: orange.into(),
        italic_fg: yellow.into(),
        strike_fg: comment.into(),
        code_fg: green.into(),
        todo_open_fg: pink.into(),
        todo_done_fg: green.into(),
        todo_done_body_fg: comment.into(),
        property_key_fg: comment.into(),
        property_value_fg: purple.into(),
        heading_fg: pink.into(),
        dim_fg: comment.into(),
        selected_bullet_bg: pink.into(),
        selected_bullet_fg: bg.into(),
        cursor_block_bg: fg.into(),
        cursor_block_fg: bg.into(),
        cursor_caret_fg: fg.into(),
        status_normal_bg: pink.into(),
        status_normal_fg: bg.into(),
        status_insert_bg: green.into(),
        status_insert_fg: bg.into(),
        status_visual_bg: purple.into(),
        status_visual_fg: bg.into(),
        status_message_fg: yellow.into(),
        list_selected_bg: pink.into(),
        list_selected_fg: bg.into(),
        help_title_fg: yellow.into(),
    }
}
