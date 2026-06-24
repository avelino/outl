//! Tauri commands for peer/device management + pairing.
//!
//! These are the only commands that bypass the workspace lock: peer
//! pairing is sync-transport state, not workspace state, so they read
//! and edit the iroh `identity.key` / `peers.json` directly.
//!
//! ## Where the iroh files live (mobile-specific)
//!
//! Unlike the TUI/desktop (which use `~/.outl/`), iOS has no meaningful
//! home directory — the sandbox `$HOME` is opaque and per-install. We
//! resolve the iroh files from the Tauri **app local data dir** via
//! [`crate::iroh_sync::iroh_dir`], the same path the live transport in
//! `iroh_sync::wire_iroh_transport` uses. That keeps the running
//! transport and the pairing handshake pointed at one `peers.json`, so a
//! freshly paired device shows up in `outl_peer_list` and starts syncing
//! after the next launch without a second source of truth.

use std::collections::HashMap;

use outl_actions::SyncTransport;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::iroh_sync::{iroh_dir, peers_path};
use crate::state::AppState;

/// DTO for a paired peer.
///
/// `Clone` is required because [`outl_peer_pair_host`] hands a `PeerDto`
/// to `app.emit("peer-paired", …)`, and Tauri's `Emitter::emit` takes the
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

/// List all paired devices.
#[tauri::command]
pub fn outl_peer_list(app: AppHandle) -> Result<Vec<PeerDto>, String> {
    let dir = iroh_dir(&app)?;
    let peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path(&dir))
        .map_err(|e| e.to_string())?;
    Ok(peers.list().iter().cloned().map(PeerDto::from).collect())
}

/// Remove a peer by node_id prefix.
#[tauri::command]
pub fn outl_peer_remove(app: AppHandle, id: String) -> Result<bool, String> {
    let dir = iroh_dir(&app)?;
    let mut peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path(&dir))
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
/// The old path bound a transient endpoint with the device identity to
/// test-dial peers. In iroh the relay keeps a single `node_id → endpoint`
/// route, so that second endpoint hijacked the route from the long-lived sync
/// endpoint and inbound sync got refused. Status now reads the transport's
/// `peer_health()` snapshot (filled by boot connect / catch-up / gossip /
/// serve) and merges it onto the full peers.json list. A peer the transport
/// hasn't dialed yet — or the case where iroh isn't wired (file transport) —
/// shows `online = false`.
#[tauri::command]
pub fn outl_peer_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PeerStatusDto>, String> {
    let dir = iroh_dir(&app)?;
    let peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path(&dir))
        .map_err(|e| e.to_string())?;

    // Snapshot the transport's per-peer health (empty when iroh isn't wired).
    let health: HashMap<String, outl_actions::PeerHealthSnapshot> = state
        .iroh
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
/// Backs the GUI's pull-to-refresh / "sync now" affordance: instead of waiting
/// for the iroh transport's 8s catch-up tick, the frontend calls this to dial
/// every peer right now and pull the freshest state. The frontend then calls
/// `reload_workspace` to re-render once the ops have landed.
///
/// No-op (returns `Ok`) when no iroh transport is wired (the iCloud file
/// transport has no peer to dial) or when the transport's runtime is down.
#[tauri::command]
pub fn outl_sync_now(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(t) = state.iroh.as_ref() {
        t.sync_now();
    }
    Ok(())
}

/// Host one pairing session and return the ticket **immediately**.
///
/// ## Why the ticket comes back before pairing finishes
///
/// `outl_sync_iroh::host_pairing` binds an endpoint, surfaces the ticket
/// via an `on_ticket` callback, and *then blocks* waiting for the other
/// device to connect (up to a 120 s internal timeout). The frontend needs
/// the ticket string up front to render the QR code while the user walks
/// it over to the second device — it can't wait for the whole handshake.
///
/// So this command:
///
/// 1. Spawns `host_pairing` on a Tokio task.
/// 2. The `on_ticket` callback sends the ticket through a oneshot.
/// 3. The command `await`s that oneshot and returns the ticket as soon as
///    the endpoint is bound — long before a peer connects.
/// 4. The spawned task keeps running. When a device pairs (or it times
///    out), it emits `peer-paired` with the new [`PeerDto`] on success so
///    the frontend can refresh its device list. On failure it emits
///    `peer-pair-failed` with the error string.
///
/// The peer is persisted to the same `peers.json` the live transport
/// syncs against, so it takes effect on the next transport start.
#[tauri::command]
pub async fn outl_peer_pair_host(
    app: AppHandle,
    state: State<'_, AppState>,
    alias: Option<String>,
) -> Result<String, String> {
    // Reuse the LIVE sync endpoint for pairing — never bind a second endpoint
    // with the device identity. A second endpoint hijacks the relay route from
    // the running transport and silently kills sync (the "Another endpoint
    // connected with the same endpoint id" relay error). See
    // `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity".
    let transport = state
        .iroh
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;

    // A capacity-1 mpsc stands in for a oneshot: `tauri::async_runtime`
    // re-exports tokio's mpsc but not its oneshot. The `on_ticket`
    // closure is a *sync* `FnOnce`, so it uses the non-blocking
    // `try_send` (capacity 1 guarantees it never returns `Full` for the
    // single ticket).
    let (ticket_tx, mut ticket_rx) = tauri::async_runtime::channel::<String>(1);
    let app_for_task = app.clone();

    // The accept side outlives this command — it runs until a peer pairs
    // or the host-accept timeout fires inside the transport's `pair_host`.
    tauri::async_runtime::spawn(async move {
        let result = transport
            .pair_host(alias, move |ticket: &str| {
                // Hand the ticket back to the awaiting command; the QR is
                // rendered client-side from this string.
                let _ = ticket_tx.try_send(ticket.to_string());
            })
            .await;

        match result {
            Ok(entry) => {
                if let Err(e) = app_for_task.emit("peer-paired", PeerDto::from(entry)) {
                    tracing::warn!("emit peer-paired: {e}");
                }
            }
            Err(e) => {
                if let Err(emit_err) = app_for_task.emit("peer-pair-failed", e.to_string()) {
                    tracing::warn!("emit peer-pair-failed: {emit_err}");
                }
            }
        }
    });

    // Resolve as soon as the ticket is known. `recv()` yielding `None` means
    // the sender was dropped before a ticket arrived (the task errored before
    // arming) — surface that as the command error.
    ticket_rx
        .recv()
        .await
        .ok_or_else(|| "pairing host failed before producing a ticket".to_string())
}

/// Join a pairing session with a ticket from the hosting device.
///
/// `outl_sync_iroh::join_pairing` connects, runs the handshake, persists
/// the remote peer, and returns the stored [`PeerEntry`] — so this is a
/// plain async command that maps the result to a [`PeerDto`]. No event
/// needed: the caller gets the new peer back directly.
#[tauri::command]
pub async fn outl_peer_pair_join(
    state: State<'_, AppState>,
    ticket: String,
    alias: Option<String>,
) -> Result<PeerDto, String> {
    // Dial out over the LIVE sync endpoint, not a fresh one (see
    // `outl_peer_pair_host` for why a second endpoint is fatal).
    let transport = state
        .iroh
        .clone()
        .ok_or_else(|| "iroh transport not running; cannot pair".to_string())?;
    let entry = transport
        .pair_join(ticket, alias)
        .await
        .map_err(|e| e.to_string())?;
    Ok(PeerDto::from(entry))
}
