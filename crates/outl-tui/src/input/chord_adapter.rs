//! `crossterm::KeyEvent` → [`outl_shortcuts::Chord`] adapter.
//!
//! The TUI handles `KeyEvent`s directly in `input/normal.rs` rather
//! than going through `outl_shortcuts::lookup`; this module is the one
//! place that bridges a raw crossterm key into the portable [`Chord`]
//! type so a plugin's `contributes.keybindings` (parsed by
//! `outl-plugins` into [`outl_shortcuts::ChordSequence`]) can be
//! compared against a live keystroke.
//!
//! Scope is deliberately narrow: plugin keybindings are `Mode::Global`
//! and the TUI only dispatches them from Normal mode (see
//! `input/normal.rs::try_plugin_binding`), so we only need to map the
//! keys a Normal-mode user can press. Keys that don't have a
//! [`Chord`] equivalent (e.g. modifier-only presses) return `None` and
//! never match a plugin binding.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use outl_shortcuts::{Chord, Key, Modifiers};

/// Translate a crossterm key into the portable [`Chord`] form.
///
/// Returns `None` for keys outl-shortcuts has no variant for (so they
/// simply never match a plugin binding rather than mis-firing one).
pub(super) fn chord_from_key(key: KeyEvent) -> Option<Chord> {
    let mods = mods_from_crossterm(key.modifiers);
    let k = match key.code {
        // `Key::char` lowercases, matching how `Chord::parse` (and
        // therefore the plugin manifest) normalises the key — so a
        // plugin's `"Ctrl+T"` matches whether the terminal reports
        // `t` or `T`. Shift is still carried in `mods` for chords
        // like `Ctrl+Shift+A` that the plugin spelled explicitly.
        KeyCode::Char(c) => Key::char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F(n) => Key::Function(n),
        _ => return None,
    };
    Some(Chord::new(mods, k))
}

fn mods_from_crossterm(m: KeyModifiers) -> Modifiers {
    let mut out = Modifiers::NONE;
    if m.contains(KeyModifiers::CONTROL) {
        out = out | Modifiers::CTRL;
    }
    if m.contains(KeyModifiers::ALT) {
        out = out | Modifiers::ALT;
    }
    if m.contains(KeyModifiers::SHIFT) {
        out = out | Modifiers::SHIFT;
    }
    if m.contains(KeyModifiers::SUPER) || m.contains(KeyModifiers::META) {
        out = out | Modifiers::META;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_char_lowercased() {
        let c = chord_from_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE)).unwrap();
        assert_eq!(c, Chord::new(Modifiers::NONE, Key::Char('t')));
    }

    #[test]
    fn ctrl_modifier_carried() {
        let c = chord_from_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(c, Chord::new(Modifiers::CTRL, Key::Char('d')));
    }

    #[test]
    fn named_key_maps() {
        let c = chord_from_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)).unwrap();
        assert_eq!(c, Chord::new(Modifiers::CTRL, Key::Enter));
    }

    #[test]
    fn unmappable_key_is_none() {
        // A bare modifier press (no concrete key) has no Chord form.
        assert!(chord_from_key(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)).is_none());
    }

    #[test]
    fn matches_a_plugin_parsed_chord() {
        // The plugin manifest's `"Ctrl+T"` parses to this; a live
        // Ctrl+T keystroke must produce the same Chord so the
        // dispatcher's `==` fires.
        let parsed = outl_shortcuts::ChordSequence::parse("Ctrl+T").unwrap();
        let live =
            chord_from_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(parsed.0[0], live);
    }
}
