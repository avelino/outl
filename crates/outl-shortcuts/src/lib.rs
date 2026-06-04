//! # outl-shortcuts
//!
//! Single source of truth for outl's keyboard bindings, consumed by
//! every client that paints an editor:
//!
//! - **`outl-tui`** translates `crossterm::event::KeyEvent` to
//!   [`Chord`] and looks up the binding for its current [`Mode`].
//! - **`outl-desktop`** exposes [`default_bindings`] over a Tauri
//!   command; the Solid frontend translates `KeyboardEvent` to
//!   [`Chord`], looks up the binding, and dispatches to the JS
//!   handler that maps each [`Action`] to a Tauri call.
//!
//! Both clients see the same `(chord → action)` mapping, so a key
//! the user knows from the TUI works identically in the desktop.
//! A user-level override (TOML / settings) plugs into the same
//! pipeline — same `Binding`, different source list.
//!
//! ## What this crate owns
//!
//! - The [`Action`] enum — every named operation outl performs in
//!   response to a key. Adding an entry here is a coordinated
//!   commit: clients have to add a handler for it, but the *name*
//!   stays one place.
//! - The [`Chord`] / [`ChordSequence`] types — modifiers + key
//!   prefix, expressed independently of any input library so both
//!   crossterm (TUI) and the browser DOM (desktop) can map into
//!   them.
//! - The [`Mode`] enum — which modal state a binding applies to.
//!   `Global` matches everywhere; `Normal` / `Insert` / `Visual` /
//!   `Overlay` are the TUI's vim modes (desktop subscribes only
//!   while `settings.vim_mode == true`).
//! - The default binding table ([`default_bindings`]) and helpers
//!   to query it by mode or action.
//!
//! ## What this crate does NOT own
//!
//! - Handlers. Each client maps `Action -> {do_something}` itself.
//!   This crate doesn't know how the TUI commits an insert buffer
//!   or how the desktop calls `paste_markdown_at`.
//! - Input adapters. `crossterm::KeyEvent -> Chord` lives in
//!   `outl-tui`; `KeyboardEvent -> Chord` lives in the desktop's
//!   `lib/shortcuts.ts`. Both produce a [`Chord`] this crate then
//!   resolves.

mod action;
mod binding;
mod chord;
mod defaults;

pub use action::Action;
pub use binding::{Binding, Mode};
pub use chord::{Chord, ChordSequence, Key, Modifiers};
pub use defaults::default_bindings;

/// Return every default binding active in `mode`, in the order
/// they appear in [`default_bindings`]. Bindings tagged
/// [`Mode::Global`] are included unconditionally — chrome shortcuts
/// like `Cmd+P` work everywhere.
pub fn bindings_for_mode(mode: Mode) -> Vec<Binding> {
    default_bindings()
        .into_iter()
        .filter(|b| b.mode == Mode::Global || b.mode == mode)
        .collect()
}

/// Look up the action triggered by `seq` when the editor is in
/// `mode`. Returns `None` when nothing matches — the client falls
/// back to whatever its default key handling does (typing into a
/// textarea, …).
///
/// Prefix matching is the caller's job: when a [`ChordSequence`]
/// has two chords (e.g. `g j`), the caller buffers the first chord
/// and re-queries with the full sequence on the second key.
pub fn lookup(mode: Mode, seq: &ChordSequence) -> Option<Action> {
    default_bindings()
        .into_iter()
        .find(|b| (b.mode == Mode::Global || b.mode == mode) && &b.chord == seq)
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_has_a_description() {
        for b in default_bindings() {
            assert!(
                !b.description.is_empty(),
                "binding for {:?} ({:?}) has empty description",
                b.action,
                b.chord
            );
        }
    }

    #[test]
    fn no_duplicate_chord_in_same_mode() {
        // Two bindings sharing the same `(mode, chord)` would make
        // `lookup` non-deterministic — guard against it at the
        // catalog level so a typo is caught by `cargo test`.
        let bindings = default_bindings();
        for (i, a) in bindings.iter().enumerate() {
            for b in &bindings[i + 1..] {
                if a.mode == b.mode && a.chord == b.chord {
                    panic!(
                        "duplicate binding in mode {:?}: {:?} maps to both {:?} and {:?}",
                        a.mode, a.chord, a.action, b.action
                    );
                }
            }
        }
    }

    #[test]
    fn bindings_round_trip_via_serde() {
        let bs = default_bindings();
        let json = serde_json::to_string(&bs).unwrap();
        let back: Vec<Binding> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), bs.len());
    }

    #[test]
    fn global_chrome_shortcuts_resolve_in_every_mode() {
        // `Cmd+P` is the canonical chrome shortcut — must work in
        // Normal, Insert, Visual, Overlay. We test against any
        // chord wired to `OpenPicker` in `Global` mode.
        let modes = [Mode::Normal, Mode::Insert, Mode::Visual, Mode::Overlay];
        let picker_bindings: Vec<_> = default_bindings()
            .into_iter()
            .filter(|b| b.action == Action::OpenPicker && b.mode == Mode::Global)
            .collect();
        assert!(
            !picker_bindings.is_empty(),
            "no Global binding for OpenPicker"
        );
        let chord = &picker_bindings[0].chord;
        for mode in modes {
            assert_eq!(
                lookup(mode, chord),
                Some(Action::OpenPicker),
                "OpenPicker not reachable in {mode:?}",
            );
        }
    }
}
