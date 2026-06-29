//! Live peer-status probing over the sync ALPN.
//!
//! [`probe_peers`] opens a short-lived iroh endpoint and attempts a
//! `SYNC_ALPN` connection to every peer in the [`PeersStore`], reporting
//! reachability and round-trip latency.
//!
//! ## CLI-only — the GUI must NOT use this
//!
//! This binds a **transient endpoint with the device identity**. When a
//! [`crate::IrohSyncTransport`] is already running (every GUI client), that
//! second endpoint hijacks the relay's `node_id → endpoint` route from the
//! long-lived sync endpoint, so inbound sync connections get refused
//! ("the server refused to accept a new connection"). The desktop / mobile
//! `outl_peer_status` commands therefore read reachability from the running
//! transport's `peer_health()` snapshot instead — see
//! `crate::IrohSyncTransport::peer_health` and the crate `CLAUDE.md`.
//!
//! `probe_peers` survives **only** for `outl peer status` in the CLI, which has
//! no running transport (and so no route to steal). Don't reach for it from any
//! context where the transport is live.
//!
//! Probing is best-effort: a failed connection is reported as
//! `online = false`, never an error on the whole batch.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iroh::Endpoint;

use crate::identity::IrohIdentity;
use crate::peers::{PeerEntry, PeersStore};
use crate::protocol::SYNC_ALPN;

/// How long to wait for a single peer connection before declaring it offline.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Reachability status for a single known peer.
#[derive(Debug, Clone)]
pub struct PeerStatus {
    /// Peer's iroh node id (hex-encoded public key), copied from its
    /// [`PeerEntry`].
    pub node_id: String,
    /// Human-readable label the peer advertised at pairing time, if any.
    pub alias: Option<String>,
    /// Whether a `SYNC_ALPN` connection succeeded within the probe timeout.
    pub online: bool,
    /// Round-trip time of the successful connect, in milliseconds. `None` when
    /// the peer is offline.
    pub rtt_ms: Option<u64>,
}

/// Probe every peer in `peers` by attempting a short-timeout iroh connection
/// on [`SYNC_ALPN`]. Returns one [`PeerStatus`] per peer, in the store's order.
///
/// Builds one transient endpoint with `identity` and probes all peers
/// concurrently. A peer that fails to connect (timeout, no route, refused)
/// is reported as `online = false` with `rtt_ms = None` — it never fails the
/// whole batch.
pub async fn probe_peers(identity: &IrohIdentity, peers: &PeersStore) -> Result<Vec<PeerStatus>> {
    let entries: Vec<PeerEntry> = peers.list().to_vec();
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // STOPGAP: IPv4-only bind (iroh 1.0.0 multipath stalls on unreachable IPv6
    // direct paths). Matches the engine/pairing endpoints so the CLI probe
    // reflects the same reachability the real sync gets. Revert to the plain
    // dual-stack builder when iroh > 1.0.0 ships the multipath fallback fix.
    // See `crate::bind`. (CLI-only: see the module doc — the GUI reads
    // reachability from the running transport, not this probe.)
    let endpoint = crate::bind::n0_builder_ipv4_only(None)
        .secret_key(identity.secret_key().clone())
        .alpns(vec![SYNC_ALPN.to_vec()])
        .bind()
        .await
        .context("bind probe endpoint")?;

    // Spawn one probe task per peer so they run concurrently. Each task owns a
    // clone of the endpoint and resolves to a fully-formed PeerStatus.
    let mut handles = Vec::with_capacity(entries.len());
    for entry in entries {
        let ep = endpoint.clone();
        handles.push(tokio::spawn(async move { probe_one(&ep, entry).await }));
    }

    let mut statuses = Vec::with_capacity(handles.len());
    for handle in handles {
        // A panicked probe task (it shouldn't panic) degrades to "offline"
        // rather than poisoning the batch.
        match handle.await {
            Ok(status) => statuses.push(status),
            Err(join_err) => {
                tracing::warn!("peer probe task failed: {join_err}");
            }
        }
    }

    endpoint.close().await;
    Ok(statuses)
}

/// Probe a single peer. Never returns an error: an unreachable peer yields a
/// `PeerStatus` with `online = false`.
async fn probe_one(endpoint: &Endpoint, entry: PeerEntry) -> PeerStatus {
    let mut status = PeerStatus {
        node_id: entry.node_id.clone(),
        alias: entry.alias.clone(),
        online: false,
        rtt_ms: None,
    };

    // Dial the peer's full `EndpointAddr` (relay + direct addrs captured at
    // pairing) so the probe doesn't depend on n0 discovery resolving a bare
    // node id — that dependency was why the status dot showed offline even when
    // the peer was reachable on the same LAN. Falls back to id (+ relay url) for
    // legacy peers.json entries.
    let addr = match entry.iroh_endpoint_addr() {
        Ok(addr) => addr,
        Err(e) => {
            tracing::debug!("peer {} has an unparseable node id: {e}", entry.node_id);
            return status;
        }
    };
    let node_id = addr.id;

    let started = Instant::now();
    match tokio::time::timeout(PROBE_TIMEOUT, endpoint.connect(addr, SYNC_ALPN)).await {
        Ok(Ok(conn)) => {
            let rtt = started.elapsed();
            status.online = true;
            status.rtt_ms = Some(rtt.as_millis() as u64);
            // Close cleanly so the peer doesn't keep the connection around.
            conn.close(0u32.into(), b"probe");
        }
        Ok(Err(e)) => {
            tracing::debug!("probe connect to {} failed: {e}", node_id.fmt_short());
        }
        Err(_timeout) => {
            tracing::debug!("probe connect to {} timed out", node_id.fmt_short());
        }
    }

    status
}

/// Blocking wrapper around [`probe_peers`] for callers without a tokio runtime
/// (e.g. the CLI). Spins up a current-thread runtime and blocks on the probe.
pub fn probe_peers_blocking(
    identity: &IrohIdentity,
    peers: &PeersStore,
) -> Result<Vec<PeerStatus>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build current-thread runtime for peer probe")?;
    rt.block_on(probe_peers(identity, peers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_probes_to_empty_vec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity =
            IrohIdentity::load_or_generate(&dir.path().join("identity.key")).expect("identity");
        let peers = PeersStore::load_or_default(&dir.path().join("peers.json")).expect("peers");

        let statuses = probe_peers_blocking(&identity, &peers).expect("probe");
        assert!(statuses.is_empty());
    }

    #[test]
    fn unreachable_peer_is_offline_not_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity =
            IrohIdentity::load_or_generate(&dir.path().join("identity.key")).expect("identity");
        let peers_path = dir.path().join("peers.json");
        let mut peers = PeersStore::load_or_default(&peers_path).expect("peers");
        // A valid-but-unreachable node id (random keypair we never bind).
        let unreachable = iroh::SecretKey::generate().public().to_string();
        peers
            .add(PeerEntry {
                node_id: unreachable.clone(),
                alias: Some("ghost".into()),
                relay_url: None,
                endpoint_addr: None,
                added_at: "2026-01-01T00:00:00Z".into(),
            })
            .expect("add peer");

        let statuses = probe_peers_blocking(&identity, &peers).expect("probe");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].node_id, unreachable);
        assert_eq!(statuses[0].alias.as_deref(), Some("ghost"));
        assert!(!statuses[0].online);
        assert!(statuses[0].rtt_ms.is_none());
    }
}
