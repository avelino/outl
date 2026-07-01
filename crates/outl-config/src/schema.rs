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
    pub sync: SyncConfig,
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

/// Calendar / time section — controls how outl renders "now" and
/// "today".
///
/// `timezone` is an optional IANA name (`"Europe/London"`,
/// `"America/Sao_Paulo"`). When unset, outl uses the operating
/// system's local timezone — the right default on a normally
/// configured machine. Set it explicitly when the OS clock lies about
/// the zone: containers and Chrome OS **Crostini** run in UTC even
/// though the user's real timezone isn't, which pushes the journal
/// date and the status-line clock an hour (or more) off. See issue
/// #107.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarCfg {
    /// IANA timezone name, e.g. `"Europe/London"`. `None` (the
    /// default) means "use the OS local timezone". An unknown or
    /// unparseable name is ignored when the clock initializes (logged)
    /// and also falls back to local.
    pub timezone: Option<String>,
}

/// Which sync transport a client wires up at boot.
///
/// `lowercase` serde so the TOML reads `transport = "file"` /
/// `transport = "iroh"` — matching how a user thinks of them, not
/// the Rust variant casing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncTransportKind {
    /// File-based transport (iCloud Drive / shared filesystem). The opt-out
    /// from iroh: set `transport = "file"`. Still fully supported.
    File,
    /// iroh P2P transport (QUIC + hole punching). The default — P2P is
    /// outl's primary sync. Override with `transport = "file"`.
    #[default]
    Iroh,
}

/// Sync section. Controls which transport moves the per-actor op log
/// between devices. Missing `[sync]` falls back to [`SyncTransportKind::Iroh`]
/// (P2P is outl's primary sync); `transport = "file"` is the explicit opt-out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Transport to use. Defaults to [`SyncTransportKind::Iroh`].
    pub transport: SyncTransportKind,

    /// Optional relay URL for the `iroh` transport. `None` (or an
    /// empty string in the TOML, normalized to `None` on read) means
    /// use iroh's n0 default relays. Ignored by the `file` transport.
    pub relay_url: Option<String>,
}

impl SyncConfig {
    /// The configured relay URL, with empty strings treated as
    /// "unset". A user who writes `relay_url = ""` in TOML to mean
    /// "use the defaults" gets `None`, same as omitting the key.
    pub fn relay_url(&self) -> Option<&str> {
        self.relay_url.as_deref().filter(|s| !s.is_empty())
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
        assert!(c.calendar.timezone.is_none());
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert!(c.sync.relay_url.is_none());
    }

    #[test]
    fn calendar_section_parses_timezone() {
        let c: Config = toml::from_str("[calendar]\ntimezone = \"Europe/London\"\n").unwrap();
        assert_eq!(c.calendar.timezone.as_deref(), Some("Europe/London"));
    }

    #[test]
    fn missing_calendar_section_leaves_timezone_unset() {
        // No [calendar] → timezone None → clock uses OS local (previous behaviour).
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert!(c.calendar.timezone.is_none());
    }

    #[test]
    fn empty_config_defaults_to_iroh_transport() {
        // P2P is outl's primary sync, so a missing [sync] section defaults to
        // iroh. `transport = "file"` is the explicit opt-out.
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_section_parses_file_transport() {
        let c: Config = toml::from_str("[sync]\ntransport = \"file\"\n").unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::File);
    }

    #[test]
    fn sync_section_parses_iroh_transport() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.transport, SyncTransportKind::Iroh);
        // No relay set → falls back to defaults (None).
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_empty_relay_url_normalizes_to_none() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = ""
"#,
        )
        .unwrap();
        assert!(c.sync.relay_url().is_none());
    }

    #[test]
    fn sync_relay_url_is_returned_when_set() {
        let c: Config = toml::from_str(
            r#"
[sync]
transport = "iroh"
relay_url = "https://relay.example"
"#,
        )
        .unwrap();
        assert_eq!(c.sync.relay_url(), Some("https://relay.example"));
    }
}
