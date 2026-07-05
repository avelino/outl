//! `run_code_block` — thin wrapper over
//! `outl_tauri_shared::commands::exec::run_code_block` (which in turn
//! delegates the orchestration to `outl_actions::exec`). Adding
//! behaviour here is almost always a smell; promote it upstream so the
//! desktop client picks it up for free.

use tauri::State;

use crate::state::AppState;
use outl_tauri_shared::commands::exec::{self as shared, RunCodeBlockReply};

#[tauri::command]
pub(crate) fn run_code_block(
    page_id: String,
    block_id: String,
    state: State<'_, AppState>,
) -> Result<RunCodeBlockReply, String> {
    shared::run_code_block(state.inner(), page_id, block_id)
}
