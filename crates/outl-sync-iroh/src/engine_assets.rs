//! Asset sync — peer-to-peer binary-asset (uploaded file) transfer.
//!
//! Uploaded files (a PDF, an image) are copied into `<root>/assets/<hash>.<ext>`
//! and referenced from markdown as `[name](assets/<hash>.<ext>)`. Their bytes
//! NEVER enter the op log (a multi-MB blob replayed through the CRDT would bloat
//! every device's log irreversibly — see `outl_actions::asset`). They are plain
//! content-addressed blobs, replicated like the `.md` projections: the `file`
//! transport (iCloud / Syncthing) carries them for free, but over the default
//! iroh (p2p) transport they must be transferred explicitly. This module is that
//! transport, mirroring [`crate::engine_snapshot`] for a *set* of blobs.
//!
//! Because a device holds N assets (not one, like a snapshot), the protocol
//! negotiates a **manifest** first:
//!
//! 1. The initiator opens a bi stream on [`ASSET_ALPN`] and sends an empty
//!    manifest-request marker frame.
//! 2. The responder ([`AssetProtocolHandler`]) replies with an
//!    [`AssetManifest`](crate::protocol::encode_asset_manifest) — the basenames
//!    in its `assets/` dir. Names ARE content hashes, so two devices name the
//!    same content identically; a name the initiator already holds needs no
//!    transfer.
//! 3. The initiator diffs the manifest against its own `assets/` and, for each
//!    missing file, sends an [`AssetRequest`](crate::protocol::encode_blob_frame)
//!    (the name) and reads back the bytes as a blob frame, writing each
//!    atomically (tmp + rename). After the last request it finishes the stream.
//! 4. The responder serves each requested name (validated: plain basename, no
//!    `/` or `..`) until the initiator finishes, then closes.
//!
//! Like the snapshot transfer, neither side holds the workspace lock — assets are
//! immutable content-addressed cache files read straight off disk — and every
//! failure is best-effort: an absent, unreadable, or hash-mismatched asset is
//! skipped, never fatal (the op log stays source of truth; a link to a
//! not-yet-transferred asset just renders as a dead link until the next pull).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, info, warn};

use outl_actions::{assets_dir, SyncProgress};
use outl_md::asset::is_safe_asset_name;

use crate::engine_sync::{read_frame, read_frame_reporting};
use crate::protocol::{
    decode_asset_manifest, encode_asset_manifest, encode_blob_frame, ASSET_ALPN,
};

/// Bound on a single asset-transfer connect attempt. Mirrors
/// [`crate::engine_snapshot`]'s `SNAPSHOT_CONNECT_TIMEOUT`: iroh 1.0.0 multipath
/// can stall ~30s on a dead direct addr, so each attempt is capped and the
/// bare-id (relay/discovery) fallback takes over.
const ASSET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Monotonic sequence for temp-file names, so two concurrent pulls of the same
/// asset (a pair + a catch-up tick landing together) never share a tmp path.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The content hash a file named `<hash>.<ext>` (or bare `<hash>`) claims, as the
/// stem before the first `.`. A sha-256 hex hash carries no dot, so this always
/// isolates it.
fn claimed_hash(name: &str) -> &str {
    name.split_once('.').map(|(h, _)| h).unwrap_or(name)
}

/// List the safe, transferable asset basenames in `<root>/assets/`.
///
/// Skips the dir entirely when absent (a workspace with no uploads yet → empty
/// manifest). Skips non-files, dotfiles, and `*.tmp` (in-flight writes from
/// `import_asset` / a concurrent pull), and any name that fails the
/// anti-traversal guard.
async fn list_asset_names(workspace_root: &Path) -> Vec<String> {
    let dir = assets_dir(workspace_root);
    let mut names = Vec::new();
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(_) => return names, // absent dir → empty manifest
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        if let Ok(ft) = entry.file_type().await {
            if !ft.is_file() {
                continue;
            }
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.ends_with(".tmp") {
            continue;
        }
        if is_safe_asset_name(&name) {
            names.push(name);
        }
    }
    names
}

/// Write `bytes` to `<dir>/<name>` atomically (unique tmp + rename), idempotent.
///
/// A pre-existing file is left untouched (content-addressed: the bytes are
/// identical). A rename race — a concurrent pull landed the same file first — is
/// treated as success, not an error.
async fn write_asset_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let dest = dir.join(name);
    if dest.exists() {
        return Ok(());
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.pull.tmp", std::process::id()));
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("write asset tmp {}", tmp.display()))?;
    if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
        // Clean up our tmp; if another concurrent pull already landed the file
        // (identical content-addressed bytes) that's success, not failure.
        let _ = tokio::fs::remove_file(&tmp).await;
        if dest.exists() {
            return Ok(());
        }
        return Err(e).with_context(|| format!("rename asset into {}", dest.display()));
    }
    Ok(())
}

/// Router handler that serves this device's `assets/` to a dialing peer.
///
/// Content-addressed and device-agnostic — unlike the snapshot handler, there is
/// no per-actor scoping: a device serves whatever assets it holds. Holds no
/// workspace lock (assets are immutable cache files read straight off disk).
#[derive(Clone)]
pub(crate) struct AssetProtocolHandler {
    /// Local workspace root, so the handler can resolve `<root>/assets/`.
    pub(crate) workspace_root: PathBuf,
}

impl std::fmt::Debug for AssetProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetProtocolHandler")
            .field("workspace_root", &self.workspace_root)
            .finish()
    }
}

impl ProtocolHandler for AssetProtocolHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(conn).await {
            warn!("asset serve failed: {e:#}");
            return Err(AcceptError::from_boxed(e.into()));
        }
        Ok(())
    }
}

impl AssetProtocolHandler {
    /// Send the manifest, then serve per-name byte requests until the initiator
    /// finishes its send stream.
    async fn serve(&self, conn: Connection) -> Result<()> {
        let (mut send, mut recv) = conn.accept_bi().await.context("accept asset bi stream")?;

        // 1. Drain the manifest-request marker frame (its content is irrelevant —
        //    the ALPN itself means "tell me what assets you have").
        let _marker = read_frame(&mut recv)
            .await
            .context("read asset manifest request")?;

        // 2. Ship the manifest (basenames we hold; empty when we have none).
        let names = list_asset_names(&self.workspace_root).await;
        send.write_all(&encode_asset_manifest(&names)?)
            .await
            .context("send asset manifest")?;

        // 3. Serve each requested name. The initiator sends one name frame per
        //    asset it lacks and reads back one blob frame; when it finishes the
        //    stream, `read_frame` errors (clean EOF) and we stop. We ALWAYS reply
        //    with exactly one frame per request so the ping-pong stays aligned —
        //    an unknown / unsafe / unreadable name yields an empty frame the
        //    initiator skips.
        let dir = assets_dir(&self.workspace_root);
        loop {
            let frame = match read_frame(&mut recv).await {
                Ok(f) => f,
                // Clean EOF (initiator finished) or the peer went away — either
                // ends the exchange.
                Err(_) => break,
            };
            let requested = std::str::from_utf8(&frame[4..]).ok().map(str::to_string);
            let bytes = match requested {
                Some(name) if is_safe_asset_name(&name) => {
                    // Absent / unreadable → empty reply (the initiator skips it).
                    tokio::fs::read(dir.join(&name)).await.unwrap_or_default()
                }
                Some(name) => {
                    warn!("asset serve: refusing unsafe requested name {name:?}");
                    Vec::new()
                }
                None => {
                    warn!("asset serve: non-UTF-8 asset request; replying empty");
                    Vec::new()
                }
            };
            send.write_all(&encode_blob_frame(&bytes)?)
                .await
                .context("send asset bytes")?;
        }

        send.finish().context("finish asset send")?;
        // Wait for the initiator to close before the endpoint tears the
        // connection down (mirrors the snapshot handler).
        conn.closed().await;
        Ok(())
    }
}

/// Connect to `peer` on [`ASSET_ALPN`], resilient to a stale direct address.
///
/// Mirrors `engine_snapshot::connect_snapshot`: try the full addr (fast on-LAN),
/// fall back to the bare node id via relay / discovery if that stalls or fails
/// and the addr carried a (possibly dead) direct addr.
async fn connect_asset(
    endpoint: &iroh::Endpoint,
    peer_addr: iroh::EndpointAddr,
) -> Result<Connection> {
    let node_id = peer_addr.id;
    let had_direct = peer_addr.ip_addrs().next().is_some();

    match tokio::time::timeout(
        ASSET_CONNECT_TIMEOUT,
        endpoint.connect(peer_addr, ASSET_ALPN),
    )
    .await
    {
        Ok(Ok(conn)) => return Ok(conn),
        Ok(Err(e)) if !had_direct => return Err(e).context("asset connect"),
        Err(_) if !had_direct => return Err(anyhow::anyhow!("asset connect timed out")),
        Ok(Err(e)) => debug!(
            "direct asset connect to {} failed ({e}); retrying via relay/discovery",
            node_id.fmt_short()
        ),
        Err(_) => debug!(
            "direct asset connect to {} timed out; retrying via relay/discovery",
            node_id.fmt_short()
        ),
    }

    tokio::time::timeout(ASSET_CONNECT_TIMEOUT, endpoint.connect(node_id, ASSET_ALPN))
        .await
        .context("asset relay/discovery connect timed out")?
        .context("asset connect (relay/discovery)")
}

/// Pull every asset `peer` holds that this device lacks, writing each atomically
/// into `<root>/assets/`. Best-effort and idempotent.
///
/// Returns the number of assets actually written. A peer with no assets (empty
/// manifest) or a diff that finds nothing missing returns `Ok(0)`. A single
/// asset that arrives empty (peer lacks it) or whose bytes don't hash to their
/// name (a corrupt / malicious peer) is skipped, never written — the rest still
/// transfer.
pub(crate) async fn pull_assets_from_peer(
    endpoint: &iroh::Endpoint,
    peer: iroh::EndpointAddr,
    workspace_root: &Path,
    progress: &crate::progress::ProgressSink,
) -> Result<usize> {
    let peer_node_id = peer.id;
    let peer_short = peer_node_id.fmt_short().to_string();
    let conn = connect_asset(endpoint, peer).await?;
    let (mut send, mut recv) = conn.open_bi().await.context("open asset bi stream")?;

    // 1. Request the manifest (empty marker frame establishes the stream).
    send.write_all(&encode_blob_frame(&[])?)
        .await
        .context("send asset manifest request")?;
    // 2. Read the manifest.
    let manifest = read_frame(&mut recv).await.context("read asset manifest")?;
    let names = decode_asset_manifest(&manifest)?;

    // 3. Diff against local `assets/`: keep only safe names we don't already
    //    hold (content-addressed → a name match means we have the bytes).
    let dir = assets_dir(workspace_root);
    let wanted: Vec<String> = names
        .into_iter()
        .filter(|name| is_safe_asset_name(name) && !dir.join(name).exists())
        .collect();

    if wanted.is_empty() {
        send.finish().context("finish asset request stream")?;
        conn.close(0u32.into(), b"done");
        return Ok(0);
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("create assets dir {}", dir.display()))?;

    let mut written = 0usize;
    for name in &wanted {
        // Request this asset's bytes.
        send.write_all(&encode_blob_frame(name.as_bytes())?)
            .await
            .context("send asset request")?;
        // Read the bytes. An asset can be up to the `[assets] max_bytes` cap
        // (100 MiB default), so report byte progress throttled to ~256 KiB steps
        // — the same honest-percentage feed the snapshot pull emits.
        let mut last_emitted = 0u64;
        let frame = read_frame_reporting(&mut recv, |received, total| {
            if total > 0
                && (received == 0 || received >= total || received - last_emitted >= 256 * 1024)
            {
                last_emitted = received;
                progress.emit(SyncProgress::Asset {
                    peer: peer_short.clone(),
                    received,
                    total,
                });
            }
        })
        .await
        .context("read asset bytes")?;

        let body = &frame[4..];
        if body.is_empty() {
            debug!("asset pull: peer {peer_short} lacks {name}");
            continue;
        }
        // Defense in depth against a corrupt / malicious peer: the filename IS
        // the content's sha-256, so recompute and compare before landing it. A
        // mismatch means the bytes are not what the name claims — discard.
        if outl_md::asset::hash_bytes(body) != claimed_hash(name) {
            warn!("asset pull: {name} from {peer_short} failed content-hash check; discarding");
            continue;
        }
        write_asset_atomic(&dir, name, body).await?;
        written += 1;
    }

    send.finish().context("finish asset request stream")?;
    conn.close(0u32.into(), b"done");
    if written > 0 {
        info!(
            "asset pull: wrote {written}/{} assets from {peer_short}",
            wanted.len()
        );
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `is_safe_asset_name` is owned + tested in `outl_md::asset`; this crate
    // consumes it as the single anti-traversal validator.

    #[test]
    fn claimed_hash_isolates_the_stem() {
        assert_eq!(claimed_hash("abc123.pdf"), "abc123");
        assert_eq!(claimed_hash("deadbeef"), "deadbeef");
        // Real names are `<sha256hex>.<ext>` — no dot in the hash itself.
        assert_eq!(claimed_hash("00ff.tar.gz"), "00ff");
    }
}
