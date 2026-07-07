//! Desktop boot opener + background reconcile.
//!
//! The open / actor-id / orphan-reconcile primitives live in
//! `outl_tauri_shared::workspace_open` (re-exported below). This module
//! keeps the two desktop-specific orchestrations:
//!
//! - [`spawn_workspace_opener`] — background thread, run at boot when
//!   `settings.last_workspace` is already set, so the WebView starts
//!   painting before we touch the filesystem. Starts the FS watcher and
//!   wires the iroh transport into the swap-capable `AppState` slots.
//! - [`spawn_background_reconcile`] — the orphan-md pass on a worker
//!   thread, lock released between pages, so the first paint is never
//!   blocked (mobile runs the same pass inline instead).

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use outl_actions::SyncTransport;
use outl_config::SyncTransportKind;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tauri::Emitter;
use tracing::{info, warn};

use crate::fs_watcher::{self, WatcherHandle};

pub(crate) use outl_tauri_shared::workspace_open::{load_or_create_actor, open_workspace_at};

/// Background reconcile pass for pages whose `.md` is ahead of the
/// op log: pages authored via vim, pulled from peers without a
/// sidecar, imported by `outl serve`, etc. Run as a separate worker
/// thread so the first paint is never blocked.
///
/// Each page is reconciled under a lock that is **released between
/// iterations** so the frontend can read the workspace (build the
/// outline, render the picker, run autocomplete queries) without
/// waiting for the whole batch to finish. Pages that just got
/// materialised become visible to the next read, no full reload
/// required.
///
/// Emits `workspace-reconciled` when the batch completes so a client
/// that wants to refresh the current view can do so explicitly. The
/// event fires only on completion of the batch, not per-page —
/// keystroke-grained refreshes would be noisier than they help.
pub(crate) fn spawn_background_reconcile(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    thread::spawn(move || {
        let engine = outl_actions::SyncEngine::new(storage_root.clone(), hlc.actor());
        // Filesystem walk needs no workspace lock.
        let orphans = engine.scan_for_orphans();
        if orphans.is_empty() {
            return;
        }
        info!(
            "background reconcile: {} orphan(s) to process",
            orphans.len()
        );
        for path in &orphans {
            // Lock per page, drop between iterations so the frontend
            // can grab the workspace between reconciles. A page with
            // hundreds of blocks still runs in well under 50ms, well
            // inside the user's perception threshold.
            let mut slot = workspace_slot.lock();
            let Some(ws) = slot.as_mut() else {
                // Workspace was closed (user picked another) — abort
                // the rest of the batch cleanly.
                return;
            };
            if let Err(e) = outl_md::reconcile::reconcile_md(ws, &hlc, path, None) {
                warn!("orphan reconcile failed for {}: {e}", path.display());
            }
            drop(slot);
        }
        info!("background reconcile complete");
        if let Err(e) = app.emit("workspace-reconciled", ()) {
            warn!("emit workspace-reconciled: {e}");
        }
    });
}

/// Background opener used at boot when `settings.last_workspace` is
/// already set. Runs on a worker thread so the WebView starts
/// painting immediately; the frontend polls `workspace_stats` /
/// listens for the `workspace-ready` event.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_workspace_opener(
    workspace_slot: Arc<Mutex<Option<Workspace>>>,
    storage_root_slot: Arc<Mutex<Option<PathBuf>>>,
    fs_watcher_slot: Arc<Mutex<Option<WatcherHandle>>>,
    iroh_transport_slot: Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    iroh_pairing_slot: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    sync_transport_kind: SyncTransportKind,
    last_workspace: PathBuf,
    hlc: HlcGenerator,
    app: tauri::AppHandle,
) {
    let actor = hlc.actor();
    let lru_cap = outl_config::load().storage.lru_cap;
    thread::spawn(move || {
        if !last_workspace.exists() {
            warn!(
                "last_workspace {} no longer exists; user must re-pick",
                last_workspace.display()
            );
            return;
        }
        // Fast open: ops/, journals/, pages/ exist; today's journal is
        // resolved; legacy blocks moved under it. **No orphan
        // reconcile here** — that runs after we publish the workspace
        // so the user sees today's journal immediately.
        let workspace = match open_workspace_at(actor, &hlc, &last_workspace, lru_cap) {
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
        *storage_root_slot.lock() = Some(last_workspace.clone());
        if let Err(e) = app.emit("workspace-ready", ()) {
            warn!("emit workspace-ready: {e}");
        }
        info!("background workspace opener complete");

        // Wire the iroh P2P transport (best-effort, gated on
        // `[sync] transport = "iroh"`). It runs ALONGSIDE the `notify`
        // watcher started above: both deliver peer ops to `<root>/ops/`
        // and both surface as `peer-ops-changed`, so the frontend
        // reload path is identical whichever wins the race. A `File`
        // config or any build failure is a silent no-op here.
        crate::iroh_sync::wire_iroh_transport(
            sync_transport_kind,
            &iroh_transport_slot,
            &iroh_pairing_slot,
            last_workspace.clone(),
            actor,
            app.clone(),
        );

        // Background reconcile: scan and reconcile orphan `.md` files in yet
        // another thread, releasing the workspace lock between each
        // page. The frontend can already query the workspace; pages
        // that get materialised by the reconcile become visible on
        // the next read (no full reload required). The reconcile
        // emits `workspace-reconciled` on completion if a client
        // wants to explicitly refresh the current view.
        spawn_background_reconcile(workspace_slot, last_workspace, hlc, app);
    });
}
