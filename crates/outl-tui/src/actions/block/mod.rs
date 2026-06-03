//! Block-level mutations: Insert mode, create / indent / outdent /
//! delete / reorder blocks, TODO prefix cycle.
//!
//! All ops snapshot through [`crate::state::App::snapshot_for_undo`]
//! so the history stack can roll back any structural change. Saves
//! go through `App::save` in `lifecycle::persistence`.
//!
//! ## Module layout
//!
//! | Submodule       | What's in it                                                |
//! |-----------------|-------------------------------------------------------------|
//! | `insert`        | `enter_insert`, `commit_insert`, `abort_insert`             |
//! | `structural`    | create / indent / outdent / delete / move block             |
//! | `backlink_edit` | `apply_to_backlink_source`, `toggle_todo_backlink`          |
//! | `metadata`      | property writes, `toggle_pinned`, `toggle_todo`             |
//! | `mod.rs` (here) | TODO-prefix cycle helpers shared with `input::insert`       |

use crate::edit_buffer::EditBuffer;
use crate::state::{DONE_PREFIX, TODO_PREFIX};

mod backlink_edit;
mod insert;
mod metadata;
mod structural;

/// Cycle a block's TODO prefix: none → `TODO ` → `DONE ` → none.
///
/// Delegates to [`outl_actions::cycle_todo`] so the TUI and the
/// mobile client share the exact same rule for cycling state.
pub(crate) fn cycle_todo_state(text: &str) -> String {
    outl_actions::cycle_todo(text)
}

/// Cycle the TODO prefix directly on an [`EditBuffer`], preserving the
/// cursor's *visual* position relative to the user's text.
///
/// - none → `TODO `: prefix added, cursor shifts right by 5.
/// - `TODO ` → `DONE `: replace in place, cursor unchanged.
/// - `DONE ` → none: prefix removed, cursor shifts left by 5
///   (clamped to 0).
pub(crate) fn cycle_todo_inline(buffer: &mut EditBuffer) {
    let prefix_chars = TODO_PREFIX.chars().count(); // 5; same for both
    let current: String = buffer.chars.iter().take(prefix_chars).collect();
    if current == TODO_PREFIX {
        // Replace `TODO ` with `DONE ` in place — same length, cursor intact.
        for (i, ch) in DONE_PREFIX.chars().enumerate() {
            buffer.chars[i] = ch;
        }
        return;
    }
    if current == DONE_PREFIX {
        // Remove the 5-char prefix.
        for _ in 0..prefix_chars {
            buffer.chars.remove(0);
        }
        buffer.cursor = buffer.cursor.saturating_sub(prefix_chars);
        return;
    }
    // No prefix yet — prepend `TODO ` and shift cursor right.
    for (i, ch) in TODO_PREFIX.chars().enumerate() {
        buffer.chars.insert(i, ch);
    }
    buffer.cursor += prefix_chars;
}
