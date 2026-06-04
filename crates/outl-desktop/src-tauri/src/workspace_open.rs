//! Workspace open / reconcile / boot opener primitives.
//!
//! Shared between two callers:
//!
//! - [`crate::commands::workspace::set_workspace`] — synchronous when
//!   the user picks a directory via the dialog.
//! - [`spawn_workspace_opener`] — background thread, run at boot when
//!   `settings.last_workspace` is already set, so the WebView starts
//!   painting before we touch the filesystem.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use outl_actions::{migrate_legacy_into_today, open_today};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tauri::Emitter;
use tracing::{info, warn};

use crate::fs_watcher::{self, WatcherHandle};

/// Open (or create) the workspace rooted at `path`.
///
/// Idempotent: the `ops/`, `journals/`, `pages/` directories are
/// created if missing, and `migrate_legacy_into_today` reshuffles any
/// pre-page-model blocks under today's journal (also idempotent).
pub(crate) fn open_workspace_at(
    actor: ActorId,
    hlc: &HlcGenerator,
    path: &Path,
) -> anyhow::Result<Workspace> {
    std::fs::create_dir_all(path.join("ops"))?;
    std::fs::create_dir_all(path.join("journals"))?;
    std::fs::create_dir_all(path.join("pages"))?;

    let storage = JsonlStorage::open(path.join("ops"), actor)?;
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(path.to_path_buf()))?;

    if let Err(e) = migrate_legacy_into_today(&mut workspace, hlc) {
        warn!("legacy migration: {e}");
    }
    if let Err(e) = open_today(&mut workspace, hlc) {
        warn!("could not pre-open today: {e}");
    }
    reconcile_orphan_md(&mut workspace, hlc, path);
    Ok(workspace)
}

/// Scan `<root>/journals/` and `<root>/pages/` for `.md` files that
/// are not represented in the op log yet — imported files, peer-written
/// projections without sidecars, or files edited externally in vim/VS
/// Code. Runs `reconcile_md` on each so the workspace, the sidecar,
/// and `.md` converge.
pub(crate) fn reconcile_orphan_md(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    storage_root: &Path,
) {
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

/// Load (or generate-and-persist) the device's actor id.
///
/// The actor identifies the device, not the workspace — it's reused
/// across whatever directory the user picks. Lives at
/// `<app_config_dir>/actor` as a plain ULID string.
pub(crate) fn load_or_create_actor(local_dir: &Path) -> std::io::Result<ActorId> {
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

/// Background opener used at boot when `settings.last_workspace` is
/// already set. Runs on a worker thread so the WebView starts
/// painting immediately; the frontend polls `workspace_stats` /
/// listens for the `workspace-ready` event.
pub(crate) fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root_slot: Arc<Mutex<Option<PathBuf>>>,
    fs_watcher_slot: Arc<Mutex<Option<WatcherHandle>>>,
    last_workspace: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    thread::spawn(move || {
        if !last_workspace.exists() {
            warn!(
                "last_workspace {} no longer exists; user must re-pick",
                last_workspace.display()
            );
            return;
        }
        let workspace = match open_workspace_at(actor, &hlc, &last_workspace) {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "background open failed for {}: {e}",
                    last_workspace.display()
                );
                return;
            }
        };
        // Start the FS watcher BEFORE we swap the slot so the first
        // peer change after boot is captured. Failure is logged but
        // non-fatal — the user can still work, just without
        // automatic reload.
        match fs_watcher::start_watcher(&last_workspace, actor, app.clone()) {
            Ok(handle) => fs_watcher::swap_watcher(&fs_watcher_slot, Some(handle)),
            Err(e) => warn!("fs watcher failed to start: {e}"),
        }
        *workspace_slot.lock() = Some(workspace);
        *storage_root_slot.lock() = Some(last_workspace);
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");
    });
}
