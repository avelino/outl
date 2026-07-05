//! Wire-format adapter between the Solid frontend and the shared
//! [`outl_config::Config`] file at `~/.config/outl/config.toml`.
//!
//! The frontend continues to see a flat shape (`last_workspace`,
//! `vim_mode`, `theme`, `font_size`) because that's what the
//! `SettingsModal` was built around and there's no value in
//! reshuffling the JSON wire mid-flight. Internally we convert to /
//! from the structured `Config` so the on-disk file stays human-
//! editable and the TUI can read the same source of truth.

use serde::{Deserialize, Serialize};

use outl_config::{Config, EditorCfg, SyncConfig, SyncTransportKind, ThemeCfg, WorkspaceCfg};

/// Parse the flat wire string into a transport kind. Anything that isn't an
/// explicit `"file"` opt-out (including an empty string from an older frontend)
/// resolves to iroh — P2P is the default.
fn parse_transport(s: &str) -> SyncTransportKind {
    match s {
        "file" => SyncTransportKind::File,
        _ => SyncTransportKind::Iroh,
    }
}

/// Render a transport kind to the lowercase wire string the frontend uses.
fn transport_str(t: SyncTransportKind) -> String {
    match t {
        SyncTransportKind::File => "file",
        SyncTransportKind::Iroh => "iroh",
    }
    .to_string()
}

/// Flat shape the Solid frontend's `Settings` interface
/// (`crates/outl-desktop/src/lib/api.ts`) expects. Matches what
/// `SettingsModal.tsx` reads and writes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub last_workspace: Option<std::path::PathBuf>,
    /// Defaults to `true` — outl is keyboard-first and the same
    /// behaviour ships in the TUI.
    pub vim_mode: bool,
    /// Palette preset name from `outl_theme::PRESETS`. Default
    /// `"outl"` (brand purple).
    pub theme: String,
    /// Outline font size in pixels.
    pub font_size: u32,
    /// Sync transport: `"iroh"` (P2P, default) or `"file"` (iCloud /
    /// shared filesystem opt-out). The Sync panel writes this.
    pub sync_transport: String,
}

impl Settings {
    /// Default values used when `config.toml` doesn't exist yet.
    /// Mirrors `Config::default()` field-for-field.
    pub fn fresh() -> Self {
        Config::default().into()
    }
}

impl From<Config> for Settings {
    fn from(c: Config) -> Self {
        Self {
            last_workspace: c.workspace.last,
            vim_mode: c.editor.vim_mode,
            theme: c.theme.preset,
            font_size: c.editor.font_size,
            sync_transport: transport_str(c.sync.transport),
        }
    }
}

impl From<Settings> for Config {
    fn from(s: Settings) -> Self {
        Self {
            workspace: WorkspaceCfg {
                last: s.last_workspace,
            },
            theme: ThemeCfg { preset: s.theme },
            editor: EditorCfg {
                vim_mode: s.vim_mode,
                font_size: s.font_size,
            },
            // The flat desktop Settings doesn't model `[calendar]`; `save`
            // restores it from disk so a hand-set timezone survives a
            // settings write (same pattern as `sync.relay_url`).
            calendar: outl_config::CalendarCfg::default(),
            sync: SyncConfig {
                transport: parse_transport(&s.sync_transport),
                // relay_url isn't modeled in the flat Settings; `save` restores
                // the on-disk value so editing the transport doesn't drop it.
                relay_url: None,
            },
            // `[tui]` is TUI-only; the desktop doesn't model it. `save`
            // restores it from disk so a hand-set `mouse_capture` survives
            // a settings write (same pattern as `[calendar]`).
            tui: outl_config::TuiCfg::default(),
            // `[snapshot]` is core-managed; the desktop doesn't model it.
            // `save` restores it from disk so a hand-set policy survives a
            // settings write (same pattern as `[calendar]` / `[tui]`).
            snapshot: outl_config::SnapshotCfg::default(),
        }
    }
}

/// Load `config.toml` from `~/.config/outl/` and project to the
/// flat wire shape. Missing / malformed file = defaults — the
/// `outl-config` crate already logs the parse error.
///
/// The `_app_config_dir` parameter is kept for the AppState
/// signature (other modules read it for the actor file location)
/// but the config itself ignores it; the path is XDG-driven.
pub fn load(_app_config_dir: &std::path::Path) -> Settings {
    outl_config::load().into()
}

/// Save the flat wire shape as `config.toml`. Same path
/// (`~/.config/outl/config.toml`) regardless of where the OS
/// thinks the app's config directory is.
pub fn save(_app_config_dir: &std::path::Path, settings: &Settings) -> anyhow::Result<()> {
    let mut cfg: Config = settings.clone().into();
    // The flat `Settings` carries the transport choice (the Sync panel
    // writes it), so `into()` already set `cfg.sync.transport`. It does NOT
    // model `relay_url` or `[calendar]`, so restore those from disk in one
    // read — otherwise saving the transport would wipe a custom relay or a
    // hand-set timezone (and two reads could mix fields across a concurrent
    // edit).
    let on_disk = outl_config::load();
    cfg.sync.relay_url = on_disk.sync.relay_url;
    cfg.calendar = on_disk.calendar;
    outl_config::save(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fresh_matches_config_defaults() {
        let s = Settings::fresh();
        assert!(s.last_workspace.is_none());
        assert!(
            s.vim_mode,
            "vim mode is on by default — outl is keyboard-first"
        );
        assert_eq!(s.theme, "outl");
        assert_eq!(s.font_size, 15);
        assert_eq!(s.sync_transport, "iroh", "P2P is the default transport");
    }

    #[test]
    fn round_trips_via_config() {
        let s = Settings {
            last_workspace: Some(PathBuf::from("/tmp/ws")),
            vim_mode: false,
            theme: "dracula".into(),
            font_size: 18,
            sync_transport: "file".into(),
        };
        let cfg: Config = s.clone().into();
        let back: Settings = cfg.into();
        assert_eq!(back.last_workspace, s.last_workspace);
        assert_eq!(back.vim_mode, s.vim_mode);
        assert_eq!(back.theme, s.theme);
        assert_eq!(back.font_size, s.font_size);
        assert_eq!(back.sync_transport, s.sync_transport);
    }
}
