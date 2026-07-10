//! Structural template commands — thin wrappers over
//! `outl_tauri_shared::commands::template`. Wire names and reply shapes
//! are the shared crate's contract; nothing is added here. Registered
//! for parity with desktop so a template is reachable from the mobile UI
//! (the template sheet) without a plugin.

use tauri::State;

use crate::state::{AppState, PageView};
use outl_tauri_shared::commands::template as shared;
use outl_tauri_shared::state::TemplateDto;

/// List every structural template in the workspace (any page with a
/// non-empty `template::` property), for the template picker sheet.
///
/// The `_cmd` suffix avoids a glob-import collision with the shared
/// `outl_actions::list_templates` action re-exported into `commands`.
#[tauri::command]
pub(crate) fn list_templates_cmd(state: State<'_, AppState>) -> Result<Vec<TemplateDto>, String> {
    shared::list_templates(state.inner())
}

/// Deep-copy the template `name` under `target_block`, returning the
/// refreshed `PageView` of the block's enclosing page.
#[tauri::command]
pub(crate) fn instantiate_template_at(
    name: String,
    target_block: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::instantiate_template_at(state.inner(), name, target_block)
}
