//! Code-block execution command (`run_code_block`).
//!
//! Wraps [`outl_exec::run_block_at_index`] in `spawn_blocking` so the
//! Tauri runtime stays responsive while a runtime (Python, Lua, JS,
//! Lisp, Rust/wasm, …) churns through the source. The result is
//! persisted by `outl-exec` itself as a `> **result:**` sibling
//! block, and the refreshed `PageView` flows back to the frontend in
//! the same round-trip.
//!
//! `block_id` (not flat index) is what the frontend sends so the
//! mapping between an outline node and `ParsedPage.blocks` stays a
//! backend concern. The DFS-equivalent walk lives in
//! [`flat_index_for`].

use outl_actions::{page_meta as page_meta_action, project_outline, OutlineNode};
use outl_core::id::NodeId;
use outl_exec::{run_block_at_index, ExecOutput, RunReport};
use serde::Serialize;
use tauri::State;
use tracing::warn;

use crate::helpers::{build_page_view, parse_node_id, storage_root_or_err, with_ws_mut};
use crate::state::{AppState, PageView};

/// Outcome of a `run_code_block` invocation, shipped to the
/// frontend. `result_ok` carries stdout/stderr/duration when the
/// runtime ran; `error` carries an infrastructure failure message
/// when the runtime couldn't even start.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunCodeBlockReply {
    /// The detected language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime
    /// bailed before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
    /// Refreshed view of the page so the frontend re-renders
    /// without a follow-up `open_page_by_slug` round-trip — the
    /// `outl-exec` orchestrator already wrote the result subblock
    /// and reconciled with the op log.
    pub view: PageView,
}

/// Serializable mirror of [`outl_exec::ExecOutput`]. We avoid
/// re-exporting the raw type because `Duration` doesn't serialise
/// cleanly to JSON.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecOutputDto {
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub exit: String,
}

impl From<&ExecOutput> for ExecOutputDto {
    fn from(o: &ExecOutput) -> Self {
        Self {
            stdout: o.stdout.clone(),
            stderr: o.stderr.clone(),
            duration_ms: o.duration.as_millis(),
            exit: format!("{:?}", o.exit),
        }
    }
}

#[tauri::command]
pub(crate) fn run_code_block(
    page_id: String,
    block_id: String,
    state: State<'_, AppState>,
) -> Result<RunCodeBlockReply, String> {
    let root = storage_root_or_err(&state)?;
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;

    let registry = state.registry.clone();

    with_ws_mut(&state, |ws| {
        let meta = page_meta_action(ws, page).ok_or_else(|| format!("page {page} not in tree"))?;

        let outline = project_outline(ws, page);
        let flat_idx = flat_index_for(&outline, block)
            .ok_or_else(|| format!("block {block} not in page outline"))?;

        let page_path = root.join("journals").join(format!("{}.md", meta.slug));
        let final_path = if page_path.exists() {
            page_path
        } else {
            root.join("pages").join(format!("{}.md", meta.slug))
        };

        let report: RunReport =
            run_block_at_index(ws, &state.hlc, &final_path, flat_idx, &registry, None)
                .map_err(|e| format!("run_block_at_index: {e}"))?;

        let (result_ok, error) = match &report.result {
            Ok(out) => (Some(ExecOutputDto::from(out)), None),
            Err(e) => (None, Some(format!("{e}"))),
        };

        let view = build_page_view(ws, &root, page).map_err(|e| e.to_string())?;
        if error.is_some() {
            warn!("run_code_block runtime error: {:?}", error);
        }
        Ok(RunCodeBlockReply {
            language: report.language.clone(),
            result_ok,
            error,
            view,
        })
    })
}

/// Resolve a block's `NodeId` to its flat DFS index inside the
/// projected outline — the same ordering `outl_exec::run_block_at_index`
/// expects (it parses the `.md` and walks `ParsedPage.blocks` in DFS).
///
/// Returns `None` when the id isn't in the outline (foreign page,
/// stale call, etc.).
///
/// `OutlineNode.id` is the ULID-as-`String` projected for the wire,
/// so we stringify the target once before comparing.
fn flat_index_for(outline: &[OutlineNode], target: NodeId) -> Option<usize> {
    let target_str = target.to_string();
    fn walk(nodes: &[OutlineNode], target: &str, counter: &mut usize) -> Option<usize> {
        for n in nodes {
            if n.id == target {
                return Some(*counter);
            }
            *counter += 1;
            if let Some(hit) = walk(&n.children, target, counter) {
                return Some(hit);
            }
        }
        None
    }
    let mut counter = 0usize;
    walk(outline, &target_str, &mut counter)
}
