//! Mobile workspace resolution + background opener.
//!
//! The open / actor-id / orphan-reconcile primitives live in
//! `outl_tauri_shared::workspace_open` (re-exported below). This module
//! keeps what is genuinely mobile:
//!
//! ## Storage is a local folder, synced by iroh (no iCloud)
//!
//! The workspace root is a folder on this device; iroh P2P is the only
//! sync. A fresh install works with **no iCloud at all**: the default
//! root is `<app-data-dir>/outl/` and iroh ships the op log to paired
//! devices.
//!
//! Resolution order in [`resolve_storage_root`]:
//!
//! 1. The persisted `WorkspaceCfg.last` path, when present and usable
//!    (survives restarts — written by [`persist_workspace_path`]).
//! 2. The app-local default `<app-data-dir>/outl/`.
//!
//! Picking an arbitrary folder is the deferred native-picker concern (see
//! `workspace_picker`); until that lands the default local root is the
//! only path a fresh install ever opens.
//!
//! ## Other mobile divergences from desktop
//!
//! - Orphan `.md` reconcile runs **inline** in the boot path (desktop
//!   spawns a background thread). Mobile workspaces are smaller and
//!   the UI waits on `workspace-ready` anyway, so blocking the opener
//!   keeps the first paint deterministic.
//! - There is no filesystem watcher: a single device needs none, and
//!   peer ops arrive through iroh, which pokes the reload itself (see
//!   `iroh_sync::wire_iroh_transport`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tauri::Emitter;
use tracing::{info, warn};

pub(crate) use outl_tauri_shared::workspace_open::{
    load_or_create_actor, open_workspace_at, reconcile_orphan_md,
};

/// Folder name for the **local** default workspace, created under the
/// app's data dir when the user hasn't picked anything yet.
///
/// A fresh install lands here — iroh is the sync.
const LOCAL_WORKSPACE_DIR: &str = "outl";

/// The app-local default workspace root: `<app-data-dir>/outl/`.
///
/// This is what a fresh install uses — iroh syncs it P2P.
pub(crate) fn local_default_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOCAL_WORKSPACE_DIR)
}

/// Resolve the workspace root to open on boot.
///
/// Order:
///
/// 1. A previously chosen folder, if `persisted` points at something
///    usable (the user picked it; it survived this restart).
/// 2. The app-local default `<app-data-dir>/outl/` — a fresh install,
///    synced by iroh.
///
/// `persisted` is `WorkspaceCfg.last` from `outl-config`. A `None` (first
/// launch) or a path that no longer resolves falls through to the local
/// default.
pub(crate) fn resolve_storage_root(app_data_dir: &Path, persisted: Option<&Path>) -> PathBuf {
    if let Some(chosen) = persisted {
        // Accept the chosen path as long as its parent exists (the
        // workspace dir itself is created by the caller). A path whose
        // parent vanished — e.g. a removed external volume — falls through
        // to the local default instead of failing the boot.
        let usable = chosen.exists() || chosen.parent().map(|p| p.exists()).unwrap_or(false);
        if usable {
            info!("opening chosen workspace at {}", chosen.display());
            return chosen.to_path_buf();
        }
        warn!(
            "chosen workspace {} no longer resolves; using local default",
            chosen.display()
        );
    }

    let default_root = local_default_root(app_data_dir);
    info!(
        "no workspace chosen; using local default {}",
        default_root.display()
    );
    default_root
}

/// Persist the chosen workspace path so the next launch reopens it
/// instead of the default.
///
/// Writes `WorkspaceCfg.last` through `outl-config` (the single config
/// reader/writer — never hand-edit the TOML). Best-effort: a failure is
/// logged, never fatal, because the workspace is already open in memory.
#[allow(dead_code)] // Wired by the folder picker; see workspace_picker.rs.
pub(crate) fn persist_workspace_path(path: &Path) {
    let mut cfg = outl_config::load();
    if cfg.workspace.last.as_deref() == Some(path) {
        return;
    }
    cfg.workspace.last = Some(path.to_path_buf());
    if let Err(e) = outl_config::save(&cfg) {
        warn!("could not persist chosen workspace {}: {e}", path.display());
    }
}

/// Background opener. Runs once per process; sets the inner
/// `Option<Workspace>` and emits the `workspace-ready` event when done.
///
/// Unlike the desktop opener, the orphan-md reconcile runs **inline**
/// here (before publishing the workspace) so the very first
/// `build_page_view` call already sees imported / peer-written blocks —
/// e.g. backlinks on today's journal include yesterday's imports.
pub(crate) fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    thread::spawn(move || {
        let mut workspace = match open_workspace_at(actor, &hlc, &storage_root) {
            Ok(w) => w,
            Err(e) => {
                warn!("background open failed for {}: {e}", storage_root.display());
                return;
            }
        };
        // Reconcile any `.md` files the op log doesn't know about yet —
        // imported journals (Roam dump, Logseq move), peer-written `.md`
        // that arrived without its sidecar, or files edited externally
        // in vim / VS Code.
        reconcile_orphan_md(&mut workspace, &hlc, &storage_root);
        *workspace_slot.lock() = Some(workspace);
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");
    });
}
