//! Theme commands — surface the shared `outl-theme` presets to the
//! Solid frontend.
//!
//! The frontend takes the returned `Palette` and writes each field
//! as a CSS custom property (`--color-outl-accent`, …) on `<html>`.
//! Tailwind utilities then reference those tokens via
//! `text-(--color-outl-accent)`. Switching a theme is just a swap
//! of the var values — no reload, no remount.

use outl_theme::Palette;

/// List of every built-in palette name, in user-facing order
/// (matches `outl_theme::PRESETS`).
#[tauri::command]
pub(crate) fn list_themes() -> Vec<String> {
    outl_theme::PRESETS.iter().map(|s| s.to_string()).collect()
}

/// Resolve a palette by name, falling back to the default when the
/// caller passes an unknown / empty string. The default mirrors
/// `outl_theme::default()` so the desktop's first-launch experience
/// matches the TUI's `theme = "outl"` default.
#[tauri::command]
pub(crate) fn get_theme(name: Option<String>) -> Palette {
    name.as_deref()
        .and_then(outl_theme::by_name)
        .unwrap_or_else(outl_theme::default)
}
