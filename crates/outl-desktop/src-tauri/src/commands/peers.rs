//! Tauri commands for peer/device management.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

fn outl_dir() -> std::path::PathBuf {
    dirs::home_dir().expect("home dir").join(".outl")
}

/// DTO for a paired peer.
#[derive(Debug, Serialize, Deserialize)]
pub struct PeerDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub added_at: String,
}

/// List all paired devices.
#[tauri::command]
pub fn outl_peer_list() -> Result<Vec<PeerDto>, String> {
    let peers = outl_sync_iroh::PeersStore::load_or_default(&outl_dir().join("peers.json"))
        .map_err(|e| e.to_string())?;
    Ok(peers
        .list()
        .iter()
        .map(|p| PeerDto {
            node_id: p.node_id.clone(),
            alias: p.alias.clone(),
            added_at: p.added_at.clone(),
        })
        .collect())
}

/// Remove a peer by node_id prefix.
#[tauri::command]
pub fn outl_peer_remove(id: String) -> Result<bool, String> {
    let mut peers = outl_sync_iroh::PeersStore::load_or_default(&outl_dir().join("peers.json"))
        .map_err(|e| e.to_string())?;
    peers.remove(&id).map_err(|e| e.to_string())
}

/// DTO for a peer's live reachability status.
#[derive(serde::Serialize)]
pub struct PeerStatusDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub online: bool,
    pub rtt_ms: Option<u64>,
}

/// Reachability for each paired peer, read from the **running** iroh
/// transport's own dial outcomes — never a fresh probe endpoint.
///
/// Binding a second endpoint with the device identity (the old probe path)
/// hijacks the relay route from the long-lived sync endpoint, so inbound sync
/// connections get refused. Status now reads the transport's
/// `peer_health()` snapshot (boot connect / catch-up / gossip / serve fill it)
/// and merges it onto the full peers.json list. A peer the transport hasn't
/// dialed yet — or the case where no iroh transport is wired (file transport)
/// — shows `online = false`.
#[tauri::command]
pub fn outl_peer_status(state: State<'_, AppState>) -> Result<Vec<PeerStatusDto>, String> {
    let peers = outl_sync_iroh::PeersStore::load_or_default(&outl_dir().join("peers.json"))
        .map_err(|e| e.to_string())?;

    // Snapshot the transport's per-peer health (empty when iroh isn't wired).
    let health: HashMap<String, outl_actions::PeerHealthSnapshot> = state
        .iroh_transport
        .lock()
        .as_ref()
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
/// Backs the GUI's "Refresh" / "sync now" affordance: instead of waiting for
/// the iroh transport's 8s catch-up tick, the frontend calls this to dial every
/// peer right now and pull the freshest state, then `reload_workspace` to
/// re-render once the ops have landed.
///
/// Reads the stored `Arc<dyn SyncTransport>` and calls the trait's `sync_now`.
/// No-op (returns `Ok`) when no iroh transport is wired (the file transport's
/// default `sync_now` does nothing) or its runtime is down.
#[tauri::command]
pub fn outl_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(t) = state.iroh_transport.lock().as_ref() {
        t.sync_now();
    }
    Ok(())
}

/// Result of a completed pairing handshake — the peer that was added.
#[derive(serde::Serialize)]
pub struct PairedPeerDto {
    pub node_id: String,
    pub alias: Option<String>,
    pub added_at: String,
}

impl From<outl_sync_iroh::PeerEntry> for PairedPeerDto {
    fn from(p: outl_sync_iroh::PeerEntry) -> Self {
        PairedPeerDto {
            node_id: p.node_id,
            alias: p.alias,
            added_at: p.added_at,
        }
    }
}

/// Host a pairing session: bind a transient endpoint, emit the ticket
/// **early** (so the frontend can render it / a QR while we wait), then
/// block until the other device connects and completes the handshake.
///
/// Mirrors the mobile/CLI design: the ticket is surfaced before the
/// command resolves via the `peer-pairing-ticket` event (`{ ticket }`),
/// and `peer-paired` (`PairedPeerDto`) fires once a peer is persisted to
/// `~/.outl/peers.json`. The command resolves with the same
/// [`PairedPeerDto`] so a caller that prefers awaiting over listening
/// also gets the result.
///
/// `alias` is an optional human label advertised to the peer.
#[tauri::command]
pub async fn outl_peer_pair_host(
    app: AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
) -> Result<PairedPeerDto, String> {
    // Pair over the LIVE sync endpoint — never bind a second endpoint with the
    // device identity, which would hijack the relay route and silently kill
    // sync ("Another endpoint connected with the same endpoint id"). See
    // `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity".
    let transport = state
        .iroh_pairing
        .lock()
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;

    // `pair_host` invokes `on_ticket` the moment the ticket is known — before
    // it blocks on the inbound connection — so emitting from there gets the
    // ticket to the UI immediately.
    let ticket_app = app.clone();
    let entry = transport
        .pair_host(alias, move |ticket| {
            if let Err(e) = ticket_app.emit(
                "peer-pairing-ticket",
                PairingTicketPayload {
                    ticket: ticket.to_string(),
                },
            ) {
                tracing::warn!("emit peer-pairing-ticket: {e}");
            }
        })
        .await
        .map_err(|e| e.to_string())?;

    let dto: PairedPeerDto = entry.into();
    if let Err(e) = app.emit("peer-paired", &dto) {
        tracing::warn!("emit peer-paired: {e}");
    }
    Ok(dto)
}

/// Join a pairing session from a ticket string produced by a host's
/// [`outl_peer_pair_host`]. Dials over the **live sync endpoint**, completes
/// the handshake, persists the host to `~/.outl/peers.json`, and emits
/// `peer-paired`.
#[tauri::command]
pub async fn outl_peer_pair_join(
    app: AppHandle,
    state: State<'_, AppState>,
    ticket: String,
    alias: Option<String>,
) -> Result<PairedPeerDto, String> {
    let transport = state
        .iroh_pairing
        .lock()
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;

    let entry = transport
        .pair_join(ticket, alias)
        .await
        .map_err(|e| e.to_string())?;

    let dto: PairedPeerDto = entry.into();
    if let Err(e) = app.emit("peer-paired", &dto) {
        tracing::warn!("emit peer-paired: {e}");
    }
    Ok(dto)
}

/// Payload for the early `peer-pairing-ticket` event the host emits
/// before the handshake completes.
#[derive(serde::Serialize, Clone)]
struct PairingTicketPayload {
    ticket: String,
}
