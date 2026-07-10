//! Test-only hooks that drive the **real** sync code paths (no logic copy).
//!
//! Integration tests live in `tests/` — a separate crate — so they can only
//! reach `pub` items. These thin wrappers expose the production `delta_sync`
//! initiator and the production `SyncProtocolHandler` responder (mounted on a
//! `Router`) so a test can stand up two endpoints over loopback and reconcile
//! them through the exact same wire code the transport runs.
//!
//! Connecting via a full [`iroh::EndpointAddr`] (with the direct addrs from
//! `endpoint.addr()`) keeps loopback sync deterministic without a relay or n0
//! discovery — the transport's own `delta_sync` connects by bare node id, which
//! would otherwise depend on discovery being reachable.
//!
//! This module is `#[doc(hidden)]` and exists purely so the out-of-crate
//! integration tests can exercise the real reconciliation logic.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::protocol::Router;
use outl_core::id::ActorId;
use outl_core::WorkspaceId;

use crate::engine::{delta_sync, SyncProtocolHandler};
use crate::engine_catchup::run_catch_up;
use crate::peers::PeerEntry;
use crate::protocol::SYNC_ALPN;

/// Bind a sync-ALPN endpoint with the given identity, exactly like the
/// transport's `run_iroh` does.
pub async fn bind_sync_endpoint(identity: &crate::IrohIdentity) -> Result<iroh::Endpoint> {
    // STOPGAP: IPv4-only bind, matching the production endpoints (engine /
    // pairing / status) so the loopback tests exercise the same code path.
    // IPv4-only also makes loopback sync more deterministic: `endpoint.addr()`
    // carries only the 127.0.0.1 direct addr, so the test connect never races a
    // `[::1]` path. Revert when iroh > 1.0.0 ships the multipath fallback fix.
    // See `crate::bind`.
    crate::bind::n0_builder_ipv4_only(None)
        .secret_key(identity.secret_key().clone())
        .alpns(vec![SYNC_ALPN.to_vec()])
        .bind()
        .await
        .context("bind sync endpoint")
}

/// A responder that completes the sync exchange up to — but NOT
/// including — the durable-ingest `close(0, "done")`.
///
/// It sends a valid (empty) response + empty push, drains the
/// initiator's frames so its writes all succeed, then closes with a
/// **non-"done"** code WITHOUT ingesting. This simulates a
/// suspended-iPhone / carrier-NAT-drop peer: the connection completes
/// cleanly for the initiator, but the peer never durably persisted the
/// push. Used to prove `delta_sync` does NOT report success in that
/// case (the false-"catch-up: sync ok" bug).
#[derive(Debug, Clone)]
struct HalfResponder;

impl iroh::protocol::ProtocolHandler for HalfResponder {
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> std::result::Result<(), iroh::protocol::AcceptError> {
        let Ok((mut send, mut recv)) = conn.accept_bi().await else {
            return Ok(());
        };
        // Send a valid response + empty push so the initiator proceeds
        // all the way through pushing its own ops.
        let response = crate::protocol::SyncResponse {
            vector_clock: std::collections::HashMap::new(),
        };
        if let Ok(bytes) = crate::protocol::encode_response(&response) {
            let _ = send.write_all(&bytes).await;
        }
        if let Ok(bytes) = crate::protocol::encode_ops_blob(&[]) {
            let _ = send.write_all(&bytes).await;
        }
        let _ = send.finish();
        // Drain the initiator's request + push so ITS writes succeed
        // (the connection looks healthy from the initiator's side) — but
        // never ingest, and close with a code that is NOT the "done"
        // durable-ingest sentinel.
        let _ = recv.read_to_end(16 * 1024 * 1024).await;
        conn.close(9u32.into(), b"early-no-ingest");
        Ok(())
    }
}

/// Mount a `HalfResponder` (completes the exchange but never confirms
/// durable ingest) on a `Router`. See the `HalfResponder` doc.
pub fn spawn_half_responder(endpoint: iroh::Endpoint) -> Router {
    Router::builder(endpoint)
        .accept(SYNC_ALPN, HalfResponder)
        .spawn()
}

/// Mount the production `SyncProtocolHandler` on a `Router` and return it.
///
/// Keep the returned `Router` alive for as long as the responder must accept
/// connections; drop it (or call `shutdown()`) to stop serving.
pub fn spawn_responder(
    endpoint: iroh::Endpoint,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> Router {
    Router::builder(endpoint)
        .accept(
            SYNC_ALPN,
            SyncProtocolHandler {
                workspace_root,
                workspace_id: Arc::new(RwLock::new(workspace_id)),
                actor,
                peer_ready_tx,
                // A fresh per-responder append guard: the test responder is the
                // only writer to its workspace, so a standalone lock is enough.
                append_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            },
        )
        .spawn()
}

/// Run the production `delta_sync` initiator against `peer` (a full
/// [`iroh::EndpointAddr`] from the responder's `endpoint.addr()`).
pub async fn run_delta_sync(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
) -> Result<()> {
    let append_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    delta_sync(
        endpoint,
        peer,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        &append_lock,
    )
    .await
}

/// Drive the production catch-up loop with a test-controlled tick `period` and a
/// `resolve_peers` closure (called once per tick) that yields the peers to dial.
///
/// This is the exact engine `run_iroh` spawns at boot; the only difference is
/// that production's resolver reloads `peers.json` and builds an addr from each
/// [`crate::PeerEntry`] (id + relay), whereas a test injects loopback
/// [`iroh::EndpointAddr`]s with direct addrs so no relay is needed. Lets a test
/// prove "a peer added AFTER the loop started gets caught up" over real QUIC.
///
/// Runs until the spawned task is dropped/aborted (the loop never returns).
///
/// `wid_changed`: `Some(rx)` wires the workspace-id-change broadcast so a test
/// can prove the loop **clears its per-session `synced` dedup and re-dials**
/// when the joiner adopts the host's id at runtime; `None` drives a fixed id and
/// asserts plain convergence (the adoption path is covered by
/// `catch_up_redials_after_workspace_id_change`).
#[allow(clippy::too_many_arguments)]
pub async fn run_catch_up_loop<F>(
    endpoint: iroh::Endpoint,
    period: Duration,
    resync_after: Duration,
    resolve_peers: F,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    wid_changed: Option<tokio::sync::broadcast::Receiver<WorkspaceId>>,
) where
    F: FnMut() -> Vec<iroh::EndpointAddr>,
{
    run_catch_up(
        endpoint,
        period,
        resync_after,
        resolve_peers,
        workspace_root,
        Arc::new(RwLock::new(workspace_id)),
        actor,
        peer_ready_tx,
        // Tests assert sync convergence, not the GUI reachability projection,
        // so they don't record health.
        None,
        // Fresh append guard; no shared in-flight set (the per-session `synced`
        // dedup inside the loop is what the test exercises).
        std::sync::Arc::new(tokio::sync::Mutex::new(())),
        None,
        wid_changed,
    )
    .await
}

/// The **real** gossip-topic derivation, exposed so an integration test can
/// assert that two devices at DIFFERENT local paths but with the SAME
/// [`WorkspaceId`] land on the SAME topic (and a different id → different topic).
/// Returns the topic as bytes so the test crate doesn't need the `iroh-gossip`
/// `TopicId` type in scope.
pub fn topic_id_bytes(workspace_id: &WorkspaceId) -> [u8; 32] {
    *crate::engine::workspace_topic_id(workspace_id).as_bytes()
}

/// Build the **real** membership broadcast payload from a `peers.json` file,
/// then parse it back through the **real** receive-side decoder — exactly the
/// round-trip a device performs over gossip. Returns the decoded peer list a
/// receiver would merge (empty when the source has no peers).
///
/// Exercises the production `build_membership_payload` + `parse_membership`
/// without standing up iroh-gossip's loopback swarm (which needs a relay to form
/// reliably). The transitive-merge + re-dial behaviour the test then asserts is
/// the same code the live receive task runs.
pub fn membership_roundtrip(peers_path: &Path) -> Vec<PeerEntry> {
    let Some(payload) = crate::engine_membership::build_membership_payload(peers_path)
        .expect("build membership payload")
    else {
        return Vec::new();
    };
    let content = std::str::from_utf8(&payload).expect("membership payload is utf8");
    crate::engine_membership::parse_membership(content)
        .expect("payload is a membership message")
        .expect("membership payload decodes")
}

/// Merge a gossiped peer list into `peers_path` via the **real**
/// `merge_membership` (drops self + unreachable, only adds unknown node_ids,
/// persists). Returns the number of peers newly added.
pub fn membership_merge(peers_path: &Path, self_node_id: &str, incoming: Vec<PeerEntry>) -> usize {
    crate::engine_membership::merge_membership(peers_path, self_node_id, incoming)
        .expect("merge membership")
}
