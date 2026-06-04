//! Every named editor operation outl can perform in response to a
//! key chord. Adding a variant is a coordinated change: each client
//! must learn to dispatch it (TUI: a method on `App`; desktop: a
//! JS handler that calls the right Tauri command).
//!
//! Variants are grouped by intent — chrome, navigation, editing,
//! visual / range, code execution — so the help overlay can list
//! them in sensible sections.

use serde::{Deserialize, Serialize};

/// Tagged union of every action the binding catalog can resolve to.
///
/// `#[serde(tag = "kind")]` keeps the wire format readable
/// (`{"kind":"OpenToday"}`) so the desktop frontend can `switch`
/// on a string discriminant instead of arbitrary indices.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Action {
    // ── chrome ────────────────────────────────────────────────────
    /// Open the picker overlay (Cmd/Ctrl+P).
    OpenPicker,
    /// Open the slash / colon command palette.
    OpenCommandPalette,
    /// Toggle the help overlay.
    ToggleHelp,
    /// Toggle the left sidebar.
    ToggleSidebar,
    /// Toggle the right backlinks panel.
    ToggleBacklinks,
    /// Open the settings modal.
    OpenSettings,
    /// Quit the application (TUI: `q q` chord; desktop: `Cmd+Q` via OS).
    Quit,

    // ── navigation ───────────────────────────────────────────────
    /// Open today's journal.
    OpenToday,
    /// Open the previous day's journal (only meaningful on a journal page).
    PrevDay,
    /// Open the next day's journal.
    NextDay,
    /// Selection one block down (in flat DFS order, skipping collapsed subtrees).
    SelectionDown,
    /// Selection one block up.
    SelectionUp,
    /// Open the ref under the cursor (`[[ref]]`, `#tag`, `((blk-…))`).
    /// In Normal mode where the cursor sits on a block, falls through
    /// to "enter Insert at end of block" when there's no ref under cursor.
    OpenRefUnderCursor,

    // ── block structure ──────────────────────────────────────────
    /// Enter Insert at end of current block (TUI `i` / `Enter` on Normal).
    EnterInsert,
    /// Enter Insert at start of current block (TUI `I`).
    EnterInsertAtStart,
    /// New block below + Insert (TUI `o`; desktop `Enter` in Insert).
    NewBlockBelow,
    /// New block above + Insert (TUI `O`).
    NewBlockAbove,
    /// Indent the current block (`Tab`).
    IndentBlock,
    /// Outdent the current block (`Shift-Tab`).
    OutdentBlock,
    /// Move the current block up among its siblings.
    MoveBlockUp,
    /// Move the current block down.
    MoveBlockDown,
    /// Delete the current block (TUI `d d` chord; desktop `Backspace` on empty).
    DeleteBlock,
    /// Fold / unfold the current block.
    ToggleCollapsed,
    /// Cycle TODO state (none → TODO → DONE → none).
    ToggleTodo,
    /// Copy the current block's `((blk-…))` ref handle to clipboard.
    CopyBlockRef,

    // ── insert-mode commits ──────────────────────────────────────
    /// Commit in-flight edit and leave Insert mode (`Esc`).
    ExitInsert,
    /// Commit + new block below + continue editing (`Enter` in Insert).
    CommitAndContinue,
    /// Backspace on an empty block deletes the block (desktop in Insert).
    DeleteEmptyBlock,

    // ── visual / range ───────────────────────────────────────────
    /// Enter Visual mode (TUI `v`).
    EnterVisual,
    /// Yank the visual range to register.
    YankRange,
    /// Delete the visual range.
    DeleteRange,

    // ── code execution ───────────────────────────────────────────
    /// Run the code block under the cursor through `outl-exec`.
    RunCodeBlock,

    // ── inline markdown wrappers (Insert mode only) ──────────────
    //
    // These mirror the conventions every popular markdown editor
    // ships (Notion, Obsidian, Discord, Slack): chord wraps the
    // active textarea selection, or inserts the delimiter pair
    // around the caret. The handler accesses `document.activeElement`
    // — it does not need the workspace.
    /// Wrap selection (or insert empty pair) with `**bold**`.
    WrapBold,
    /// Wrap with `_italic_` (outl's canonical italic delimiter; the
    /// parser also accepts `*…*` for compatibility with mainstream
    /// markdown, but new content uses underscores).
    WrapItalic,
    /// Wrap with `` `code` ``.
    WrapCode,
    /// Wrap with `~~strike~~`.
    WrapStrike,
    /// Wrap selection as the label of `[label](url)` and select
    /// `url` for the user to type the destination into.
    InsertLink,

    // ── undo / redo ──────────────────────────────────────────────
    /// Undo the last mutation (`u` in TUI Normal; `Cmd+Z` desktop).
    Undo,
    /// Redo (`Ctrl+R` in TUI Normal; `Cmd+Shift+Z` desktop).
    Redo,
}
