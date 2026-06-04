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
    }
}
