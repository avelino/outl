//! Tauri commands for peer/device management.
//!
//! List / remove / status / force-sync are thin wrappers over
//! `outl_tauri_shared::commands::peers`. The **pairing** commands stay
//! desktop-local: they need the concrete `IrohSyncTransport` from
//! `AppState::iroh_pairing` (pairing isn't a `SyncTransport` trait
//! concern) and their reply shape (`PairedPeerDto` + the early
//! `peer-pairing-ticket` event) is the desktop's wire contract — the
//! mobile client resolves the raw ticket string instead.

use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use outl_tauri_shared::commands::peers::{self as shared, PeerDto, PeerStatusDto};

/// List all paired devices.
#[tauri::command]
pub fn outl_peer_list(state: State<'_, AppState>) -> Result<Vec<PeerDto>, String> {
    shared::peer_list(state.inner())
}

/// Remove a peer by node_id prefix.
#[tauri::command]
pub fn outl_peer_remove(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    shared::peer_remove(state.inner(), id)
}

/// Reachability for each paired peer, read from the **running** iroh
/// transport's own dial outcomes — see the shared body for why a fresh
/// probe endpoint is never bound.
#[tauri::command]
pub fn outl_peer_status(state: State<'_, AppState>) -> Result<Vec<PeerStatusDto>, String> {
    shared::peer_status(state.inner())
}

/// Force an immediate P2P sync pass against every paired peer — the
/// trigger behind the Sync panel's Refresh.
#[tauri::command]
pub fn outl_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    shared::sync_now(state.inner())
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
/// the workspace's `.outl/peers.json`. The command resolves with the same
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
/// the handshake, persists the host to the workspace's `.outl/peers.json`,
/// and emits `peer-paired`.
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
