//! Folder selection for the mobile workspace.
//!
//! ## Why this module exists
//!
//! Storage is a folder on this device, synced by iroh (no iCloud — see
//! `workspace_open` module doc). This module owns the *picking*: persist a
//! chosen path and hand it back so the app reopens against it.
//!
//! ## What is implemented now
//!
//! - [`set_workspace`] — the Tauri command the frontend calls once it has
//!   a folder path. It validates the path, persists it as
//!   `WorkspaceCfg.last` (so the next launch reopens it), and asks the app
//!   to reopen. The actual reopen is **boot-read** today: the boot path in
//!   `lib.rs` reads `WorkspaceCfg.last` and opens it
//!   ([`workspace_open::resolve_storage_root`]). `set_workspace` therefore
//!   persists + emits `workspace-reopen-required` so the frontend can
//!   prompt a relaunch (or a future runtime swap can hook the same event).
//!   This keeps the mobile `AppState.storage_root` single-root invariant
//!   intact — swapping it live would mean turning it into an
//!   `Arc<Mutex<Option<PathBuf>>>` and rebinding the iroh transport
//!   mid-flight, a separate change.
//!
//! ## What is DEFERRED (native picker UI + security-scoped bookmark)
//!
//! The native `UIDocumentPickerViewController` (folder mode) bridge is
//! **not** implemented here yet, and it must not be faked. Two real
//! blockers, confirmed against the Tauri 2 iOS state of the art:
//!
//! 1. **Tauri's own folder picker on iOS is incomplete** — `tauri-plugin-dialog`'s
//!    folder-open path opens the wrong controller and does not return a
//!    security-scoped handle (tauri-apps/plugins-workspace#3030). So we
//!    can't simply call the plugin and trust the result.
//!
//! 2. **Security-scoped bookmarks are required for persistence.** A path
//!    returned by `UIDocumentPickerViewController` for a folder *outside*
//!    the app sandbox is only accessible while the security scope is
//!    held. To reopen it on the next launch the app must store an
//!    `NSURL` bookmark (`bookmarkData(options: .minimalBookmark)`) and
//!    resolve it with `startAccessingSecurityScopedResource()`. Storing
//!    only the string path (what `WorkspaceCfg.last` holds) is enough for
//!    a folder *inside* the sandbox, but **not** for an arbitrary
//!    Files-app folder.
//!
//! ### The deferred native piece, concretely
//!
//! A follow-up adds an ObjC/`objc2` bridge that:
//!   - presents `UIDocumentPickerViewController(forOpeningContentTypes: [.folder])`
//!     from the root view controller on the main thread,
//!   - in its delegate, calls `startAccessingSecurityScopedResource()`,
//!     serialises a bookmark via `bookmarkData(...)`, and persists the
//!     bookmark bytes next to the `actor` file,
//!   - on boot, resolves the bookmark back to a live security-scoped URL
//!     before `resolve_storage_root` runs.
//!
//! Until that lands, [`set_workspace`] works for any path the frontend can
//! already reach without a scoped bookmark (the app sandbox, the local
//! default).

use std::path::{Path, PathBuf};

use tauri::{Emitter, State};
use tracing::{info, warn};

use crate::state::AppState;
use crate::workspace_open::persist_workspace_path;

/// Event emitted after a successful [`set_workspace`] so the frontend
/// knows the chosen folder is persisted and the workspace must be
/// reopened against it.
///
/// Boot-read model: the next launch picks up `WorkspaceCfg.last`. A
/// future runtime-swap implementation can listen for the same event and
/// rebind in place instead of asking for a relaunch.
const REOPEN_EVENT: &str = "workspace-reopen-required";

/// Persist a user-chosen folder as the workspace and request a reopen.
///
/// Called by the frontend once it has a folder path (from a future native
/// picker or a manual entry). The path is taken verbatim — *where* the
/// folder lives is the user's choice, not ours.
///
/// Best-effort persistence: a config write failure is logged, not fatal.
///
/// NOTE: this command must be in the `invoke_handler!` list in `lib.rs` to
/// be callable.
#[tauri::command]
#[allow(dead_code)] // Caller arrives with the deferred native folder picker.
pub(crate) fn set_workspace(
    path: String,
    app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let path = PathBuf::from(&path);
    validate_pickable(&path)?;

    // Create the directory if the user picked a fresh folder. Failure
    // here is fatal to the pick (we can't open what we can't create).
    std::fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;

    persist_workspace_path(&path);
    info!("workspace folder chosen: {}", path.display());

    if let Err(e) = app.emit(REOPEN_EVENT, path.to_string_lossy().into_owned()) {
        warn!("emit {REOPEN_EVENT}: {e}");
    }
    Ok(())
}

/// Guard a path before we accept it as a workspace.
///
/// Rejects the obviously-wrong (empty, relative) so a malformed pick
/// fails loudly instead of silently creating a workspace in the process
/// cwd. We deliberately do **not** require the path to already exist —
/// the user may pick a brand-new folder name.
fn validate_pickable(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("workspace path is empty".to_string());
    }
    if !path.is_absolute() {
        return Err(format!(
            "workspace path must be absolute, got {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_rejected() {
        assert!(validate_pickable(Path::new("")).is_err());
    }

    #[test]
    fn relative_path_is_rejected() {
        assert!(validate_pickable(Path::new("notes/outl")).is_err());
    }

    #[test]
    fn absolute_path_is_accepted_even_if_missing() {
        // A fresh folder name the user typed/picked is valid — we create
        // it on accept. Use a path that won't exist so the test asserts
        // the "missing is fine" branch.
        assert!(validate_pickable(Path::new("/nonexistent/outl-ws-xyz")).is_ok());
    }
}
