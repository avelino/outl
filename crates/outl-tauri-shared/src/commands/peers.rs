//! Peer / device management command bodies.
//!
//! These are the only bodies that bypass the workspace lock: the peer
//! list is sync-transport state, not workspace state, so they read and
//! edit `<workspace>/.outl/peers.json` directly via
//! `outl_sync_iroh::PeersStore`.
//!
//! **Pairing stays client-side.** `outl_peer_pair_host` returns a
//! `PairedPeerDto` on the desktop but the raw ticket `String` on mobile
//! (established wire contracts); unifying them would change one client's
//! wire format, so only the DTOs and the read/remove/status/sync bodies
//! live here.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::host::AppHost;

/// Resolve the active workspace's `peers.json` path
/// (`<workspace>/.outl/peers.json`) and run the one-time global →
/// workspace migration. The peer list is per-GRAPH; only the device
/// identity stays per-install.
///
/// Errors with the loading sentinel while the background opener hasn't
/// published a workspace yet (desktop), so the frontend retries instead
/// of touching a stale path. Infallible on clients with a boot-resolved
/// root (mobile).
fn workspace_peers_path<S: AppHost>(state: &S) -> Result<PathBuf, String> {
    let root = state.storage_root()?;
    outl_sync_iroh::migrate_global_peers_if_absent(&root);
    Ok(outl_sync_iroh::workspace_peers_path(&root))
}

/// DTO for a paired peer.
///
/// `Clone` because the mobile pairing task hands one to
/// `app.emit("peer-paired", …)`, and Tauri's `Emitter::emit` takes the
/// payload by `Clone + Serialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub added_at: String,
}

impl From<outl_sync_iroh::PeerEntry> for PeerDto {
    fn from(p: outl_sync_iroh::PeerEntry) -> Self {
        Self {
            node_id: p.node_id,
            alias: p.alias,
            added_at: p.added_at,
        }
    }
}

/// DTO for a peer's live reachability status.
#[derive(Debug, Clone, Serialize)]
pub struct PeerStatusDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub online: bool,
    pub rtt_ms: Option<u64>,
}

/// List all paired devices.
pub fn peer_list<S: AppHost>(state: &S) -> Result<Vec<PeerDto>, String> {
    let peers = outl_sync_iroh::PeersStore::load_or_default(&workspace_peers_path(state)?)
        .map_err(|e| e.to_string())?;
    Ok(peers.list().iter().cloned().map(PeerDto::from).collect())
}

/// Remove a peer by node_id prefix; `true` if any matched.
pub fn peer_remove<S: AppHost>(state: &S, id: String) -> Result<bool, String> {
    let mut peers = outl_sync_iroh::PeersStore::load_or_default(&workspace_peers_path(state)?)
        .map_err(|e| e.to_string())?;
    peers.remove(&id).map_err(|e| e.to_string())
}

/// Reachability for each paired peer, read from the **running** iroh
/// transport's own dial outcomes — never a fresh probe endpoint.
///
/// Binding a second endpoint with the device identity (the old probe
/// path) hijacks the relay route from the long-lived sync endpoint, so
/// inbound sync connections get refused. Status reads the transport's
/// `peer_health()` snapshot (boot connect / catch-up / gossip / serve
/// fill it) and merges it onto the full peers.json list. A peer the
/// transport hasn't dialed yet — or the case where no iroh transport is
/// wired (file transport) — shows `online = false`.
pub fn peer_status<S: AppHost>(state: &S) -> Result<Vec<PeerStatusDto>, String> {
    let peers = outl_sync_iroh::PeersStore::load_or_default(&workspace_peers_path(state)?)
        .map_err(|e| e.to_string())?;

    // Snapshot the transport's per-peer health (empty when iroh isn't wired).
    let health: HashMap<String, outl_actions::PeerHealthSnapshot> = state
        .sync_transport()
        .map(|t| {
            t.peer_health()
                .into_iter()
                .map(|h| (h.node_id.clone(), h))
                .collect()
        })
        .unwrap_or_default();

    Ok(peers
        .list()
        .iter()
        .map(|p| {
            let h = health.get(&p.node_id);
            PeerStatusDto {
                node_id: p.node_id.clone(),
                alias: p.alias.clone(),
                online: h.map(|h| h.reachable).unwrap_or(false),
                rtt_ms: h.and_then(|h| h.last_rtt_ms),
            }
        })
        .collect())
}

/// Force an immediate P2P sync pass against every paired peer.
///
/// Backs the GUI's "Refresh" / pull-to-refresh affordance: instead of
/// waiting for the iroh transport's catch-up tick, the frontend calls
/// this to dial every peer right now and pull the freshest state, then
/// `reload_workspace` to re-render once the ops have landed.
///
/// No-op (returns `Ok`) when no iroh transport is wired (the file
/// transport has no peer to dial) or its runtime is down.
pub fn sync_now<S: AppHost>(state: &S) -> Result<(), String> {
    if let Some(t) = state.sync_transport() {
        t.sync_now();
    }
    Ok(())
}
