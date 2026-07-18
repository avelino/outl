//! Wire protocol for the outl sync ALPN.
//!
//! ALPN: `b"outl-sync/2"`
//!
//! ## Sync request (JSON, 4-byte length prefix)
//!
//! Sent by the side that wants to pull:
//! ```json
//! {
//!   "workspace_id": "my-workspace",
//!   "vector_clock": {
//!     "<actor-ulid>": {
//!       "max": { "physical_ms": 1234567890123, "logical": 5, "actor": "<ulid>" },
//!       "count": 347
//!     }
//!   }
//! }
//! ```
//!
//! ## Response (JSON, 4-byte length prefix)
//!
//! Sent by the responder right after it decodes the request, carrying the
//! responder's own vector clock so the initiator can compute the reverse
//! delta. Same `{ actor → ActorClock }` shape as the request's `vector_clock`.
//!
//! ## Ops blob (JSONL, 4-byte length prefix)
//!
//! A length-prefixed batch of newline-separated `LogOp` JSON lines. Used in
//! both directions so a single bi stream can carry two independent op batches
//! without EOF framing ambiguity.
//!
//! ## Bidirectional exchange (single bi stream)
//!
//! 1. initiator → responder: [`SyncRequest`] (vector clock A).
//! 2. responder → initiator: [`SyncResponse`] (vector clock B).
//! 3. responder → initiator: ops blob — ops missing under clock A (per-actor:
//!    everything above `A[actor].max`, or the actor's FULL log when a gap
//!    below `A[actor].max` is detected — see `engine_sync::ops_missing_for`).
//! 4. initiator → responder: ops blob — same rule under clock B, then
//!    `finish()`.
//!
//! Every step is length-prefixed, so both directions fully reconcile on one
//! connection.

use anyhow::Result;
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::LogOp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ALPN for the op-sync protocol.
///
/// v2 bumped the vector clock from a bare max-HLC per actor to
/// `ActorClock` (max + count) so the sender can detect gaps below the
/// receiver's watermark. v1 and v2 clocks are wire-incompatible; the ALPN
/// bump makes an old↔new dial fail cleanly at connect instead of
/// half-conversing.
pub const SYNC_ALPN: &[u8] = b"outl-sync/2";

/// ALPN for device pairing.
pub const PAIRING_ALPN: &[u8] = b"outl-sync/pair/1";

/// ALPN for peer snapshot transfer (Phase 2 snapshot sync).
///
/// A freshly-paired device pulls a peer's materialized snapshot
/// (`snap-<actor>.bin`) over this ALPN so it can boot from settled state
/// instead of receiving + replaying the full op log. Carried on the SAME sync
/// endpoint's router (one endpoint per identity). See `crate::engine_snapshot`.
pub const SNAPSHOT_ALPN: &[u8] = b"outl-snapshot/1";

/// What one side knows about one actor's ops: the highest HLC it holds and
/// how many DISTINCT ops (by HLC) it holds for that actor — all `<= max` by
/// definition.
///
/// The `count` is what turns the max-HLC watermark into a gap detector: a
/// bare max assumes in-order, gapless delivery, so an op landing AHEAD of a
/// pending backlog permanently hid everything below the watermark (the
/// sender assumed the receiver had it). With the count, the sender can tell
/// "receiver holds fewer ops below its own max than I do" and fall back to a
/// full-log resend for that actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorClock {
    /// Highest HLC held for this actor.
    pub max: Hlc,
    /// Number of distinct ops (by HLC) held for this actor.
    pub count: u64,
}

/// The body of a sync request.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Workspace slug identifier.
    pub workspace_id: String,
    /// Per-actor max-HLC + distinct-op count. Missing actors imply "never
    /// seen" (HLC zero, zero ops).
    pub vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncRequest` with a 4-byte big-endian length prefix.
pub fn encode_request(req: &SyncRequest) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(req)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncRequest` from a 4-byte length-prefixed buffer.
pub fn decode_request(buf: &[u8]) -> Result<SyncRequest> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    Ok(serde_json::from_slice(&buf[4..4 + len])?)
}

/// The body of a sync response — the responder's own vector clock.
///
/// Sent right after the responder decodes the request, so the initiator can
/// compute the reverse delta (the ops the responder is missing).
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Per-actor max-HLC + distinct-op count the responder holds. Missing
    /// actors imply "never seen" (HLC zero, zero ops).
    pub vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncResponse` with a 4-byte big-endian length prefix.
pub fn encode_response(resp: &SyncResponse) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(resp)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncResponse` from a 4-byte length-prefixed buffer.
pub fn decode_response(buf: &[u8]) -> Result<SyncResponse> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    Ok(serde_json::from_slice(&buf[4..4 + len])?)
}

/// Serialize a single `LogOp` as a JSONL line (no trailing newline).
pub fn encode_op(op: &LogOp) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(op)?)
}

/// Deserialize a JSONL line into a `LogOp`.
pub fn decode_op(line: &[u8]) -> Result<LogOp> {
    Ok(serde_json::from_slice(line)?)
}

/// Serialize a batch of `LogOp`s into a length-prefixed JSONL blob.
///
/// Layout: `[4-byte big-endian length][JSONL body]`, where the body is
/// newline-separated `LogOp` JSON lines (with a trailing newline per line).
/// An empty slice yields a zero-length body, so "no ops to send" is still a
/// valid, unambiguous frame.
pub fn encode_ops_blob(ops: &[LogOp]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for op in ops {
        body.extend_from_slice(&encode_op(op)?);
        body.push(b'\n');
    }
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&body);
    Ok(buf)
}

/// Frame an arbitrary byte blob with a 4-byte big-endian length prefix.
///
/// Same framing as [`encode_ops_blob`], but over raw bytes rather than encoded
/// ops — used by [`crate::engine_snapshot`] to ship a materialized snapshot
/// (`snap-<actor>.bin`) as one frame on a bi stream. An empty slice yields a
/// valid zero-length body, so "no snapshot to send" is still an unambiguous
/// frame the reader can skip.
pub fn encode_blob_frame(body: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(body);
    Ok(buf)
}

/// Read the declared length of a length-prefixed ops blob from its first
/// 4 bytes. The full frame is `4 + returned_len` bytes.
pub fn ops_blob_len(prefix: &[u8]) -> Result<usize> {
    anyhow::ensure!(prefix.len() >= 4, "buffer too short for length prefix");
    Ok(u32::from_be_bytes(prefix[..4].try_into()?) as usize)
}

/// Decode a length-prefixed ops blob into `LogOp`s.
///
/// Lines that fail to decode are skipped (the caller logs); the function only
/// errors on a malformed length prefix.
pub fn decode_ops_blob(buf: &[u8]) -> Result<Vec<LogOp>> {
    let len = ops_blob_len(buf)?;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    let body = &buf[4..4 + len];
    let mut ops = Vec::new();
    for line in body.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(op) = decode_op(line) {
            ops.push(op);
        }
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_through_length_prefix() {
        let mut vc = HashMap::new();
        let actor = ActorId::new();
        vc.insert(
            actor,
            ActorClock {
                max: Hlc::new(42, 7, actor),
                count: 12,
            },
        );
        let req = SyncRequest {
            workspace_id: "demo".into(),
            vector_clock: vc,
        };
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.workspace_id, "demo");
        let clock = decoded.vector_clock.get(&actor).unwrap();
        assert_eq!(clock.max.physical_ms, 42);
        assert_eq!(clock.count, 12);
    }

    #[test]
    fn decode_request_rejects_short_buffer() {
        assert!(decode_request(&[0, 0]).is_err());
    }

    #[test]
    fn response_roundtrips_through_length_prefix() {
        let mut vc = HashMap::new();
        let actor = ActorId::new();
        vc.insert(
            actor,
            ActorClock {
                max: Hlc::new(99, 3, actor),
                count: 2000,
            },
        );
        let resp = SyncResponse { vector_clock: vc };
        let encoded = encode_response(&resp).unwrap();
        let decoded = decode_response(&encoded).unwrap();
        let clock = decoded.vector_clock.get(&actor).unwrap();
        assert_eq!(clock.max.physical_ms, 99);
        assert_eq!(clock.max.logical, 3);
        assert_eq!(clock.count, 2000);
    }

    #[test]
    fn decode_response_rejects_short_buffer() {
        assert!(decode_response(&[0, 0]).is_err());
    }

    fn sample_op(actor: ActorId, physical_ms: u64) -> LogOp {
        use outl_core::fractional::Fractional;
        use outl_core::id::NodeId;
        use outl_core::op::Op;
        LogOp {
            ts: Hlc::new(physical_ms, 0, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    #[test]
    fn ops_blob_roundtrips() {
        let actor = ActorId::new();
        let ops = vec![
            sample_op(actor, 1),
            sample_op(actor, 2),
            sample_op(actor, 3),
        ];
        let blob = encode_ops_blob(&ops).unwrap();
        let decoded = decode_ops_blob(&blob).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].ts.physical_ms, 1);
        assert_eq!(decoded[2].ts.physical_ms, 3);
    }

    #[test]
    fn empty_ops_blob_is_valid_zero_length_frame() {
        let blob = encode_ops_blob(&[]).unwrap();
        assert_eq!(blob.len(), 4, "empty blob is just the length prefix");
        assert_eq!(ops_blob_len(&blob).unwrap(), 0);
        assert!(decode_ops_blob(&blob).unwrap().is_empty());
    }

    #[test]
    fn ops_blob_len_reads_declared_length() {
        let actor = ActorId::new();
        let ops = vec![sample_op(actor, 7)];
        let blob = encode_ops_blob(&ops).unwrap();
        let declared = ops_blob_len(&blob[..4]).unwrap();
        assert_eq!(declared, blob.len() - 4);
    }

    #[test]
    fn decode_ops_blob_rejects_short_buffer() {
        assert!(decode_ops_blob(&[0, 0]).is_err());
    }
}
