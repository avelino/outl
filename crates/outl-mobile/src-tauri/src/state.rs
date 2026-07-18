//! Mobile `AppState` + its [`AppHost`] projection.
//!
//! The wire types (`PageView`, `CreateBlockReply`, `WorkspaceSummary`,
//! `ERR_LOADING`) live in `outl-tauri-shared` and are re-exported here
//! so the rest of the crate keeps importing them from `crate::state`.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_sync_iroh::IrohSyncTransport;
use outl_tauri_shared::AppHost;
use parking_lot::Mutex;

pub(crate) use outl_tauri_shared::{CreateBlockReply, PageView, WorkspaceSummary};

/// Shared mutable state held by Tauri.
///
/// Note: `storage_root` is a plain `PathBuf` (resolved on boot inside
/// `workspace_open::resolve_storage_root`) — unlike the desktop crate,
/// the mobile client always has a workspace path: the folder the user
/// previously chose (`WorkspaceCfg.last`), or the app-local default
/// `<app-data-dir>/outl/` on a fresh install. Don't copy desktop's
/// `Arc<Mutex<Option<PathBuf>>>` shape — the single-root divergence is
/// deliberate and absorbed by the shared `AppHost` trait. Switching
/// folders at runtime is a `workspace-reopen-required` event + relaunch
/// today (see `workspace_picker`); a live swap would be what flips this
/// to the desktop shape.
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
    /// The running iroh P2P sync transport, when wired at boot.
    ///
    /// This is a **lifetime guard**: the transport's `start()` spawns a
    /// detached `outl-iroh-sync` thread that owns the tokio runtime via
    /// internal `Arc`s, but the `shutdown_tx` / `announce_tx` handles
    /// live on this struct. Keeping the value here means the app holds a
    /// handle for its whole lifetime; if we dropped it the transport
    /// would stay running but lose any future `shutdown()` / announce
    /// path. `None` when iroh is disabled in config or failed to bind
    /// (the app then simply runs without P2P sync until a later relaunch
    /// retries).
    ///
    /// Read through `AppHost::sync_transport` by the shared peer-status /
    /// force-sync / announce paths, cloned by the pairing commands, and
    /// shut down gracefully in `Drop`.
    pub(crate) iroh: Option<IrohSyncTransport>,
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Make the "graceful shutdown() on app exit" the field doc promises real.
        // The detached transport thread would die with the process regardless,
        // but shutdown() releases the relay route right away instead of waiting
        // for the OS to reap the socket — so another process on this device
        // reclaims the route immediately.
        if let Some(transport) = &self.iroh {
            transport.shutdown();
        }
    }
}

/// The mobile projection onto the shared command surface: the root is
/// fixed for the process lifetime (`storage_root()` never errors), the
/// concrete transport is wrapped as `Arc<dyn SyncTransport>` on demand,
/// and there is no undo history (the `history()` default of `None`
/// skips snapshot recording entirely).
impl AppHost for AppState {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }

    fn workspace_arc(&self) -> std::sync::Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }

    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }

    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.storage_root.clone())
    }

    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        self.iroh
            .clone()
            .map(|t| Arc::new(t) as Arc<dyn SyncTransport>)
    }

    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
}
