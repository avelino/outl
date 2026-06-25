//! iroh P2P sync wiring for the mobile client.
//!
//! Owns three mobile-specific concerns the desktop/TUI keep elsewhere:
//!
//! - **Where the device identity lives.** Unlike the TUI (which uses
//!   `~/.outl/`), iOS has no meaningful home directory — the sandbox
//!   `$HOME` is per-install and opaque. We resolve `identity.key` from
//!   the Tauri **app local data dir** so it sits next to the persisted
//!   `actor` ULID and survives across launches. [`iroh_dir`] is the
//!   single source of that per-device path. The identity is per-DEVICE
//!   (one node id per install), never per-graph.
//!
//! - **Where the peer list lives.** The paired-peer list is per-GRAPH,
//!   so it lives at `<workspace_root>/.outl/peers.json` (via
//!   [`outl_sync_iroh::workspace_peers_path`]) — NOT next to the
//!   identity. The running transport and the pairing commands both
//!   resolve it from the workspace root, so a freshly paired device shows
//!   up in `outl_peer_list` and syncs after the next launch.
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
use outl_sync_iroh::{
    migrate_global_peers_if_absent, workspace_peers_path, IrohIdentity, IrohSyncTransport,
    PeersStore,
};
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

/// Per-GRAPH peers file: `<workspace_root>/.outl/peers.json`.
///
/// Takes the **workspace root** (not the per-device `iroh_dir`): the peer
/// list belongs to the graph, so it moves with the workspace, while the
/// device identity stays in `iroh_dir`.
pub(crate) fn peers_path(workspace_root: &std::path::Path) -> PathBuf {
    workspace_peers_path(workspace_root)
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
    // Peer list is per-GRAPH: read it from `<workspace_root>/.outl/peers.json`,
    // migrating any legacy global list on first open. Identity above stays in
    // the per-device `iroh_dir`.
    migrate_global_peers_if_absent(&workspace_root);
    let peers = match PeersStore::load_or_default(&peers_path(&workspace_root)) {
        Ok(p) => p,
        Err(e) => {
            warn!("iroh disabled: peers load: {e}");
            return None;
        }
    };

    // `[sync] relay_url` from the global config: `None` (or empty) keeps iroh's
    // n0 default relay, `Some(url)` points the sync endpoint at a custom relay.
    let relay_url = outl_config::load().sync.relay_url().map(str::to_string);
    let transport = IrohSyncTransport::new(identity, peers, relay_url);

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
