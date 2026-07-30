//! Asset commands — thin `#[tauri::command]` wrappers over the shared
//! bodies in `outl_tauri_shared::commands::asset`. Wire names and reply
//! shapes are the shared crate's contract; nothing is added here.

use tauri::State;

use crate::state::{AppState, PageView};
use outl_actions::ImportedAsset;
use outl_tauri_shared::commands::asset as shared;

#[tauri::command]
pub(crate) fn open_asset(url: String, state: State<'_, AppState>) -> Result<(), String> {
    shared::open_asset(state.inner(), url)
}

#[tauri::command]
pub(crate) fn import_asset_file(
    source_path: String,
    state: State<'_, AppState>,
) -> Result<ImportedAsset, String> {
    shared::import_asset_file(state.inner(), source_path)
}

#[tauri::command]
pub(crate) fn attach_asset(
    source_path: String,
    page_id: String,
    after_block_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::attach_asset(state.inner(), source_path, page_id, after_block_id)
}
