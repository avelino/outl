//! Page / journal navigation commands.

use outl_actions::{
    find_by_slug, journal_slug, journal_title, list_pages, next_journal_date, open_journal,
    open_or_create_by_name, open_or_create_by_ref, open_today, page_meta as page_meta_action,
    previous_journal_date, today, PageKind, PageMeta,
};
use tauri::State;

use crate::helpers::{build_page_view, parse_date, storage_root_or_err, with_ws, with_ws_mut};
use crate::state::{AppState, PageView};

#[tauri::command]
pub(crate) fn list_all_pages(state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
    with_ws(&state, |ws| Ok(list_pages(ws)))
}

#[tauri::command]
pub(crate) fn search_pages(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    with_ws(&state, |ws| {
        let q = query.trim().to_lowercase();
        let pages = list_pages(ws);
        if q.is_empty() {
            return Ok(pages.into_iter().take(25).collect());
        }
        let mut scored: Vec<(u8, PageMeta)> = pages
            .into_iter()
            .filter_map(|p| {
                let title = p.title.to_lowercase();
                let slug = p.slug.to_lowercase();
                let score = if title == q || slug == q {
                    0
                } else if title.starts_with(&q) || slug.starts_with(&q) {
                    1
                } else if title.contains(&q) || slug.contains(&q) {
                    2
                } else {
                    return None;
                };
                Some((score, p))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
        Ok(scored.into_iter().map(|(_, p)| p).take(25).collect())
    })
}

#[tauri::command]
pub(crate) fn open_today_journal(state: State<'_, AppState>) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn open_journal_for(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let date = parse_date(&slug)?;
    let id = with_ws_mut(&state, |ws| {
        open_journal(ws, &state.hlc, date).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn open_page_by_slug(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let existing = with_ws(&state, |ws| Ok(find_by_slug(ws, &slug)))?;
    let id = match existing {
        Some(id) => id,
        None => with_ws_mut(&state, |ws| {
            open_or_create_by_name(ws, &state.hlc, &slug, PageKind::Page).map_err(|e| e.to_string())
        })?,
    };
    with_ws(&state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn open_ref(target: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let id = with_ws_mut(&state, |ws| {
        open_or_create_by_ref(ws, &state.hlc, &target).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn previous_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(previous_journal_date(date)))
}

#[tauri::command]
pub(crate) fn next_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(next_journal_date(date)))
}

#[tauri::command]
pub(crate) fn today_slug_cmd() -> String {
    journal_slug(today())
}

#[tauri::command]
pub(crate) fn date_title(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_title(date))
}

#[tauri::command]
pub(crate) fn resolve_ref(
    target: String,
    state: State<'_, AppState>,
) -> Result<Option<PageMeta>, String> {
    with_ws(&state, |ws| {
        if let Some(id) = find_by_slug(ws, &target) {
            return Ok(page_meta_action(ws, id));
        }
        let normalised = outl_md::slug::slugify(&target);
        if normalised != target {
            if let Some(id) = find_by_slug(ws, &normalised) {
                return Ok(page_meta_action(ws, id));
            }
        }
        let lower = target.to_lowercase();
        Ok(list_pages(ws)
            .into_iter()
            .find(|p| p.title.to_lowercase() == lower))
    })
}
