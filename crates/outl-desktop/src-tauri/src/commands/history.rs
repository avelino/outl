//! Undo / redo commands.
//!
//! Thin adapters over `outl_actions::history` — the stacks live in
//! `AppState::history` (one per page, snapshots are the page's
//! rendered `.md`), the restore routes through
//! `outl_actions::restore_page_md` so every undo / redo is new ops in
//! the log. `finish_in_page_with` (helpers.rs) is the recording side
//! of this pair.

use outl_actions::render_page_md;
use tauri::State;

use crate::helpers::{build_page_view, parse_node_id, storage_root_or_err, with_ws_mut};
use crate::state::{AppState, PageView};

enum Direction {
    Undo,
    Redo,
}

/// Revert the last committed mutation on `page_id`. Errors with
/// `"nothing to undo"` when the stack is empty so the frontend can
/// surface it as a status message.
#[tauri::command]
pub(crate) fn undo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    step_history(&state, &page_id, Direction::Undo)
}

/// Re-apply the mutation the last `undo_page` reverted.
#[tauri::command]
pub(crate) fn redo_page(page_id: String, state: State<'_, AppState>) -> Result<PageView, String> {
    step_history(&state, &page_id, Direction::Redo)
}

fn step_history(
    state: &State<'_, AppState>,
    page_id: &str,
    direction: Direction,
) -> Result<PageView, String> {
    let root = storage_root_or_err(state.inner())?;
    let page = parse_node_id(page_id)?;
    with_ws_mut(state.inner(), |ws| {
        let current = render_page_md(ws, page);
        let snapshot = {
            let mut map = state.history.lock();
            let stacks = map.entry(page).or_default();
            match direction {
                Direction::Undo => stacks.undo(current),
                Direction::Redo => stacks.redo(current),
            }
        }
        .ok_or_else(|| {
            match direction {
                Direction::Undo => "nothing to undo",
                Direction::Redo => "nothing to redo",
            }
            .to_string()
        })?;
        outl_actions::restore_page_md(ws, &state.hlc, &root, page, &snapshot)
            .map_err(|e| e.to_string())?;
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}
