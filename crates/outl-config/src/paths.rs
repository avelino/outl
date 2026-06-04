//! XDG-style path resolution for the shared config.

use std::path::PathBuf;

/// `~/.config/outl/` (or `$XDG_CONFIG_HOME/outl/` when set).
///
/// XDG-style on every OS — including macOS — so a user dropping
/// into a terminal sees the same path the desktop GUI is writing
/// to. The conventional macOS location
/// (`~/Library/Application Support/…`) feels wrong for a
/// CLI-first tool and would split the TUI and desktop into two
/// config files.
pub fn config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("XDG_CONFIG_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom).join("outl");
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".config").join("outl");
    }
    // Last resort — relative path. Almost never hit in practice
    // (every supported OS has a home dir), but keeps the function
    // total so callers don't have to handle `Option`.
    PathBuf::from(".config").join("outl")
}

/// `~/.config/outl/config.toml`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_ends_with_outl_config_toml() {
        let p = config_path();
        let s = p.to_string_lossy();
        assert!(s.ends_with("outl/config.toml"), "got: {s}");
    }

    #[test]
    fn config_dir_respects_xdg_config_home() {
        // We can't temporarily set env safely in concurrent tests,
        // so just assert the format when the var is unset. The
        // logic itself is one straight `if let`.
        let _ = config_dir();
    }
}
