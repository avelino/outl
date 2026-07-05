//! `run_code_block` command body — the Tauri side of
//! `outl_actions::exec::run_code_block`.
//!
//! All the real work — walking the outline, resolving the page's `.md`
//! path, calling `outl_exec::run_block_at_index`, building the
//! Serde-friendly outcome — happens in `outl-actions` so every client
//! shares the same flow. This body owns only:
//!
//! - argument parsing (`String` → `NodeId`)
//! - the `AppHost` lookup (workspace + storage_root + hlc + registry)
//! - composing the refreshed [`PageView`] into the wire reply
//!
//! Synchronous — Tauri serves sync commands from its own multi-threaded
//! worker pool, so a long-running runtime doesn't park the JS-side event
//! loop, but it **does** hold the workspace mutex for the whole call.
//! Making this async + `spawn_blocking` to release the mutex earlier is
//! tracked in the desktop polish backlog.

use outl_actions::{run_code_block as action_run_code_block, ExecOutputDto, RunCodeBlockOutcome};
use serde::Serialize;
use tracing::warn;

use crate::helpers::{build_page_view, parse_node_id, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

/// Wire reply for `run_code_block`. Adds the [`PageView`] to the shared
/// [`RunCodeBlockOutcome`] so the frontend re-renders in a single
/// round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct RunCodeBlockReply {
    /// The detected language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime bailed
    /// before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
    /// Refreshed view of the page so the frontend re-renders without a
    /// follow-up `open_page_by_slug` round-trip — `outl-exec` already
    /// wrote the result subblock and reconciled with the op log.
    pub view: PageView,
}

pub fn run_code_block<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
) -> Result<RunCodeBlockReply, String> {
    let root = state.storage_root()?;
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    let registry = state.exec_registry();

    with_ws_mut(state, |ws| {
        let RunCodeBlockOutcome {
            language,
            result_ok,
            error,
        } = action_run_code_block(ws, state.hlc(), &root, &registry, page, block)
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
