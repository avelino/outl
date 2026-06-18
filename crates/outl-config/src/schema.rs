//! Top-level [`Config`] struct + its sub-sections.
//!
//! Adding a field anywhere is a one-line change in both the struct
//! and (if surfaced to the desktop wire format) the Tauri command
//! shim. `#[serde(default)]` everywhere means a missing field falls
//! back to the type's [`Default`], so an old config file keeps
//! working after the schema grows.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Root config — three sections that map cleanly to "which client
/// cares".
///
/// - [`WorkspaceCfg`] — read by the desktop (last opened path) and
///   the TUI (when no `--path` flag is passed).
/// - [`ThemeCfg`] — read by every renderer (TUI, desktop) for which
///   `outl_theme::Palette` to render with.
/// - [`EditorCfg`] — local editing preferences, mostly desktop
///   today (the TUI is vim-style by definition).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub workspace: WorkspaceCfg,
    pub theme: ThemeCfg,
    pub editor: EditorCfg,
    pub calendar: CalendarCfg,
}

/// Workspace section — primarily where the desktop remembers the
/// last opened directory so the next launch skips the picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceCfg {
    /// Absolute path to the last workspace the user opened. The
    /// desktop writes this on every `set_workspace` call; the TUI
    /// can read it as a fallback when no `--path` flag was given.
    pub last: Option<PathBuf>,
}

/// Theme section. The `preset` name matches one of
/// `outl_theme::PRESETS` (`"outl"`, `"dracula"`, …); unknown names
/// fall back to `outl_theme::default()` at render time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeCfg {
    pub preset: String,
}

impl Default for ThemeCfg {
    fn default() -> Self {
        Self {
            preset: "outl".to_string(),
        }
    }
}

/// Editor preferences. `vim_mode` defaults to `true` because
/// outl is keyboard-first — the same default the TUI ships with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorCfg {
    /// Vim-style modal bindings (Normal / Insert / Visual).
    /// When `false`, the desktop falls back to plain text-editing
    /// chrome (no modes; OS-standard chords only). The TUI is
    /// vim-style by definition and ignores this flag.
    pub vim_mode: bool,

    /// Base font size for the outline view (pixels). The TUI
    /// doesn't read this; terminal font is the user's terminal
    /// setting.
    pub font_size: u32,
}

impl Default for EditorCfg {
    fn default() -> Self {
        Self {
            vim_mode: true,
            font_size: 15,
        }
    }
}

/// Calendar section — how the mini-calendar / week views lay out.
/// Read by every client that renders a calendar (TUI sidebar, desktop
/// sidebar; the mobile calendar has no `~/.config` to read on iOS and
/// keeps its own default until it grows an in-app setting).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarCfg {
    /// Which day the week grid starts on.
    pub week_start: WeekStart,
}

/// First column of a calendar week grid.
///
/// Defaults to [`WeekStart::Monday`] — the historical TUI / desktop
/// behaviour — so an existing config keeps rendering identically.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    /// ISO week: Monday first (`Mon … Sun`).
    #[default]
    Monday,
    /// US-style week: Sunday first (`Sun … Sat`).
    Sunday,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_values() {
        let c = Config::default();
        assert_eq!(c.theme.preset, "outl");
        assert!(c.editor.vim_mode);
        assert_eq!(c.editor.font_size, 15);
        assert!(c.workspace.last.is_none());
        assert_eq!(c.calendar.week_start, WeekStart::Monday);
    }

    #[test]
    fn week_start_parses_and_defaults_per_section() {
        // Explicit value round-trips through the lowercase serde repr.
        let c: Config = toml::from_str("[calendar]\nweek_start = \"sunday\"\n").unwrap();
        assert_eq!(c.calendar.week_start, WeekStart::Sunday);

        // A config that omits `[calendar]` entirely still falls back to
        // the Monday default (the `#[serde(default)]` partial-section path).
        let c: Config = toml::from_str("[theme]\npreset = \"dracula\"\n").unwrap();
        assert_eq!(c.calendar.week_start, WeekStart::Monday);
    }
}
