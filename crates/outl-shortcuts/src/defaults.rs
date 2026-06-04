//! Default binding catalog.
//!
//! Hand-curated table seeded from `outl-tui/src/input/` (the
//! shipping shortcuts) plus the OS-standard chrome bindings the
//! desktop already relied on (`Cmd/Ctrl+P`, `Cmd/Ctrl+B`, etc.).
//!
//! Adding a binding here lights it up on both clients
//! simultaneously — the TUI's chord adapter and the desktop's
//! `KeyboardEvent` adapter both go through the same lookup.

use crate::action::Action;
use crate::binding::{Binding, Mode};
use crate::chord::{Chord, ChordSequence, Key, Modifiers};

fn ctrl(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ctrl(c))
}
fn meta(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::meta(c))
}
fn ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::ch(c))
}
fn key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::plain(k))
}
fn shift(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, k))
}
fn shift_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::SHIFT, Key::char(c)))
}
fn shift_meta_ch(c: char) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META | Modifiers::SHIFT, Key::char(c)))
}
fn meta_key(k: Key) -> ChordSequence {
    ChordSequence::chord(Chord::new(Modifiers::META, k))
}
fn pair(a: char, b: char) -> ChordSequence {
    ChordSequence::pair(Chord::ch(a), Chord::ch(b))
}

/// Every binding outl ships with by default.
///
/// `Global` entries fire in every mode — those are the OS-standard
/// chrome chords (`Cmd+P`, `Cmd+T`, `Cmd+,`, `Cmd+[`, `Cmd+]`,
/// `Cmd+Shift+E`, `Cmd+Shift+B`). The TUI adapter doesn't emit
/// `META` from crossterm, so on a terminal these manifest as their
/// TUI-native variants (single letters, brackets, …).
///
/// Two chords were deliberately freed up vs. the first cut:
///
/// - `Cmd+B` is **reserved for `WrapBold`** in Insert mode, matching
///   every markdown editor on the planet (Notion, Obsidian, Discord,
///   Slack, Typora). It would be hostile to retrain users on a
///   non-standard meaning.
/// - `Cmd+\` is **1Password's** global autofill shortcut on macOS.
///   Hijacking it breaks every user who has 1Password installed.
///
/// Sidebar / backlinks panel toggles ride `Cmd+Shift+E` (mirrors
/// VS Code's "explorer" pane) and `Cmd+Shift+B`.
pub fn default_bindings() -> Vec<Binding> {
    use Mode::*;

    vec![
        // ── Global chrome (OS-standard, work in every mode) ───────
        Binding::new(meta('p'), Global, Action::OpenPicker, "Open quick switcher"),
        Binding::new(ctrl('p'), Global, Action::OpenPicker, "Open quick switcher"),
        Binding::new(meta('t'), Global, Action::OpenToday, "Open today's journal"),
        Binding::new(
            shift_meta_ch('e'),
            Global,
            Action::ToggleSidebar,
            "Toggle sidebar",
        ),
        Binding::new(
            shift_meta_ch('b'),
            Global,
            Action::ToggleBacklinks,
            "Toggle backlinks panel",
        ),
        Binding::new(meta(','), Global, Action::OpenSettings, "Open settings"),
        Binding::new(meta('['), Global, Action::PrevDay, "Previous journal day"),
        Binding::new(meta(']'), Global, Action::NextDay, "Next journal day"),
        Binding::new(ctrl('c'), Global, Action::Quit, "Quit"),
        // ── Inline markdown wrappers (Insert mode — textarea focused) ──
        //
        // Mirrors the convention every popular markdown editor
        // (Notion, Obsidian, Discord, Slack, Typora) ships.
        // Cmd+B/I/E/K for bold / italic / code / link; Cmd+Shift+X
        // for strikethrough (Slack/Discord convention, avoids the
        // Cmd+Shift+S "Save As" conflict).
        Binding::new(meta('b'), Insert, Action::WrapBold, "Bold (**…**)"),
        // outl ships `_…_` as the canonical italic — the parser
        // accepts `*…*` too but `.md` projections emit underscores.
        Binding::new(meta('i'), Insert, Action::WrapItalic, "Italic (_…_)"),
        Binding::new(meta('e'), Insert, Action::WrapCode, "Inline code (`…`)"),
        Binding::new(
            meta('k'),
            Insert,
            Action::InsertLink,
            "Insert link ([label](url))",
        ),
        Binding::new(
            shift_meta_ch('x'),
            Insert,
            Action::WrapStrike,
            "Strikethrough (~~…~~)",
        ),
        // ── Normal mode (vim-style, TUI parity) ───────────────────
        Binding::new(ch('t'), Normal, Action::OpenToday, "Open today's journal"),
        Binding::new(
            key(Key::Home),
            Normal,
            Action::OpenToday,
            "Open today (Home)",
        ),
        Binding::new(ch('['), Normal, Action::PrevDay, "Previous journal day"),
        Binding::new(ch(']'), Normal, Action::NextDay, "Next journal day"),
        Binding::new(
            pair('g', 'j'),
            Normal,
            Action::OpenToday,
            "Jump to today (chord)",
        ),
        Binding::new(
            ctrl('p'),
            Normal,
            Action::OpenPicker,
            "Quick switcher (fuzzy)",
        ),
        Binding::new(ch('?'), Normal, Action::ToggleHelp, "Toggle help popup"),
        Binding::new(
            ch(':'),
            Normal,
            Action::OpenCommandPalette,
            "Command palette",
        ),
        Binding::new(pair('q', 'q'), Normal, Action::Quit, "Quit (chord)"),
        Binding::new(ch('j'), Normal, Action::SelectionDown, "Selection down"),
        Binding::new(
            key(Key::Down),
            Normal,
            Action::SelectionDown,
            "Selection down",
        ),
        Binding::new(ch('k'), Normal, Action::SelectionUp, "Selection up"),
        Binding::new(key(Key::Up), Normal, Action::SelectionUp, "Selection up"),
        Binding::new(
            ch('i'),
            Normal,
            Action::EnterInsert,
            "Insert at end of block",
        ),
        Binding::new(
            shift_ch('i'),
            Normal,
            Action::EnterInsertAtStart,
            "Insert at start",
        ),
        Binding::new(
            key(Key::Enter),
            Normal,
            Action::OpenRefUnderCursor,
            "Open ref / enter Insert",
        ),
        Binding::new(ch('o'), Normal, Action::NewBlockBelow, "New block below"),
        Binding::new(
            shift_ch('o'),
            Normal,
            Action::NewBlockAbove,
            "New block above",
        ),
        Binding::new(key(Key::Tab), Normal, Action::IndentBlock, "Indent block"),
        Binding::new(
            shift(Key::Tab),
            Normal,
            Action::OutdentBlock,
            "Outdent block",
        ),
        Binding::new(
            pair('d', 'd'),
            Normal,
            Action::DeleteBlock,
            "Delete block (chord)",
        ),
        Binding::new(ch('c'), Normal, Action::ToggleCollapsed, "Fold / unfold"),
        Binding::new(
            pair('y', 'r'),
            Normal,
            Action::CopyBlockRef,
            "Copy block ref handle",
        ),
        Binding::new(ch('v'), Normal, Action::EnterVisual, "Enter Visual mode"),
        Binding::new(ch('u'), Normal, Action::Undo, "Undo"),
        Binding::new(ctrl('r'), Normal, Action::Redo, "Redo"),
        // ── Insert mode ───────────────────────────────────────────
        //
        // Only `Esc` and `Cmd+Enter` land in the catalog. Every
        // other in-editor chord (`Tab`/`Shift-Tab` for indent,
        // `Backspace` on an empty block, `[[`/`((` auto-pair,
        // plain `Enter` for a newline) is owned by `<BlockRow />`'s
        // textarea `onKeyDown`. If we bound them here, the
        // dispatcher would `preventDefault` first and the textarea
        // would never see the keystroke.
        Binding::new(
            key(Key::Esc),
            Insert,
            Action::ExitInsert,
            "Commit + exit Insert",
        ),
        Binding::new(
            meta_key(Key::Enter),
            Insert,
            Action::CommitAndContinue,
            "Commit + new block below",
        ),
        // ── Visual mode ──────────────────────────────────────────
        Binding::new(key(Key::Esc), Visual, Action::ExitInsert, "Leave Visual"),
        Binding::new(ch('y'), Visual, Action::YankRange, "Yank range"),
        Binding::new(ch('d'), Visual, Action::DeleteRange, "Delete range"),
        Binding::new(ch('j'), Visual, Action::SelectionDown, "Extend down"),
        Binding::new(ch('k'), Visual, Action::SelectionUp, "Extend up"),
        // ── Overlay (picker / palette have their own keys; arrow + Enter + Esc) ──
        Binding::new(key(Key::Esc), Overlay, Action::ExitInsert, "Close overlay"),
        Binding::new(
            key(Key::Down),
            Overlay,
            Action::SelectionDown,
            "Highlight next",
        ),
        Binding::new(
            key(Key::Up),
            Overlay,
            Action::SelectionUp,
            "Highlight previous",
        ),
    ]
}
