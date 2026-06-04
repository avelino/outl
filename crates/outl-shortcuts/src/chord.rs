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
}
