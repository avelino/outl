//! iCloud workspace resolution + background opener.
//!
//! All the mobile-specific divergences from the desktop client live
//! here:
//!
//! - The workspace root is always `<ubiquity-container>/Documents/`
//!   (the iCloud Documents folder); the user does not pick a path.
//! - Peer-file materialisation is handled outside Rust (NSMetadataQuery
//!   + NSFileCoordinator in `gen/apple/.../main.mm`).
//! - Orphan `.md` reconcile runs **inline** in the boot path (desktop
//!   spawns a background thread). Mobile workspaces are smaller and
//!   the UI waits on `workspace-ready` anyway, so blocking the opener
//!   keeps the first paint deterministic.

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

use crate::icloud_path;

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

/// iCloud Ubiquity Container identifier registered in the
/// `com.apple.developer.icloud-container-identifiers` entitlement.
const ICLOUD_CONTAINER_ID: &str = "iCloud.app.outl.mobile-app";

pub(crate) fn workspace_root_in(container: &Path) -> PathBuf {
    let mut p = container.to_path_buf();
    for seg in WORKSPACE_SUBDIR {
        p.push(seg);
    }
    p
}

fn ops_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("ops")
}

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

pub(crate) fn resolve_storage_root(local_fallback: &Path) -> PathBuf {
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
pub(crate) fn spawn_workspace_opener(
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
