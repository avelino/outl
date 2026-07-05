//! Page / journal navigation command bodies.

use outl_actions::{
    find_by_slug, journal_slug, journal_title, list_pages, next_journal_date, open_journal,
    open_or_create_by_name, open_or_create_by_ref, open_today, page_meta as page_meta_action,
    previous_journal_date, search_persons as action_search_persons, today, PageKind, PageMeta,
};

use crate::helpers::{build_page_view, parse_date, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

pub fn list_all_pages<S: AppHost>(state: &S) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| Ok(list_pages(ws)))
}

/// Filter known pages by a case-insensitive substring match against
/// title + slug. Powers the quick switcher and the `[[…]]` ref
/// suggester.
///
/// Ranking, weakest filter to strongest: title or slug
/// `starts_with(query)` outranks an interior match; exact equality
/// (case-insensitive) ranks above prefix. Capped at 25 so the floating
/// list stays cheap to render. An empty query returns the most recent
/// pages (whatever order `list_pages` defaults to), trimmed to the cap.
pub fn search_pages<S: AppHost>(state: &S, query: String) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| {
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
/// helper. Powers the `@` mention autocomplete.
pub fn search_persons<S: AppHost>(state: &S, query: String) -> Result<Vec<PageMeta>, String> {
    with_ws(state, |ws| Ok(action_search_persons(ws, &query)))
}

/// Search the GitHub gemoji catalog for shortcodes matching `query`.
/// Powers the `:shortcode:` autocomplete in the block editor. Wraps
/// `outl_md::emoji::search` so every client ranks identically — no
/// parallel catalog index lives in a frontend bundle.
///
/// Returns up to `limit` hits. Empty / whitespace-only query
/// short-circuits to an empty vec (matches the helper's contract).
pub fn emoji_search(query: String, limit: usize) -> Result<Vec<outl_md::emoji::EmojiHit>, String> {
    Ok(outl_md::emoji::search(&query, limit))
}

pub fn open_today_journal<S: AppHost>(state: &S) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let id = with_ws_mut(state, |ws| {
        open_today(ws, state.hlc()).map_err(|e| e.to_string())
    })?;
    with_ws(state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

pub fn open_journal_for<S: AppHost>(state: &S, slug: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let date = parse_date(&slug)?;
    let id = with_ws_mut(state, |ws| {
        open_journal(ws, state.hlc(), date).map_err(|e| e.to_string())
    })?;
    with_ws(state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

/// The command name says "slug" for backward-compat with the frontend,
/// but the input is whatever the user typed / clicked — try the literal
/// first (programmatic callers and the picker pass a clean slug), then
/// fall through to `open_or_create_by_name`, which slugifies the input
/// for the disk path and keeps the original as the page title. That way
/// clicking a ref to a page that doesn't exist still opens a fresh page
/// in the same round-trip.
///
/// Before building the view, lazily project the page's `.md` + sidecar
/// **only when the `.md` is absent from disk**. Without this, a page
/// synced from a peer exists in the in-memory CRDT tree (so it shows up
/// in search) but its `.md` was never written on this device;
/// `read_page_outline` does `fs::read_to_string().unwrap_or_default()`,
/// a missing file silently returns `""`, `parse("")` produces an empty
/// outline, and the page opens blank (issue #120).
/// The `_if_absent` guard avoids rewriting the `.outl` sidecar on every
/// open: `build_sidecar` stamps `last_synced_at: now()`, so an
/// unconditional projection would create constant sync churn on the
/// hottest nav path even when nothing changed.
pub fn open_page_by_slug<S: AppHost>(state: &S, slug: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let existing = with_ws(state, |ws| Ok(find_by_slug(ws, &slug)))?;
    let id = match existing {
        Some(id) => id,
        None => with_ws_mut(state, |ws| {
            open_or_create_by_name(ws, state.hlc(), &slug, PageKind::Page)
                .map_err(|e| e.to_string())
        })?,
    };
    with_ws_mut(state, |ws| {
        if let Err(e) = outl_actions::apply_page_md_with_sidecar_if_absent(ws, &root, id) {
            tracing::warn!("open_page_by_slug: apply_page_md_with_sidecar failed for {slug}: {e}");
        }
        Ok(())
    })?;
    with_ws(state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

/// Open whatever a user-typed ref / tag / picker entry points at, in one
/// round-trip. `open_or_create_by_ref` is the single decision tree
/// (date → journal, else literal/slugified/title match → existing page,
/// else create), so the command never bubbles `invalid …` back for
/// normal user input and the frontend has no branching to drift.
///
/// Two-phase: resolve-or-create the target NodeId (mutation), then
/// project the page's `.md` + sidecar to disk before building the view.
/// Without the projection, `open_or_create_by_ref` would `Op::Create` +
/// `SetProp` the page into the op log but `pages/<slug>.md` would never
/// land on disk — `WorkspaceIndex` (which parses `.md` from disk) would
/// silently disagree with the tree CRDT, and a peer pulling the
/// workspace would never see the page at all.
pub fn open_ref<S: AppHost>(
    state: &S,
    app: &tauri::AppHandle,
    target: String,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let id = with_ws_mut(state, |ws| {
        open_or_create_by_ref(ws, state.hlc(), &target).map_err(|e| e.to_string())
    })?;
    with_ws_mut(state, |ws| {
        if let Err(e) = outl_actions::apply_page_md_with_sidecar(ws, &root, id) {
            // Non-fatal: the op log already has the mutation; the `.md`
            // projection will be retried on the next save / by the
            // orphan scanner on the next boot. Surface both to the local
            // log AND to the frontend via a dedicated event so the UI
            // can show a toast — the ref is already in the user's buffer
            // at this point, and silently leaving the `.md` un-projected
            // means the next reopen shows a "link to nothing".
            let msg = format!("{e}");
            tracing::warn!("open_ref: apply_page_md_with_sidecar failed for {target}: {msg}");
            let _ = tauri::Emitter::emit(
                app,
                "ref-projection-failed",
                serde_json::json!({ "target": target, "error": msg }),
            );
        }
        Ok(())
    })?;
    with_ws(state, |ws| {
        build_page_view(ws, &root, id).map_err(|e| e.to_string())
    })
}

pub fn previous_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(previous_journal_date(date)))
}

pub fn next_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(next_journal_date(date)))
}

pub fn today_slug() -> String {
    journal_slug(today())
}

pub fn date_title(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_title(date))
}

/// Resolve (never create) what a ref would land on, for autocomplete
/// previews. Three phases: literal slug match, normalised slug match
/// (`[[avelino/outl]]` resolves to the page stored as `avelino-outl` —
/// the same rule `open_or_create_by_name` uses on creation), and a
/// last-resort case-insensitive title lookup.
pub fn resolve_ref<S: AppHost>(state: &S, target: String) -> Result<Option<PageMeta>, String> {
    with_ws(state, |ws| {
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
