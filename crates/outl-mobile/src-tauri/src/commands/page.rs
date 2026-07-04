//! Page / journal navigation commands.

use outl_actions::{
    find_by_slug, journal_slug, journal_title, list_pages, next_journal_date, open_journal,
    open_or_create_by_name, open_or_create_by_ref, open_today, page_meta as page_meta_action,
    previous_journal_date, read_page_outline_with_workspace,
    search_persons as action_search_persons, today, ActionError, OutlineNode, PageKind, PageMeta,
};
use tauri::State;

use crate::commands::block::create_block;
use crate::helpers::{build_page_view, parse_date, with_ws, with_ws_mut};
use crate::state::{AppState, PageView};

#[tauri::command]
pub(crate) fn list_all_pages(state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
    with_ws(&state, |ws| Ok(list_pages(ws)))
}

/// Filter known pages by a case-insensitive substring match against
/// title + slug. Used by the mobile ref suggester (the dropdown that
/// appears when the user is typing inside `[[…]]`).
///
/// Ranking, weakest filter to strongest:
///   - title or slug `starts_with(query)` outranks an interior match
///   - exact-equality (case-insensitive) ranks above prefix
///
/// We cap the response at 25 so the floating list stays cheap to
/// render on a phone. An empty query returns the most recent pages
/// (whatever order `list_pages` defaults to), trimmed to the cap.
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

/// Same shape as [`search_pages`], but filtered to `type:: person`
/// pages and ranked by the shared `outl_actions::search_persons`
/// helper. Powers the `@` mention autocomplete on mobile.
#[tauri::command]
pub(crate) fn search_persons(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<PageMeta>, String> {
    with_ws(&state, |ws| Ok(action_search_persons(ws, &query)))
}

/// Search the GitHub gemoji catalog for shortcodes matching `query`.
/// Powers the `:shortcode:` autocomplete in the mobile block editor.
/// Wraps `outl_md::emoji::search` — same ranking the TUI and desktop
/// use, no parallel catalog index ships in the IPA's JS bundle.
#[tauri::command]
pub(crate) fn outl_emoji_search(
    query: String,
    limit: usize,
) -> Result<Vec<outl_md::emoji::EmojiHit>, String> {
    Ok(outl_md::emoji::search(&query, limit))
}

#[tauri::command]
pub(crate) fn open_today_journal(state: State<'_, AppState>) -> Result<PageView, String> {
    let id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn open_journal_for(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let date = parse_date(&slug)?;
    let id = with_ws_mut(&state, |ws| {
        open_journal(ws, &state.hlc, date).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub(crate) fn open_page_by_slug(
    slug: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    // The command name says "slug" for backward-compat with the
    // frontend, but the input is whatever the user typed / clicked —
    // `[[avelino/outl]]` arrives verbatim. Try the literal first
    // (covers programmatic callers that already passed a clean slug
    // and the picker that selects an existing page), then fall
    // through to `open_or_create_by_name`, which slugifies the input
    // for the disk path and keeps the original as the page's title.
    // That way clicking a ref to a page that doesn't exist still
    // opens a fresh page in the same round-trip — the user never
    // sees an "invalid page slug" toast.
    let existing = with_ws(&state, |ws| Ok(find_by_slug(ws, &slug)))?;
    let id = match existing {
        Some(id) => id,
        None => with_ws_mut(&state, |ws| {
            open_or_create_by_name(ws, &state.hlc, &slug, PageKind::Page).map_err(|e| e.to_string())
        })?,
    };
    // Project the page's `.md` + sidecar before reading it back, but
    // only when the `.md` is absent from disk.
    // Without this, a page that was synced from another device exists
    // in the in-memory CRDT tree (so it appears in search) but its
    // `.md` file was never written to the new device's disk.
    // `read_page_outline` does `fs::read_to_string().unwrap_or_default()`,
    // so a missing file produces an empty outline — the page opens blank.
    // The guard (project only when absent) avoids rewriting the `.outl`
    // sidecar on every open: `build_sidecar` stamps `last_synced_at: now()`
    // so an unconditional call would create constant sync churn on the
    // hottest nav path even when nothing changed.
    with_ws_mut(&state, |ws| {
        if let Err(e) =
            outl_actions::apply_page_md_with_sidecar_if_absent(ws, &state.storage_root, id)
        {
            eprintln!("open_page_by_slug: apply_page_md_with_sidecar failed for {slug}: {e}");
        }
        Ok::<_, String>(())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

/// Open whatever a user-typed ref / tag / picker entry points at,
/// in one round-trip.
///
/// The frontend used to split the discrimination between a regex
/// (`^\d{4}-\d{2}-\d{2}$`) and two separate commands (`open_journal_for`
/// + `open_page_by_slug`), each of which validated strict. That meant
/// `[[2026-13-01]]` matched the regex, hit `open_journal_for`, and
/// surfaced an `invalid date slug` toast — even though falling through
/// to "create a regular page named `2026-13-01`" was the obviously
/// right behaviour. `open_or_create_by_ref` is the single decision
/// tree (date → journal, else literal/slugified/title match → existing
/// page, else create), so the command never bubbles `invalid …` back
/// for normal user input and the frontend has no branching to drift.
#[tauri::command]
pub(crate) fn open_ref(
    target: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let id = with_ws_mut(&state, |ws| {
        open_or_create_by_ref(ws, &state.hlc, &target).map_err(|e| e.to_string())
    })?;
    // Project the page's `.md` + sidecar after the mutation. Without
    // this, `open_or_create_by_ref` creates the page in the op log
    // but `pages/<slug>.md` never lands on disk — `WorkspaceIndex`
    // disagrees with the tree CRDT silently, the just-inserted
    // `[[@name]]` link points at a phantom until the next save
    // touches the page. Mirrors the desktop fix.
    with_ws_mut(&state, |ws| {
        if let Err(e) = outl_actions::apply_page_md_with_sidecar(ws, &state.storage_root, id) {
            let msg = format!("{e}");
            eprintln!("open_ref: apply_page_md_with_sidecar failed for {target}: {msg}");
            let _ = tauri::Emitter::emit(
                &app,
                "ref-projection-failed",
                serde_json::json!({ "target": target, "error": msg }),
            );
        }
        Ok::<_, String>(())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
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
        // 1. Literal slug match (covers picker selections + clean refs).
        if let Some(id) = find_by_slug(ws, &target) {
            return Ok(page_meta_action(ws, id));
        }
        // 2. Normalised slug match — `[[avelino/outl]]` should resolve
        //    to the page stored as `avelino-outl` (same rule the
        //    `open_or_create_by_name` path uses on creation, so refs
        //    typed before the page existed still find it after).
        //    Skip when slugify produced the same string we already
        //    tried in step 1.
        let normalised = outl_md::slug::slugify(&target);
        if normalised != target {
            if let Some(id) = find_by_slug(ws, &normalised) {
                return Ok(page_meta_action(ws, id));
            }
        }
        // 3. Title lookup (case-insensitive). Last resort so a user
        //    who renamed the title but kept the slug still resolves.
        let lower = target.to_lowercase();
        Ok(list_pages(ws)
            .into_iter()
            .find(|p| p.title.to_lowercase() == lower))
    })
}

// ---------------------------------------------------------------------------
// Compat shims
// ---------------------------------------------------------------------------

/// Legacy: returns the outline of today's journal so the old frontend
/// that doesn't know about pages still works.
#[tauri::command]
pub(crate) fn list_outline(state: State<'_, AppState>) -> Result<Vec<OutlineNode>, String> {
    let today_id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        let meta = page_meta_action(ws, today_id)
            .ok_or_else(|| ActionError::NotInTree(today_id.to_string()))
            .map_err(|e| e.to_string())?;
        read_page_outline_with_workspace(&state.storage_root, &meta, ws)
            .map(|po| po.nodes)
            .map_err(|e| e.to_string())
    })
}

/// Legacy quick capture used by older frontends.
#[tauri::command]
pub(crate) fn add_block(text: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("empty block".to_string());
    }
    let today_id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    create_block(today_id.to_string(), None, None, Some(trimmed), state).map(|r| r.view)
}
