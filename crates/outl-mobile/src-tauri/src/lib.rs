//! outl-mobile — Tauri 2 mobile companion app.
//!
//! Thin glue layer:
//!
//! - **Storage:** `outl_core::JsonlStorage` writes the op log into the
//!   iCloud Ubiquity Container's `Documents/ops/` directory. Each
//!   device only writes to its own `ops-<actor>.jsonl`; iCloud syncs
//!   the files in for free.
//! - **Actions:** delegated wholesale to `outl-actions` so the TUI,
//!   the future Tauri desktop, and this mobile app all share the same
//!   semantics for edit / indent / outdent / TODO / delete / move /
//!   journal / page / backlinks.
//! - **Tauri commands:** lightweight wrappers that parse `String`
//!   ids, call into `outl-actions`, and return the new outline so the
//!   Solid frontend renders in a single round-trip.
//!
//! ## Async startup
//!
//! The Tauri `setup` callback returns immediately so the WebView
//! starts painting right away. Opening the iCloud workspace (filesystem
//! reads + op-log replay) runs on a background thread; commands that
//! need the workspace return a `workspace_loading` error until it's
//! ready, and the frontend retries on a short interval. As soon as the
//! workspace lands, Tauri emits a `workspace-ready` event the frontend
//! can listen for to refresh proactively.

mod icloud_path;

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use chrono::NaiveDate;
use outl_actions::{
    append_block, apply_page_md_with_sidecar, backlinks_for_page, create_after, date_from_slug,
    delete, edit_text, find_by_slug, indent, journal_slug, journal_title, list_pages,
    migrate_legacy_into_today, move_down, move_up, next_journal_date, open_journal,
    open_or_create_by_name, open_today, outdent, page_meta as page_meta_action,
    paste_markdown as action_paste_markdown, previous_journal_date, read_page_view_with_workspace,
    set_block_collapsed as action_set_block_collapsed, today, toggle_todo as action_toggle_todo,
    ActionError, Backlink, OutlineNode, PageKind, PageMeta, PasteAnchor,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use parking_lot::{Mutex, MutexGuard};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tracing::{info, warn};

/// Sub-path under the iCloud Ubiquity Container where the workspace
/// lives.
///
/// The container itself is already namespaced as
/// `iCloud.app.outl.mobile-app`, so re-tagging an inner `outl/`
/// folder underneath it is noise. We use the standard iOS
/// `Documents/` directory directly: iCloud Documents only syncs that
/// path between devices, and the resulting layout matches what the
/// user sees in the Files app.
///
/// Kept non-dotted because iCloud Documents skips paths starting
/// with `.` when syncing between devices.
const WORKSPACE_SUBDIR: [&str; 1] = ["Documents"];

fn workspace_root_in(container: &Path) -> PathBuf {
    let mut p = container.to_path_buf();
    for seg in WORKSPACE_SUBDIR {
        p.push(seg);
    }
    p
}

fn ops_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("ops")
}

/// iCloud Ubiquity Container identifier registered in the
/// `com.apple.developer.icloud-container-identifiers` entitlement.
const ICLOUD_CONTAINER_ID: &str = "iCloud.app.outl.mobile-app";

/// Sentinel error returned by every workspace-touching command while
/// the workspace is still being opened on the background thread.
const ERR_LOADING: &str = "workspace_loading";

/// Shared mutable state held by Tauri.
struct AppState {
    /// `None` until the background opener completes.
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    storage_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceSummary {
    blocks: usize,
    ops: usize,
    actor: String,
    storage_root: String,
    ready: bool,
}

/// Reply shape for every "open page / open journal" command. Bundles
/// the page meta with the outline so the frontend gets everything in
/// one trip.
#[derive(Debug, Clone, Serialize)]
struct PageView {
    page: PageMeta,
    outline: Vec<OutlineNode>,
    backlinks: Vec<Backlink>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_node_id(s: &str) -> Result<NodeId, String> {
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| format!("invalid node id {s}: {e}"))
}

fn parse_date(slug: &str) -> Result<NaiveDate, String> {
    date_from_slug(slug).ok_or_else(|| format!("invalid date slug: {slug}"))
}

/// Acquire a read-only handle to the workspace. Returns the
/// `workspace_loading` sentinel string while the background opener is
/// still running.
fn with_ws<F, T>(state: &State<'_, AppState>, f: F) -> Result<T, String>
where
    F: FnOnce(&Workspace) -> Result<T, String>,
{
    let guard: MutexGuard<'_, Option<Workspace>> = state.workspace.lock();
    match guard.as_ref() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// Acquire a mutable handle to the workspace.
fn with_ws_mut<F, T>(state: &State<'_, AppState>, f: F) -> Result<T, String>
where
    F: FnOnce(&mut Workspace) -> Result<T, String>,
{
    let mut guard = state.workspace.lock();
    match guard.as_mut() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

fn build_page_view(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    // Read the outline straight from the page's `.md` (+ sidecar for
    // stable block ids). This is the v0 contract: `.md` is the source
    // of truth, the op log is history. `project_outline(workspace,_)`
    // is still available for tools that need to materialise from the
    // op log, but the UI must not use it — it would silently disagree
    // with what the user sees in Files.app or any other editor.
    //
    // The workspace-aware variant overlays `Op::SetCollapsed` so the
    // returned `OutlineNode.collapsed` reflects the op log (the only
    // place that state legitimately lives — sidecars LWW under iCloud
    // and would lose flips).
    let outline = read_page_view_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| Vec::new());
    let backlinks = backlinks_for_page(workspace, storage_root, &meta);
    Ok(PageView {
        page: meta,
        outline,
        backlinks,
    })
}

/// Apply a workspace mutation `f` and project the result back to
/// `.md` + sidecar.
///
/// The op log is the source of truth: every concurrent edit between
/// peers ends up there (each device appends to its own
/// `ops-<actor>.jsonl`, iCloud syncs files individually, and HLC
/// ordering merges them deterministically). The `.md` and the
/// sidecar are projections — we always regenerate them after the
/// workspace mutation so what the user reads on disk matches the
/// op-log state.
///
/// We do **not** run `reconcile_md` before `f`. The op log is already
/// up to date with whatever peers have delivered through the jsonl
/// files; trying to "catch up" from the `.md` would risk emitting
/// Delete cascades when the on-disk `.md` lagged behind the op log
/// (which it does on every iCloud propagation window).
fn finish_in_page<F>(state: &State<'_, AppState>, page_id: NodeId, f: F) -> Result<PageView, String>
where
    F: FnOnce(&mut Workspace) -> Result<(), ActionError>,
{
    with_ws_mut(state, |ws| {
        f(ws).map_err(|e| e.to_string())?;
        if let Err(e) = apply_page_md_with_sidecar(ws, &state.storage_root, page_id) {
            warn!("page md+sidecar sync failed: {e}");
        }
        build_page_view(ws, &state.storage_root, page_id).map_err(|e| e.to_string())
    })
}

// ---------------------------------------------------------------------------
// Page / journal commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_all_pages(state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
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
fn search_pages(query: String, state: State<'_, AppState>) -> Result<Vec<PageMeta>, String> {
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
fn open_today_journal(state: State<'_, AppState>) -> Result<PageView, String> {
    let id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn open_journal_for(slug: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let date = parse_date(&slug)?;
    let id = with_ws_mut(&state, |ws| {
        open_journal(ws, &state.hlc, date).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn open_page_by_slug(slug: String, state: State<'_, AppState>) -> Result<PageView, String> {
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
    with_ws(&state, |ws| {
        build_page_view(ws, &state.storage_root, id).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn previous_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(previous_journal_date(date)))
}

#[tauri::command]
fn next_day(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_slug(next_journal_date(date)))
}

#[tauri::command]
fn today_slug_cmd() -> String {
    journal_slug(today())
}

#[tauri::command]
fn date_title(slug: String) -> Result<String, String> {
    let date = parse_date(&slug)?;
    Ok(journal_title(date))
}

#[tauri::command]
fn workspace_stats(state: State<'_, AppState>) -> WorkspaceSummary {
    let guard = state.workspace.lock();
    let storage_root = state.storage_root.to_string_lossy().into_owned();
    match guard.as_ref() {
        Some(ws) => WorkspaceSummary {
            blocks: ws.tree().node_count(),
            ops: ws.log().len(),
            actor: ws.actor.to_string(),
            storage_root,
            ready: true,
        },
        None => WorkspaceSummary {
            blocks: 0,
            ops: 0,
            actor: String::new(),
            storage_root,
            ready: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Block mutation commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn create_block(
    page_id: String,
    after_id: Option<String>,
    parent_id: Option<String>,
    text: Option<String>,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let text_owned = text.clone();
    finish_in_page(&state, page, |ws| match after_id {
        Some(id) => {
            let node = parse_node_id(&id).map_err(ActionError::NotInTree)?;
            create_after(ws, &state.hlc, node, text_owned.as_deref()).map(|_| ())
        }
        None => {
            let parent = match parent_id {
                Some(id) => parse_node_id(&id).map_err(ActionError::NotInTree)?,
                None => page,
            };
            append_block(ws, &state.hlc, Some(parent), text_owned.as_deref()).map(|_| ())
        }
    })
}

#[tauri::command]
fn edit_block(
    page_id: String,
    id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| edit_text(ws, &state.hlc, node, &text))
}

#[tauri::command]
fn toggle_todo(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| action_toggle_todo(ws, &state.hlc, node))
}

#[tauri::command]
fn delete_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| delete(ws, &state.hlc, node))
}

#[tauri::command]
fn indent_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| match indent(ws, &state.hlc, node) {
        Err(ActionError::NoPreviousSibling(_)) => Ok(()),
        other => other,
    })
}

#[tauri::command]
fn outdent_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| match outdent(ws, &state.hlc, node) {
        Err(ActionError::AlreadyAtRoot(_)) => Ok(()),
        other => other,
    })
}

#[tauri::command]
fn move_block_up(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| move_up(ws, &state.hlc, node))
}

#[tauri::command]
fn move_block_down(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| move_down(ws, &state.hlc, node))
}

/// Set or flip the `collapsed` flag on a block.
///
/// Routes through `outl_actions::set_block_collapsed` which generates
/// an `Op::SetCollapsed` and applies it via `Workspace::apply`. The
/// op enters the device's `ops-<actor>.jsonl`, iCloud propagates the
/// file to peers, and the CRDT merges concurrent flips by HLC order.
/// Returns a freshly built page view so the frontend re-renders in a
/// single round trip.
///
/// **Deliberately bypasses `finish_in_page`.** Every other mutation
/// reprojects `.md` + sidecar at the end because it changed the
/// outline structure or text. `Op::SetCollapsed` changes neither —
/// the `.md` body stays byte-identical, the sidecar's structural
/// fields (id, position, hash, ref_handle) are untouched, and only
/// the workspace-internal `Tree.collapsed` set moves. Reprojecting
/// would write two files to disk and bump iCloud upload metadata
/// for every fold gesture; we skip both and just rebuild the page
/// view from the freshly-mutated workspace.
#[tauri::command]
fn set_block_collapsed(
    page_id: String,
    id: String,
    collapsed: bool,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    with_ws_mut(&state, |ws| {
        action_set_block_collapsed(ws, &state.hlc, node, collapsed).map_err(|e| e.to_string())?;
        build_page_view(ws, &state.storage_root, page).map_err(|e| e.to_string())
    })
}

/// Paste external clipboard markdown as a tree of blocks.
///
/// `caret` is a Unicode codepoint offset into the host block's text,
/// matching the convention `outl_actions::PasteAnchor::AtCaret` uses
/// (Rust `str::chars()` iterates codepoints, not UTF-16 code units).
/// The frontend converts `textarea.selectionStart` (UTF-16) into a
/// codepoint count via `utf16OffsetToCharOffset` before invoking
/// this command, so we get the right offset for text containing
/// emoji and other supplementary-plane characters too.
#[tauri::command]
fn paste_markdown_at(
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    finish_in_page(&state, page, |ws| {
        action_paste_markdown(
            ws,
            &state.hlc,
            PasteAnchor::AtCaret {
                block,
                caret: caret as usize,
            },
            &text,
        )
        .map(|_| ())
    })
}

#[tauri::command]
fn reload_workspace(state: State<'_, AppState>) -> Result<(), String> {
    let engine = outl_actions::SyncEngine::new(state.storage_root.clone(), state.hlc.actor());
    let mut fresh = engine
        .reload_workspace()
        .map_err(|e| format!("reload workspace: {e}"))?;
    // Catch any `.md` files iCloud delivered without their sidecar
    // (peer wrote only the projection, or an external editor like
    // vim touched the file). Runs before we resolve today's id so
    // newly-reconciled blocks show up in the rebuild that follows.
    reconcile_orphan_md(&mut fresh, &state.hlc, &state.storage_root);
    // Resolve today's journal *in the fresh workspace* so the page
    // id reflects the merged op log. `open_today` is idempotent —
    // when the page already exists it just returns the id; when it
    // doesn't, it creates one with the deterministic slug-derived
    // id, which both peers will agree on.
    let today_id = open_today(&mut fresh, &state.hlc).map_err(|e| e.to_string())?;
    let _ = engine.reproject_page(&fresh, today_id);
    *state.workspace.lock() = Some(fresh);
    Ok(())
}

#[tauri::command]
fn resolve_ref(target: String, state: State<'_, AppState>) -> Result<Option<PageMeta>, String> {
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
fn list_outline(state: State<'_, AppState>) -> Result<Vec<OutlineNode>, String> {
    let today_id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    with_ws(&state, |ws| {
        let meta = page_meta_action(ws, today_id)
            .ok_or_else(|| ActionError::NotInTree(today_id.to_string()))
            .map_err(|e| e.to_string())?;
        read_page_view_with_workspace(&state.storage_root, &meta, ws).map_err(|e| e.to_string())
    })
}

/// Legacy quick capture used by older frontends.
#[tauri::command]
fn add_block(text: String, state: State<'_, AppState>) -> Result<PageView, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("empty block".to_string());
    }
    let today_id = with_ws_mut(&state, |ws| {
        open_today(ws, &state.hlc).map_err(|e| e.to_string())
    })?;
    create_block(today_id.to_string(), None, None, Some(trimmed), state)
}

// ---------------------------------------------------------------------------
// Startup wiring
// ---------------------------------------------------------------------------

fn load_or_create_actor(local_dir: &Path) -> std::io::Result<ActorId> {
    let path = local_dir.join("actor");
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        let raw = raw.trim();
        if let Ok(ulid) = ulid::Ulid::from_str(raw) {
            info!("loaded existing actor id {ulid}");
            return Ok(ActorId(ulid));
        }
        warn!("invalid actor id in {}, regenerating", path.display());
    }
    let actor = ActorId::new();
    std::fs::write(&path, actor.to_string())?;
    info!("generated fresh actor id {actor}");
    Ok(actor)
}

fn resolve_storage_root(local_fallback: &Path) -> PathBuf {
    if let Some(container) = icloud_path::resolve_container(ICLOUD_CONTAINER_ID) {
        info!("using iCloud container at {}", container.display());
        container
    } else {
        warn!(
            "iCloud container unavailable, falling back to local {}",
            local_fallback.display()
        );
        local_fallback.to_path_buf()
    }
}

/// Background opener. Runs once per process; sets the inner
/// `Option<Workspace>` and emits the `workspace-ready` event when done.
fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    thread::spawn(move || {
        let storage = match JsonlStorage::open(ops_dir(&storage_root), actor) {
            Ok(s) => s,
            Err(e) => {
                warn!("background open: storage failed: {e}");
                return;
            }
        };
        let mut workspace = match Workspace::open_with_storage(
            actor,
            Box::new(storage),
            Some(storage_root.clone()),
        ) {
            Ok(w) => w,
            Err(e) => {
                warn!("background open: workspace failed: {e}");
                return;
            }
        };
        if let Err(e) = migrate_legacy_into_today(&mut workspace, &hlc) {
            warn!("legacy migration: {e}");
        }
        if let Err(e) = open_today(&mut workspace, &hlc) {
            warn!("could not pre-open today: {e}");
        }
        // Reconcile any `.md` files the op log doesn't know about yet —
        // imported journals (Roam dump, Logseq move), peer-written `.md`
        // that arrived without its sidecar, or files edited externally
        // in vim / VS Code. Running here means the very first
        // `build_page_view` call already sees their blocks (so e.g.
        // backlinks on today's journal include yesterday's imports).
        reconcile_orphan_md(&mut workspace, &hlc, &storage_root);
        *workspace_slot.lock() = Some(workspace);
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");
    });
}

/// Scan `<root>/journals/` and `<root>/pages/` for `.md` files that
/// are not represented in the op log yet — either no sidecar exists
/// (file was just imported, dropped in by vim, or written by a peer
/// that only shipped the projection) or the sidecar's
/// `last_synced_hash` is stale (the file was edited externally since
/// the last reconcile). Runs `reconcile_md` on each so the workspace,
/// the sidecar, and `.md` converge.
fn reconcile_orphan_md(workspace: &mut Workspace, hlc: &HlcGenerator, storage_root: &Path) {
    let engine = outl_actions::SyncEngine::new(storage_root.to_path_buf(), hlc.actor());
    let orphans = engine.scan_for_orphans();
    if orphans.is_empty() {
        return;
    }
    for path in &orphans {
        if let Err(e) = outl_md::reconcile::reconcile_md(workspace, hlc, path, None) {
            warn!("orphan reconcile failed for {}: {e}", path.display());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let local_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            std::fs::create_dir_all(&local_dir)?;

            let actor = load_or_create_actor(&local_dir)?;
            // `resolve_storage_root` returns the iCloud Ubiquity Container
            // root (the device-local mount). The workspace lives in
            // `Documents/` directly inside the container — the
            // container itself is already the outl namespace, so
            // there's no need for a second `outl/` folder. The TUI
            // is expected to be pointed at this same path via
            // `--path "<container>/Documents"`.
            let container_root = resolve_storage_root(&local_dir);
            let storage_root = workspace_root_in(&container_root);
            std::fs::create_dir_all(&storage_root)?;
            let hlc = HlcGenerator::new(actor);

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));

            spawn_workspace_opener(
                workspace.clone(),
                storage_root.clone(),
                hlc.clone(),
                app.handle().clone(),
            );

            app.manage(AppState {
                workspace,
                hlc,
                storage_root,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Page / journal navigation
            list_all_pages,
            search_pages,
            open_today_journal,
            open_journal_for,
            open_page_by_slug,
            previous_day,
            next_day,
            today_slug_cmd,
            date_title,
            workspace_stats,
            resolve_ref,
            // Mutations
            create_block,
            edit_block,
            toggle_todo,
            delete_block,
            indent_block,
            outdent_block,
            move_block_up,
            move_block_down,
            set_block_collapsed,
            paste_markdown_at,
            reload_workspace,
            // Legacy
            list_outline,
            add_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
