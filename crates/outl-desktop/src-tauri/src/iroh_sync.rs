//! iroh P2P transport wiring for the desktop client.
//!
//! The build + start + reload-bridge machinery lives in
//! `outl_tauri_shared::iroh_sync`; this module keeps what is genuinely
//! desktop:
//!
//! - **Where the identity lives.** `~/.outl/identity.key` — the same
//!   per-device path the CLI / TUI use, so every client on the machine
//!   advertises one node id.
//! - **Which event signals a reload.** `peer-ops-changed` — the same
//!   event the `notify` watcher (`fs_watcher.rs`) emits, so the frontend
//!   reload path is reused verbatim whichever delivery path wins.
//! - **Where the transport lands.** The swap-capable `AppState` slots:
//!   one `dyn SyncTransport` clone (announce / shutdown / peer-health)
//!   and one concrete clone for the pairing commands.
//!
//! ## Best-effort
//!
//! Every failure here (no `$HOME`, unreadable identity, transport build
//! error) is logged and swallowed. Sync degrades to the filesystem
//! watcher; the editor keeps working. iroh is never allowed to block or
//! abort the boot path.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::SyncTransport;
use outl_config::SyncTransportKind;
use outl_core::id::ActorId;
use outl_tauri_shared::iroh_sync::{build_iroh_transport, start_with_reload_bridge};
use parking_lot::Mutex;
use tauri::AppHandle;
use tracing::{info, warn};

/// `~/.outl` — the shared device-state directory (identity + peers),
/// the same path the CLI / TUI / pairing commands read.
pub(crate) fn outl_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".outl"))
}

/// Wire the iroh transport into the running app when the config asks
/// for it. Called from the boot opener (and from `set_workspace` on a
/// swap) once the workspace root is known — the transport needs the
/// root to write peer ops into `<root>/ops/`.
///
/// On success, stores the transport in `slot` (so the announce /
/// shutdown / peer-health paths can reach it) and in `pairing_slot`
/// (the concrete clone the pairing commands need), and spawns the
/// bridge thread that turns the transport's "peer ops landed" signal
/// into the `peer-ops-changed` event.
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
    let transport = match build_desktop_transport(&workspace_root) {
        Ok(t) => t,
        Err(e) => {
            warn!("iroh sync unavailable, using filesystem watcher: {e}");
            return;
        }
    };

    // Bridge the transport's "peer ops landed" signal to the SAME event
    // the `notify` watcher emits so the frontend reload path is reused.
    start_with_reload_bridge(&transport, workspace_root, actor, app, "peer-ops-changed");

    // Keep the concrete clone for pairing (reuses the live endpoint) and the
    // `dyn` clone for announce / shutdown / peer_health. `IrohSyncTransport`
    // is `Clone` (internally `Arc`-backed), so both handles drive the one
    // running transport.
    *pairing_slot.lock() = Some(transport.clone());
    *slot.lock() = Some(Arc::new(transport) as Arc<dyn SyncTransport>);
    info!("iroh sync transport wired");
}

/// Resolve `~/.outl/identity.key` and build the transport against the
/// per-workspace peer store — the desktop flavour of the shared
/// `build_iroh_transport`.
fn build_desktop_transport(
    workspace_root: &std::path::Path,
) -> anyhow::Result<outl_sync_iroh::IrohSyncTransport> {
    let dir = outl_dir().ok_or_else(|| anyhow::anyhow!("$HOME unset; cannot locate ~/.outl"))?;
    std::fs::create_dir_all(&dir)?;
    build_iroh_transport(&dir.join("identity.key"), workspace_root)
}
