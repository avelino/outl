//! The traits that absorb the desktop ↔ mobile structural divergence.
//!
//! The only real difference between the two `src-tauri` backends is how
//! the storage root is held: the desktop can swap workspaces at runtime
//! (`Arc<Mutex<Option<PathBuf>>>`), the mobile resolves one folder at boot
//! (`PathBuf`; switching is a relaunch). Everything else the shared
//! command bodies need is uniform and exposed through [`AppHost`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{HistoryStacks, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;

/// Implemented by each client's `AppState` so the shared command bodies
/// (generic over `S: AppHost`) can run against either backend.
pub trait AppHost: Send + Sync {
    /// The active workspace slot. `None` until the background opener (or
    /// the user's picker) publishes one.
    fn workspace(&self) -> &Mutex<Option<Workspace>>;

    /// Per-device HLC generator (actors identify devices, not workspaces).
    fn hlc(&self) -> &HlcGenerator;

    /// Filesystem root of the active workspace (parent of `ops/`,
    /// `journals/`, `pages/`). Errors with the [`crate::ERR_LOADING`]
    /// sentinel while unresolved (desktop before a workspace is picked);
    /// infallible on clients that resolve the root at boot (mobile).
    fn storage_root(&self) -> Result<PathBuf, String>;

    /// The live P2P sync transport, when wired. Used by the post-commit
    /// announce and by the peer status / force-sync commands. `None` for
    /// the file transport (or before the transport comes up).
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>>;

    /// Code-block runtimes built once at startup.
    fn exec_registry(&self) -> Arc<RuntimeRegistry>;

    /// Per-page undo / redo stacks, when the client supports block-level
    /// undo (desktop). `None` (the default) skips snapshot recording in
    /// [`crate::helpers::finish_in_page_with`] entirely — no per-mutation
    /// render is paid on clients without undo.
    fn history(&self) -> Option<&Mutex<HashMap<NodeId, HistoryStacks<String>>>> {
        None
    }
}

/// Owned handle to "the current storage root" the plugin thread can move
/// into itself. Mirrors the [`AppHost::storage_root`] divergence for code
/// that runs off the Tauri state (the dedicated plugin thread).
pub trait StorageRootProvider: Send + 'static {
    /// The root right now; `None` while no workspace is open. A provider
    /// whose value can change between calls (the desktop's swap-capable
    /// slot) makes the plugin host reload against the new root.
    fn current(&self) -> Option<PathBuf>;
}

/// Mobile: one root for the process lifetime.
impl StorageRootProvider for PathBuf {
    fn current(&self) -> Option<PathBuf> {
        Some(self.clone())
    }
}

/// Desktop: the same swap-capable slot `AppState` holds.
impl StorageRootProvider for Arc<Mutex<Option<PathBuf>>> {
    fn current(&self) -> Option<PathBuf> {
        self.lock().clone()
    }
}
