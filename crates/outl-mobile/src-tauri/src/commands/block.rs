//! Block mutation commands.

use outl_actions::{
    append_block, create_after, delete, edit_text, indent, move_down, move_up, outdent,
    paste_markdown as action_paste_markdown, set_block_collapsed as action_set_block_collapsed,
    toggle_quote as action_toggle_quote, toggle_todo as action_toggle_todo, ActionError,
    PasteAnchor,
};
use tauri::State;

use crate::helpers::{
    build_page_view, finish_in_page, finish_in_page_with, parse_node_id, with_ws_mut,
};
use crate::state::{AppState, CreateBlockReply, PageView};

#[tauri::command]
pub(crate) fn create_block(
    page_id: String,
    after_id: Option<String>,
    parent_id: Option<String>,
    text: Option<String>,
    state: State<'_, AppState>,
) -> Result<CreateBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let text_owned = text.clone();
    let (new_id, view) = finish_in_page_with(&state, page, |ws| match after_id {
        Some(id) => {
            let node = parse_node_id(&id).map_err(ActionError::NotInTree)?;
            create_after(ws, &state.hlc, node, text_owned.as_deref())
        }
        None => {
            let parent = match parent_id {
                Some(id) => parse_node_id(&id).map_err(ActionError::NotInTree)?,
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
pub(crate) fn set_block_collapsed(
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
