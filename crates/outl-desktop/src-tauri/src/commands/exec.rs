//! `run_code_block` — thin wrapper over
//! `outl_tauri_shared::commands::exec::run_code_block` (which in turn
//! delegates the orchestration to `outl_actions::exec`). Adding
//! behaviour here is almost always a smell; promote it upstream so the
//! mobile client picks it up for free.

use tauri::State;

use crate::state::AppState;
use outl_tauri_shared::commands::exec::{
    self as shared, AutoRunReply, EmbedContent, RunCodeBlockReply,
};
use std::collections::HashMap;

#[tauri::command]
pub(crate) fn run_code_block(
    page_id: String,
    block_id: String,
    state: State<'_, AppState>,
) -> Result<RunCodeBlockReply, String> {
    shared::run_code_block(state.inner(), page_id, block_id)
}

#[tauri::command]
pub(crate) fn run_auto_run_blocks(
    page_id: String,
    state: State<'_, AppState>,
) -> Result<AutoRunReply, String> {
    shared::run_auto_run_blocks(state.inner(), page_id)
}

#[tauri::command]
pub(crate) fn resolve_embeds(
    handles: Vec<String>,
    state: State<'_, AppState>,
) -> Result<HashMap<String, EmbedContent>, String> {
    shared::resolve_embeds(state.inner(), handles)
}
