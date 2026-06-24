//! iroh P2P sync wiring for the mobile client.
//!
//! Owns three mobile-specific concerns the desktop/TUI keep elsewhere:
//!
//! - **Where the device identity + peer store live.** Unlike the TUI
//!   (which uses `~/.outl/`), iOS has no meaningful home directory — the
//!   sandbox `$HOME` is per-install and opaque. We resolve the iroh
//!   files from the Tauri **app local data dir** so they sit next to the
//!   persisted `actor` ULID and survive across launches. [`iroh_dir`] is
//!   the single source of that path; the pairing commands in
//!   `commands::peers` resolve through it too, so the running transport
//!   and the pairing handshake always touch the same `peers.json`.
//!
//! - **Whether to wire iroh at all.** Driven by the `[sync]` section of
//!   the global `outl-config`. iroh is the default transport on mobile
//!   (P2P is the whole point of the companion app); we only fall back to
//!   the iCloud file transport when the config explicitly says
//!   `transport = "file"`.
//!
//! - **Resilience.** Any failure binding the identity / peer store logs
//!   and returns `None`. The app keeps running on the iCloud file
//!   transport (the native `NSMetadataQuery` watcher in `main.mm`), so a
//!   broken iroh setup never crashes startup.

use std::path::PathBuf;
use std::sync::mpsc;

use outl_actions::SyncTransport;
use outl_config::SyncTransportKind;
use outl_core::id::ActorId;
use outl_sync_iroh::{IrohIdentity, IrohSyncTransport, PeersStore};
use tauri::{Emitter, Manager};
use tracing::{info, warn};

/// Directory holding the device's iroh identity + peer store.
///
/// Lives under the Tauri app local data dir (the same place the `actor`
/// ULID is persisted) — never the iCloud container. The device identity
/// is per-install and must not sync between devices: two devices sharing
/// one `identity.key` would advertise the same node id and collide.
pub(crate) fn iroh_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("app local data dir: {e}"))?;
    Ok(dir)
}

pub(crate) fn identity_path(dir: &std::path::Path) -> PathBuf {
    dir.join("identity.key")
}

pub(crate) fn peers_path(dir: &std::path::Path) -> PathBuf {
    dir.join("peers.json")
}

/// Build + start the iroh transport when the config asks for it.
///
/// Returns the live [`IrohSyncTransport`] so the caller can stash it in
/// `AppState` (keeping its background tokio runtime alive). Returns
/// `None` when:
///
/// - the config selects the file transport (`transport = "file"`), or
/// - the app local data dir / identity / peer store can't be resolved
///   (logged, never fatal).
///
/// On success it spawns the transport (`transport.start`) and a small
/// drain thread that turns the transport's "peer ops landed" signal into
/// an `app.emit("workspace-ready", ())` — the same event the background
/// opener fires, which the frontend already listens for to reload the
/// current view. (The iCloud path uses the native `__outlOpsChanged`
/// bridge; iroh has no native watcher, so it reuses the Tauri event.)
pub(crate) fn wire_iroh_transport(
    app: &tauri::AppHandle,
    workspace_root: PathBuf,
    actor: ActorId,
    transport_kind: SyncTransportKind,
) -> Option<IrohSyncTransport> {
    if transport_kind == SyncTransportKind::File {
        info!("sync transport = file; iroh disabled by config");
        return None;
    }

    let dir = match iroh_dir(app) {
        Ok(d) => d,
        Err(e) => {
            warn!("iroh disabled: {e}");
            return None;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("iroh disabled: create {}: {e}", dir.display());
        return None;
    }

    let identity = match IrohIdentity::load_or_generate(&identity_path(&dir)) {
        Ok(id) => id,
        Err(e) => {
            warn!("iroh disabled: identity load: {e}");
            return None;
        }
    };
    let peers = match PeersStore::load_or_default(&peers_path(&dir)) {
        Ok(p) => p,
        Err(e) => {
            warn!("iroh disabled: peers load: {e}");
            return None;
        }
    };

    let transport = IrohSyncTransport::new(identity, peers);

    // The transport fires `()` on this channel whenever peer ops land on
    // disk. Bridge each signal to the `workspace-ready` Tauri event so
    // the frontend reloads — mirroring the iCloud `__outlOpsChanged`
    // path's effect without a native watcher.
    let (tx, rx) = mpsc::channel::<()>();
    transport.start(workspace_root, actor, tx);

    let app_for_drain = app.clone();
    std::thread::Builder::new()
        .name("outl-iroh-reload".into())
        .spawn(move || {
            // Blocks until the transport drops its sender (shutdown) or
            // forever while the app runs. Each `recv` is a peer-op event.
            while rx.recv().is_ok() {
                if let Err(e) = app_for_drain.emit("workspace-ready", ()) {
                    warn!("emit workspace-ready (iroh peer ops): {e}");
                }
            }
            info!("iroh reload bridge thread exiting");
        })
        .expect("spawn outl-iroh-reload thread");

    info!("iroh sync transport started");
    Some(transport)
}
