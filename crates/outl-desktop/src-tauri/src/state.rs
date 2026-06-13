//! Shared state and wire types held by Tauri.
//!
//! `AppState` is created once at `setup` time and managed by Tauri so
//! every command has access via `State<'_, AppState>`. The mutable
//! fields are wrapped in `parking_lot::Mutex` because the user can
//! swap workspaces at runtime and the background opener thread writes
//! into them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{Backlink, HistoryStacks, OutlineNode, PageMeta};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use serde::Serialize;

use crate::fs_watcher::WatcherHandle;
use crate::settings::Settings;

/// Sentinel error returned by workspace-touching commands while the
/// workspace is still being opened (background thread) or while the
/// user hasn't picked one yet. The frontend retries on a short
/// interval — see `App.tsx::refresh`.
pub(crate) const ERR_LOADING: &str = "workspace_loading";

/// Shared mutable state held by Tauri.
pub(crate) struct AppState {
    /// The active workspace. `None` until [`crate::commands::workspace::set_workspace`]
    /// or the background opener completes.
    pub workspace: Arc<Mutex<Option<Workspace>>>,
    /// Filesystem root of the active workspace (parent of `ops/`,
    /// `journals/`, `pages/`). Tracks `workspace` 1:1.
    pub storage_root: Arc<Mutex<Option<PathBuf>>>,
    /// Per-device HLC generator. Static for the process lifetime —
    /// actors identify devices, not workspaces.
    pub hlc: HlcGenerator,
    /// User preferences and last opened workspace path.
    pub settings: Arc<Mutex<Settings>>,
    /// Where `settings.json` and `actor` live
    /// (`app.path().app_config_dir()`).
    pub app_config_dir: PathBuf,
    /// Code-block runtimes built once at startup. Shared between
    /// every `run_code_block` invocation. `Arc` so the command can
    /// clone-and-move into `spawn_blocking` without locking.
    pub registry: Arc<RuntimeRegistry>,
    /// Live filesystem watcher for the active workspace. Dropped
    /// (replaced) when the user switches workspaces; `None` while
    /// the picker is still up.
    pub fs_watcher: Arc<Mutex<Option<WatcherHandle>>>,
    /// Per-page undo / redo stacks of rendered `.md` snapshots
    /// (`outl_actions::history::HistoryStacks`). `finish_in_page_with`
    /// records the pre-mutation render; `undo_page` / `redo_page`
    /// restore one through `outl_actions::restore_page_md` (the
    /// restore is ops in the log, never a rewrite). Fully cleared on
    /// workspace **switch**; on a peer-driven **reload** only the
    /// pages whose projection changed lose their stacks (see
    /// `commands::workspace::reload_workspace`).
    pub history: Mutex<HashMap<NodeId, HistoryStacks<String>>>,
}

/// Returned by the `workspace_stats` command.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkspaceSummary {
    pub blocks: usize,
    pub ops: usize,
    pub actor: String,
    pub storage_root: String,
    /// `true` when a workspace is loaded; `false` while the picker
    /// is still up or the background opener is in flight.
    pub ready: bool,
}

/// Reply shape for every "open page / open journal" command. Bundles
/// the page meta with the outline so the frontend gets everything in
/// one trip.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PageView {
    pub page: PageMeta,
    pub outline: Vec<OutlineNode>,
    pub backlinks: Vec<Backlink>,
    /// Parser recoveries for the page's `.md`. Mirrors the mobile
    /// `PageView.warnings` exactly; `<ParseWarningsBanner />` from
    /// `@outl/shared` consumes it. Empty (or absent) on a clean
    /// file — `skip_serializing_if` keeps the JSON quiet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<outl_md::ParseWarning>,
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
    pub view: PageView,
    pub new_id: String,
}
