//! Page / journal navigation commands — thin wrappers over
//! `outl_tauri_shared::commands::page`.

use outl_actions::PageMeta;
use tauri::State;

use crate::state::{AppState, PageView};
use outl_tauri_shared::commands::page as shared;
use outl_tauri_shared::state::BlockHit;

#[tauri::command]
pub(crate) fn list_all_pages(state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
    shared::list_all_pages(state.inner())
}

#[tauri::command]
pub(crate) fn search_pages(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    shared::search_pages(state.inner(), query)
}

/// Same shape as [`search_pages`], but filtered to `type:: person`
/// pages. Powers the `@` mention autocomplete in the desktop client.
#[tauri::command]
pub(crate) fn search_persons(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    shared::search_persons(state.inner(), query)
}

/// Fuzzy-search block text for the `((…))` block-ref autocomplete in
/// the desktop's block editor. Returns each hit's ref handle + snippet.
#[tauri::command]
pub(crate) fn search_blocks(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<BlockHit>, String> {
    shared::search_blocks(state.inner(), query)
}

/// Search the GitHub gemoji catalog for shortcodes matching `query`.
/// Powers the `:shortcode:` autocomplete in the desktop's block editor.
#[tauri::command]
pub(crate) fn outl_emoji_search(
    query: String,
    limit: usize,
) -> Result<Vec<outl_md::emoji::EmojiHit>, String> {
    shared::emoji_search(query, limit)
}

#[tauri::command]
pub(crate) fn open_today_journal(state: State<'_, AppState>) -> Result<PageView, String> {
    shared::open_today_journal(state.inner())
}

#[tauri::command]
pub(crate) fn open_journal_for(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_journal_for(state.inner(), slug)
}

#[tauri::command]
pub(crate) fn open_page_by_slug(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_page_by_slug(state.inner(), slug)
}

#[tauri::command]
pub(crate) fn open_ref(
    target: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    shared::open_ref(state.inner(), &app, target)
}

#[tauri::command]
pub(crate) fn previous_day(slug: String) -> Result<String, String> {
    shared::previous_day(slug)
}

#[tauri::command]
pub(crate) fn next_day(slug: String) -> Result<String, String> {
    shared::next_day(slug)
}

#[tauri::command]
pub(crate) fn today_slug_cmd() -> String {
    shared::today_slug()
}

#[tauri::command]
pub(crate) fn date_title(slug: String) -> Result<String, String> {
    shared::date_title(slug)
}

#[tauri::command]
pub(crate) fn resolve_ref(
    target: String,
    state: State<'_, AppState>,
) -> Result<Option<PageMeta>, String> {
    shared::resolve_ref(state.inner(), target)
}
