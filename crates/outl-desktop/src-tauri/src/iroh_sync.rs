//! iroh P2P transport wiring for the desktop client.
//!
//! Gated on `[sync] transport = "iroh"` in the global
//! `~/.config/outl/config.toml` (read via `outl_config::load()`). When
//! the user opts in, [`wire_iroh_transport`] builds an
//! [`outl_sync_iroh::IrohSyncTransport`] from the on-disk device
//! identity (`~/.outl/identity.key`) and peer store
//! (`~/.outl/peers.json`) — the same `~/.outl/` location the CLI, the
//! TUI, and the pairing commands use — and starts it.
//!
//! ## Relationship to the `notify` file watcher
//!
//! The cross-platform `notify` watcher (`fs_watcher.rs`) STAYS in every
//! configuration. It is the universal "a peer's `ops-*.jsonl` landed on
//! a shared folder" detector (iCloud Drive, Dropbox, Syncthing). iroh
//! runs **alongside** it: iroh receives ops over QUIC and writes them to
//! the same local `ops/` directory, so a peer write is observed by
//! whichever path delivers it first. Both end up emitting the SAME
//! `peer-ops-changed` event the frontend already listens for, so the
//! reload path (`onPeerOpsChanged` → `reload_workspace`) is reused
//! verbatim — no new frontend wiring.
//!
//! ## Best-effort
//!
//! Every failure here (no `$HOME`, unreadable identity, transport build
//! error) is logged and swallowed. Sync degrades to the filesystem
//! watcher; the editor keeps working. iroh is never allowed to block or
//! abort the boot path.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_config::SyncTransportKind;
use outl_core::id::ActorId;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

/// `~/.outl` — the shared device-state directory (identity + peers),
/// the same path the CLI / TUI / pairing commands read.
pub(crate) fn outl_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".outl"))
}

/// Build an [`outl_sync_iroh::IrohSyncTransport`] from the device's
/// on-disk identity and peer store under `~/.outl/`.
///
/// Returns the **concrete** transport (cheaply `Clone`, internally
/// `Arc`-backed). The caller stores one clone as the concrete
/// `iroh_pairing` slot (so the pairing commands can call `pair_host` /
/// `pair_join`, which reuse the live sync endpoint) and one as the
/// `dyn SyncTransport` slot (`announce` / `shutdown` / `peer_health`).
fn build_iroh_transport() -> anyhow::Result<outl_sync_iroh::IrohSyncTransport> {
    let dir = outl_dir().ok_or_else(|| anyhow::anyhow!("$HOME unset; cannot locate ~/.outl"))?;
    std::fs::create_dir_all(&dir)?;
    let identity = outl_sync_iroh::IrohIdentity::load_or_generate(&dir.join("identity.key"))?;
    let peers = outl_sync_iroh::PeersStore::load_or_default(&dir.join("peers.json"))?;
    Ok(outl_sync_iroh::IrohSyncTransport::new(identity, peers))
}

/// Wire the iroh transport into the running app when the config asks
/// for it. Called from the boot opener once the workspace root is
/// known (the transport needs the root to write peer ops into
/// `<root>/ops/`).
///
/// On success, stores the transport in `slot` (so the pairing /
/// announce / shutdown commands can reach it) and spawns the bridge
/// thread that turns the transport's "peer ops landed" signal into the
/// `peer-ops-changed` event.
///
/// Returns silently (a no-op) when `transport != Iroh` or any step
/// fails — the filesystem watcher already covers detection.
pub(crate) fn wire_iroh_transport(
    transport_kind: SyncTransportKind,
    slot: &Arc<Mutex<Option<Arc<dyn SyncTransport>>>>,
    pairing_slot: &Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>>,
    workspace_root: PathBuf,
    actor: ActorId,
    app: AppHandle,
) {
    if transport_kind != SyncTransportKind::Iroh {
        return;
    }
    let transport = match build_iroh_transport() {
        Ok(t) => t,
        Err(e) => {
            warn!("iroh sync unavailable, using filesystem watcher: {e}");
            return;
        }
    };

    // The transport fires `()` on this channel whenever peer ops have
    // been written to local `ops/`. Bridge it to the SAME event the
    // `notify` watcher emits so the frontend reload path is reused.
    let (peer_ready_tx, peer_ready_rx) = mpsc::channel::<()>();
    transport.start(workspace_root, actor, peer_ready_tx);

    std::thread::Builder::new()
        .name("outl-iroh-bridge".into())
        .spawn(move || {
            // Recv blocks until the transport signals or the sender is
            // dropped (transport shut down). Either way the loop ends
            // cleanly when the channel disconnects.
            while peer_ready_rx.recv().is_ok() {
                if let Err(e) = app.emit("peer-ops-changed", ()) {
                    warn!("emit peer-ops-changed (iroh): {e}");
                }
            }
            info!("iroh peer-ready bridge ended");
        })
        .expect("spawning the iroh peer-ready bridge thread should not fail");

    // Keep the concrete clone for pairing (reuses the live endpoint) and the
    // `dyn` clone for announce / shutdown / peer_health. `IrohSyncTransport`
    // is `Clone` (internally `Arc`-backed), so both handles drive the one
    // running transport.
    *pairing_slot.lock() = Some(transport.clone());
    *slot.lock() = Some(Arc::new(transport) as Arc<dyn SyncTransport>);
    info!("iroh sync transport wired");
}
