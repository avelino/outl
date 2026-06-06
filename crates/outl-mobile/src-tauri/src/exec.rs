//! Tauri adapter for `outl_actions::exec::run_code_block`.
//!
//! All the real work — walking the outline, resolving the page's
//! `.md` path, calling `outl_exec::run_block_at_index`, building the
//! Serde-friendly outcome — happens in `outl-actions` so the desktop
//! and mobile clients share the same flow. This file owns only:
//!
//! - argument parsing (`String` → `NodeId`)
//! - the per-client `AppState` lookup (workspace + hlc + registry)
//! - composing the refreshed [`PageView`] into the wire reply
//!
//! Adding behaviour here is almost always a smell; promote it to
//! `outl_actions::exec` instead so the desktop picks it up for free.

use outl_actions::{run_code_block as action_run_code_block, ExecOutputDto, RunCodeBlockOutcome};
use serde::Serialize;
use tauri::State;
use tracing::warn;

use crate::{build_page_view, parse_node_id, with_ws_mut, AppState, PageView};

/// Wire reply for `run_code_block`. Adds the per-client [`PageView`]
/// to the shared [`RunCodeBlockOutcome`] so the frontend re-renders
/// in a single round-trip.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunCodeBlockReply {
    /// The detected language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime
    /// bailed before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
    /// Refreshed view of the page so the frontend re-renders without
    /// a follow-up `open_page_by_slug` round-trip — `outl-exec`
    /// already wrote the result subblock and reconciled with the op
    /// log.
    pub view: PageView,
}

#[tauri::command]
pub(crate) fn run_code_block(
    page_id: String,
    block_id: String,
    state: State<'_, AppState>,
) -> Result<RunCodeBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    let root = state.storage_root.clone();
    let registry = state.registry.clone();

    with_ws_mut(&state, |ws| {
        let RunCodeBlockOutcome {
            language,
            result_ok,
            error,
        } = action_run_code_block(ws, &state.hlc, &root, &registry, page, block)
            .map_err(|e| e.to_string())?;

        if let Some(msg) = error.as_ref() {
            warn!("run_code_block runtime error: {msg}");
        }

        let view = build_page_view(ws, &root, page).map_err(|e| e.to_string())?;
        Ok(RunCodeBlockReply {
            language,
            result_ok,
            error,
            view,
        })
    })
}
