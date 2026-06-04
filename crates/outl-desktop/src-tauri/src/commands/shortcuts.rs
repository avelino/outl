//! Shortcuts command — return the shared binding catalog so the
//! Solid frontend can wire a single `keydown` listener that maps
//! the user's chord → `Action` → handler.
//!
//! Filtering by mode is a frontend concern: the desktop is a
//! mode-aware editor (Normal/Insert/Visual/Overlay) and the active
//! mode shifts as the user clicks into a textarea, opens the
//! picker, etc. The backend ships every binding; the frontend
//! `lookup(mode, chord)` picks the right one per keystroke.

use outl_shortcuts::Binding;

/// Return every default binding shipped by `outl-shortcuts`. The
/// frontend caches the result on first call and uses it for the
/// rest of the session — bindings never change at runtime today,
/// so a refresh is only needed when the user edits their config
/// (Phase X feature).
#[tauri::command]
pub(crate) fn list_shortcut_bindings() -> Vec<Binding> {
    outl_shortcuts::default_bindings()
}
