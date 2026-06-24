//! Workspace resolution + background opener.
//!
//! ## Storage is a chosen folder, not forced iCloud (Fase 2)
//!
//! The workspace root is a folder the user picks; it may live anywhere
//! — the app's local data dir (the default), the Files app, or inside an
//! iCloud container. iroh P2P is the primary sync, so the storage
//! *location* is a choice, not a hard iCloud dependency. A fresh install
//! works with **no iCloud at all**: the default root is
//! `<app-data-dir>/outl/` and iroh syncs it.
//!
//! Resolution order in [`resolve_storage_root`]:
//!
//! 1. The persisted `WorkspaceCfg.last` path, when present and usable
//!    (survives restarts — written by [`persist_workspace_path`]).
//! 2. The app-local default `<app-data-dir>/outl/` (zero iCloud).
//!
//! iCloud stays available as a *destination* the user can point the
//! folder at (via [`icloud_path::resolve_container`] +
//! [`icloud_container_workspace_root`]), but it is never the forced
//! default.
//!
//! ## Change detection is generic, not iCloud-only
//!
//! The iroh transport signals reloads when it writes peer ops (see
//! `iroh_sync::wire_iroh_transport`). The iCloud-only `NSMetadataQuery`
//! watcher (`OutlOpsWatcher.swift`) is now **conditional** on the chosen
//! folder being inside iCloud ([`icloud_path::is_inside_icloud`]): a local
//! folder relies on the iroh signal and does not require the iCloud
//! daemon. The Rust boot path surfaces that decision via
//! [`storage_is_icloud`] so the native side and the JS bridge agree.
//!
//! ## Other mobile divergences from desktop
//!
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

/// Folder name for the **local** default workspace, created under the
/// app's data dir when the user hasn't picked anything yet.
///
/// A fresh install lands here with no iCloud involved — iroh is the sync.
const LOCAL_WORKSPACE_DIR: &str = "outl";

/// Sub-path under the iCloud Ubiquity Container where the workspace
/// lives **when the user chooses to store it in iCloud**.
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
///
/// Only consulted when the user opts in to iCloud storage; the default
/// path never touches it.
const ICLOUD_CONTAINER_ID: &str = "iCloud.app.outl.mobile-app";

/// The workspace root inside an iCloud container (`<container>/Documents`).
///
/// Kept as the opt-in iCloud destination. Renamed from the old
/// `workspace_root_in` (which `lib.rs` used to call unconditionally) to
/// make the iCloud-specific intent obvious at the call site.
pub(crate) fn icloud_container_workspace_root(container: &Path) -> PathBuf {
    let mut p = container.to_path_buf();
    for seg in WORKSPACE_SUBDIR {
        p.push(seg);
    }
    p
}

/// The app-local default workspace root: `<app-data-dir>/outl/`.
///
/// This is what a fresh install uses with no iCloud — iroh syncs it P2P.
pub(crate) fn local_default_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOCAL_WORKSPACE_DIR)
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

/// Resolve the workspace root to open on boot.
///
/// **No longer forces iCloud.** Order:
///
/// 1. A previously chosen folder, if `persisted` points at something
///    usable (the user picked it; it survived this restart). The path is
///    taken verbatim — it may be local, in the Files app, or inside an
///    iCloud container; we don't second-guess where the user put it.
/// 2. The app-local default `<app-data-dir>/outl/` — a fresh install with
///    **no iCloud at all**, synced by iroh.
///
/// `persisted` is `WorkspaceCfg.last` from `outl-config`. A `None` (first
/// launch) or a path that no longer resolves falls through to the local
/// default rather than reaching for iCloud.
pub(crate) fn resolve_storage_root(app_data_dir: &Path, persisted: Option<&Path>) -> PathBuf {
    if let Some(chosen) = persisted {
        // Accept the chosen path as long as its parent exists (the
        // workspace dir itself is created by the caller). A path whose
        // parent vanished — e.g. an iCloud container the user signed out
        // of, or a removed external volume — falls through to the local
        // default instead of failing the boot.
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

/// Resolve the app's iCloud container workspace root, if the user wants to
/// store the workspace in iCloud and the container is available.
///
/// Opt-in only — the folder picker (or a future "store in iCloud" toggle)
/// calls this; the boot path does **not**. Returns `None` when the user
/// isn't signed into iCloud or the entitlement is missing, so the caller
/// can fall back to a local folder cleanly.
#[allow(dead_code)] // Wired by the folder picker; see workspace_picker.rs.
pub(crate) fn icloud_workspace_root() -> Option<PathBuf> {
    let container = icloud_path::resolve_container(ICLOUD_CONTAINER_ID)?;
    info!("iCloud container available at {}", container.display());
    Some(icloud_container_workspace_root(&container))
}

/// Whether the chosen workspace folder lives inside iCloud.
///
/// Gates the iCloud-only change detector: `true` means the
/// `NSMetadataQuery` watcher (`OutlOpsWatcher.swift`) + `NSFileCoordinator`
/// materialisation are relevant; `false` means a local folder that relies
/// on the iroh reload signal and must not require the iCloud daemon.
pub(crate) fn storage_is_icloud(storage_root: &Path) -> bool {
    icloud_path::is_inside_icloud(storage_root)
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
