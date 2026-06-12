//! Shared state held by Tauri across commands.
//!
//! Split out of `lib.rs` so the file-size guard stays happy and every
//! `commands::*` module can `use crate::state::*` without dragging
//! `lib.rs` into the import graph.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{Backlink, OutlineNode, PageMeta};
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use serde::Serialize;

/// Sentinel error returned by every workspace-touching command while
/// the workspace is still being opened on the background thread.
pub(crate) const ERR_LOADING: &str = "workspace_loading";

/// Shared mutable state held by Tauri.
///
/// Note: `storage_root` is a plain `PathBuf` (resolved on boot inside
/// `workspace_open::resolve_storage_root`) — unlike the desktop crate,
/// the mobile client always has a workspace path (iCloud container or
/// the local fallback). Don't copy desktop's
/// `Arc<Mutex<Option<PathBuf>>>` shape — the divergence is deliberate.
pub(crate) struct AppState {
    /// `None` until the background opener completes.
    pub(crate) workspace: Arc<Mutex<Option<Workspace>>>,
    pub(crate) hlc: HlcGenerator,
    pub(crate) storage_root: PathBuf,
    /// Code-block runtimes built once at startup. Shared between
    /// every `run_code_block` invocation. Kept behind `Arc` so future
    /// `spawn_blocking` callers can clone-and-move without holding
    /// the workspace mutex.
    pub(crate) registry: Arc<RuntimeRegistry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkspaceSummary {
    pub(crate) blocks: usize,
    pub(crate) ops: usize,
    pub(crate) actor: String,
    pub(crate) storage_root: String,
    pub(crate) ready: bool,
}

/// Reply shape for every "open page / open journal" command. Bundles
/// the page meta with the outline so the frontend gets everything in
/// one trip.
///
/// `warnings` is the verbatim `outl_md::ParseWarning` list surfaced
/// by `outl_actions::read_page_outline_with_workspace`. The
/// `<ParseWarningsBanner />` from `@outl/shared` reads it; clients
/// don't have to touch the field directly.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PageView {
    pub(crate) page: PageMeta,
    pub(crate) outline: Vec<OutlineNode>,
    pub(crate) backlinks: Vec<Backlink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<outl_md::ParseWarning>,
}

/// Reply for `create_block`. Pairs the refreshed [`PageView`] with the
/// id of the freshly-inserted block so the frontend can focus / start
/// editing it without re-discovering the id via a DFS diff (the diff
/// path mis-identified the new block when the anchor had children
/// — `flat[idx+1]` would land on `children[0]` instead of the new
/// sibling, and the eventual `edit_block` would target a stale id and
/// surface the `block <ULID> is not in the tree` toast).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CreateBlockReply {
    pub(crate) view: PageView,
    pub(crate) new_id: String,
}
