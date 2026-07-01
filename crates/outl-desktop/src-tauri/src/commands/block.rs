//! Block mutation commands.
//!
//! Each command parses ids, delegates the mutation to `outl-actions`,
//! and projects the result back to `.md` + sidecar via
//! [`crate::helpers::finish_in_page`]. The only deliberate exception
//! is `set_block_collapsed`, which bypasses reprojection — see its
//! doc for why.

use outl_actions::{
    append_block, apply_page_md_with_sidecar, create_after, create_before, delete, edit_text,
    enclosing_page_id, indent, move_after, move_down, move_up, outdent,
    paste_markdown as action_paste_markdown, render_block_md,
    set_block_collapsed as action_set_block_collapsed, toggle_quote as action_toggle_quote,
    toggle_todo as action_toggle_todo, ActionError, PasteAnchor,
};
use tauri::State;
use tracing::warn;

use crate::helpers::{
    build_page_view, finish_in_page, finish_in_page_with, parse_node_id, storage_root_or_err,
    with_ws, with_ws_mut,
};
use crate::state::{AppState, CreateBlockReply, PageView};

#[tauri::command]
pub(crate) fn create_block(
    page_id: String,
    after_id: Option<String>,
    before_id: Option<String>,
    parent_id: Option<String>,
    text: Option<String>,
    state: State<'_, AppState>,
) -> Result<CreateBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let text_owned = text.clone();
    // Precedence: `before_id` (vim `O` / `Cmd/Ctrl+Shift+Enter` at col 0)
    // wins over `after_id` (vim `o` / `Enter`); falling back to "last
    // child of `parent_id`" (defaults to the page root) when neither is
    // set.
    let (new_id, view) = finish_in_page_with(&state, page, |ws| {
        if let Some(id) = &before_id {
            let node = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_before(ws, &state.hlc, node, text_owned.as_deref())
        } else if let Some(id) = &after_id {
            let node = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_after(ws, &state.hlc, node, text_owned.as_deref())
        } else {
            let parent = match &parent_id {
                Some(id) => parse_node_id(id).map_err(ActionError::NotInTree)?,
                None => page,
            };
            append_block(ws, &state.hlc, Some(parent), text_owned.as_deref())
        }
    })?;
    Ok(CreateBlockReply {
        view,
        new_id: new_id.to_string(),
    })
}

#[tauri::command]
pub(crate) fn edit_block(
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
pub(crate) fn toggle_todo(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| action_toggle_todo(ws, &state.hlc, node))
}

#[tauri::command]
pub(crate) fn toggle_quote(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| action_toggle_quote(ws, &state.hlc, node))
}

#[tauri::command]
pub(crate) fn delete_block(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| delete(ws, &state.hlc, node))
}

#[tauri::command]
pub(crate) fn indent_block(
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
pub(crate) fn outdent_block(
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
pub(crate) fn move_block_up(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| move_up(ws, &state.hlc, node))
}

#[tauri::command]
pub(crate) fn move_block_down(
    page_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(&state, page, |ws| move_down(ws, &state.hlc, node))
}

/// Move `node` (`id`) to sit immediately after `after_id`,
/// re-parenting it under the target's parent. This is the workspace
/// side of the desktop's cut-and-paste-block gesture (`Cmd+X` then
/// `Cmd+V` in view mode): the block keeps its identity, so every
/// `((blk-…))` ref and backlink pointing at it stays valid.
///
/// `page_id` is the page the user is viewing (the target's page).
/// When the cut block came from a *different* page, that source page
/// is re-rendered too so its `.md` no longer lists the moved block.
/// A paste that would drop the block inside its own subtree is
/// rejected upstream (`WouldCreateCycle`) — the frontend nudges
/// instead of emitting a no-op move.
#[tauri::command]
pub(crate) fn move_block_after(
    page_id: String,
    id: String,
    after_id: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let after = parse_node_id(&after_id)?;
    with_ws_mut(&state, |ws| {
        // Capture the source page *before* the move so a cross-page
        // paste re-renders the page the block left behind, not only
        // the one it landed on.
        let source_page = enclosing_page_id(ws, node);
        move_after(ws, &state.hlc, node, after).map_err(|e| e.to_string())?;
        if let Err(e) = apply_page_md_with_sidecar(ws, &root, page) {
            warn!("destination page md+sidecar sync failed: {e}");
        }
        if let Some(src) = source_page {
            if src != page {
                if let Err(e) = apply_page_md_with_sidecar(ws, &root, src) {
                    warn!("source page md+sidecar sync failed: {e}");
                }
            }
        }
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}

/// Render the block `id` and its subtree to clean outl markdown for
/// the block clipboard (`Cmd+C` in view mode). Read-only — the paste
/// (`Cmd+V`) re-ingests this text via [`paste_block_after`] and mints
/// fresh ids, so a copy duplicates rather than moves.
#[tauri::command]
pub(crate) fn copy_block_markdown(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let node = parse_node_id(&id)?;
    with_ws(&state, |ws| Ok(render_block_md(ws, node)))
}

/// Paste clipboard `text` (clean outl markdown) as the sibling(s)
/// immediately after `after_id` — the `Cmd+V` of a *copied* block in
/// view mode. Routes through the same `paste_markdown` pipeline the
/// external-clipboard paste uses, so the duplicated subtree gets
/// fresh ids.
#[tauri::command]
pub(crate) fn paste_block_after(
    page_id: String,
    after_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let after = parse_node_id(&after_id)?;
    finish_in_page(&state, page, |ws| {
        action_paste_markdown(ws, &state.hlc, PasteAnchor::AfterBlock(after), &text).map(|_| ())
    })
}

/// Set or flip the `collapsed` flag on a block. Deliberately bypasses
/// `finish_in_page` — `Op::SetCollapsed` changes neither the `.md`
/// body nor the sidecar, so reprojecting would just bump iCloud
/// upload metadata for every fold gesture. See the mobile mirror at
/// `crates/outl-mobile/src-tauri/src/lib.rs::set_block_collapsed`.
#[tauri::command]
pub(crate) fn set_block_collapsed(
    page_id: String,
    id: String,
    collapsed: bool,
    state: State<'_, AppState>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(&state)?;
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    with_ws_mut(&state, |ws| {
        action_set_block_collapsed(ws, &state.hlc, node, collapsed).map_err(|e| e.to_string())?;
        // SetCollapsed is still a real op that must converge — announce it like
        // any other commit (this path bypasses `finish_in_page`, which is where
        // the announce normally lives).
        crate::helpers::announce_after_commit(&state, ws, page);
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}

/// Paste external clipboard markdown as a tree of blocks. `caret` is
/// a Unicode codepoint offset into the host block's text — the
/// frontend converts `textarea.selectionStart` (UTF-16) via
/// `utf16OffsetToCharOffset` from `@outl/shared/paste` first.
#[tauri::command]
pub(crate) fn paste_markdown_at(
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
