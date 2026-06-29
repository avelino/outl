//! Portable keyboard chord types.
//!
//! Independent of `crossterm`, `winit`, or `KeyboardEvent` so the
//! crate stays usable from both Rust (TUI) and JS (desktop, via a
//! serde-derived JSON wire format). Each client implements its own
//! adapter (`KeyEvent -> Chord`) and consumes [`crate::lookup`] /
//! [`crate::bindings_for_mode`] unchanged.

use serde::{Deserialize, Serialize};

/// Bitflag-style modifier set. Re-implemented as a plain `u8` so
/// no extra dep (no `bitflags`) lands in the crate.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(0b0001);
    pub const ALT: Self = Self(0b0010);
    pub const SHIFT: Self = Self(0b0100);
    /// `Cmd` on macOS / `Meta`/`Win` elsewhere. Desktop maps the
    /// platform key here; TUI usually never sees it (terminals
    /// rarely surface meta) but the variant exists for
    /// completeness.
    pub const META: Self = Self(0b1000);

    pub fn contains(self, m: Modifiers) -> bool {
        (self.0 & m.0) == m.0 && m.0 != 0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn ctrl() -> Self {
        Self::CTRL
    }
    pub fn meta() -> Self {
        Self::META
    }
    pub fn shift() -> Self {
        Self::SHIFT
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Key value, normalised across input layers.
///
/// `Char` carries the (lowercased) Unicode codepoint produced by
/// the key. Special / dedicated keys get named variants so a
/// binding doesn't have to live as `Char('\n')` — the adapter on
/// each client produces the right variant.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    /// Function keys F1-F12.
    Function(u8),
}

impl Key {
    pub fn char(c: char) -> Self {
        Self::Char(c.to_ascii_lowercase())
    }
}

/// A single chord — one keypress with optional modifiers.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Chord {
    pub mods: Modifiers,
    pub key: Key,
}

impl Chord {
    pub const fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }
    pub fn plain(key: Key) -> Self {
        Self::new(Modifiers::NONE, key)
    }
    pub fn ch(c: char) -> Self {
        Self::plain(Key::char(c))
    }
    pub fn ctrl(c: char) -> Self {
        Self::new(Modifiers::CTRL, Key::char(c))
    }
    pub fn meta(c: char) -> Self {
        Self::new(Modifiers::META, Key::char(c))
    }
    pub fn shift_meta(c: char) -> Self {
        Self::new(Modifiers::META | Modifiers::SHIFT, Key::char(c))
    }
}

/// A sequence of one or two chords. Single-chord bindings live as
/// `ChordSequence(vec![chord])`; vim-style prefixes like `g j`
/// live as two-element sequences. A future user-config that wants
/// `<leader> p` is just a longer vec — no API change.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChordSequence(pub Vec<Chord>);

impl ChordSequence {
    pub fn single(c: Chord) -> Self {
        Self(vec![c])
    }
    pub fn chord(c: Chord) -> Self {
        Self::single(c)
    }
    pub fn pair(a: Chord, b: Chord) -> Self {
        Self(vec![a, b])
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// `true` when `self` is a strict prefix of `other`. Used by
    /// callers buffering a chord while waiting on the second
    /// keypress.
    pub fn is_prefix_of(&self, other: &Self) -> bool {
        self.len() < other.len() && self.0[..] == other.0[..self.len()]
    }

    /// Parse a human chord string like `"Ctrl+Shift+A"` or `"Ctrl+T S"`
    /// (space-separated chords) into a sequence. Used to turn a plugin's
    /// `contributes.keybindings[].key` into a real [`ChordSequence`] without
    /// each client reimplementing the parse. Returns `None` on any malformed
    /// part, or a sequence longer than two chords.
    pub fn parse(s: &str) -> Option<Self> {
        let chords: Option<Vec<Chord>> = s.split_whitespace().map(Chord::parse).collect();
        let chords = chords?;
        if chords.is_empty() || chords.len() > 2 {
            return None;
        }
        Some(Self(chords))
    }
}

impl Chord {
    /// Parse a single chord like `"Ctrl+Shift+A"`: `+`-separated, the last part
    /// is the key, the rest are modifiers (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        let (key_str, mod_strs) = parts.split_last()?;
        let mut mods = Modifiers::NONE;
        for m in mod_strs {
            mods = mods
                | match m.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => Modifiers::CTRL,
                    "alt" | "option" | "opt" => Modifiers::ALT,
                    "shift" => Modifiers::SHIFT,
                    "meta" | "cmd" | "command" | "win" | "super" => Modifiers::META,
                    _ => return None,
                };
        }
        Some(Self {
            mods,
            key: parse_key(key_str)?,
        })
    }
}

/// Parse a key name (`"Enter"`, `"Space"`, `"F5"`) or a single character.
fn parse_key(s: &str) -> Option<Key> {
    match s.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(Key::Enter),
        "esc" | "escape" => Some(Key::Esc),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "space" => Some(Key::Space),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "pgup" => Some(Key::PageUp),
        "pagedown" | "pgdn" => Some(Key::PageDown),
        other => {
            if let Some(n) = other.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()) {
                if (1..=12).contains(&n) {
                    return Some(Key::Function(n));
                }
            }
            let mut chars = other.chars();
            let c = chars.next()?;
            chars.next().is_none().then(|| Key::char(c))
        }
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn single_chord_with_mods() {
        let seq = ChordSequence::parse("Ctrl+Shift+A").unwrap();
        assert_eq!(
            seq.0,
            vec![Chord {
                mods: Modifiers::CTRL | Modifiers::SHIFT,
                key: Key::Char('a')
            }]
        );
    }

    #[test]
    fn two_chord_sequence() {
        let seq = ChordSequence::parse("Ctrl+T S").unwrap();
        assert_eq!(seq.len(), 2);
        assert_eq!(
            seq.0[0],
            Chord {
                mods: Modifiers::CTRL,
                key: Key::Char('t')
            }
        );
        assert_eq!(
            seq.0[1],
            Chord {
                mods: Modifiers::NONE,
                key: Key::Char('s')
            }
        );
    }

    #[test]
    fn named_keys_and_function() {
        assert_eq!(Chord::parse("Cmd+Enter").unwrap().key, Key::Enter);
        assert_eq!(Chord::parse("F5").unwrap().key, Key::Function(5));
        assert_eq!(Chord::parse("Space").unwrap().key, Key::Space);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Chord::parse("Ctrl+").is_none());
        assert!(Chord::parse("Bogus+A").is_none());
        assert!(ChordSequence::parse("a b c").is_none()); // > 2 chords
        assert!(ChordSequence::parse("").is_none());
    }
}
