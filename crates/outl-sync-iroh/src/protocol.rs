//! Wire protocol for the outl sync ALPN.
//!
//! ALPN: `b"outl-sync/1"`
//!
//! ## Sync request (JSON, 4-byte length prefix)
//!
//! Sent by the side that wants to pull:
//! ```json
//! {
//!   "workspace_id": "my-workspace",
//!   "vector_clock": {
//!     "<actor-ulid>": { "physical_ms": 1234567890123, "logical": 5, "actor": "<ulid>" }
//!   }
//! }
//! ```
//!
//! ## Response (JSON, 4-byte length prefix)
//!
//! Sent by the responder right after it decodes the request, carrying the
//! responder's own vector clock so the initiator can compute the reverse
//! delta:
//! ```json
//! {
//!   "vector_clock": {
//!     "<actor-ulid>": { "physical_ms": 1234567890123, "logical": 5, "actor": "<ulid>" }
//!   }
//! }
//! ```
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
//! 3. responder → initiator: ops blob — ops where `op.ts > A[op.actor]`.
//! 4. initiator → responder: ops blob — ops where `op.ts > B[op.actor]`,
//!    then `finish()`.
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
pub const SYNC_ALPN: &[u8] = b"outl-sync/1";

/// ALPN for device pairing.
pub const PAIRING_ALPN: &[u8] = b"outl-sync/pair/1";

/// The body of a sync request.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Workspace slug identifier.
    pub workspace_id: String,
    /// Most recent HLC seen per actor. Missing actors imply HLC zero (never seen).
    pub vector_clock: HashMap<ActorId, Hlc>,
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
    /// Most recent HLC the responder has seen per actor. Missing actors imply
    /// HLC zero (never seen).
    pub vector_clock: HashMap<ActorId, Hlc>,
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
        vc.insert(actor, Hlc::new(42, 7, actor));
        let req = SyncRequest {
            workspace_id: "demo".into(),
            vector_clock: vc,
        };
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.workspace_id, "demo");
        assert_eq!(decoded.vector_clock.get(&actor).unwrap().physical_ms, 42);
    }

    #[test]
    fn decode_request_rejects_short_buffer() {
        assert!(decode_request(&[0, 0]).is_err());
    }

    #[test]
    fn response_roundtrips_through_length_prefix() {
        let mut vc = HashMap::new();
        let actor = ActorId::new();
        vc.insert(actor, Hlc::new(99, 3, actor));
        let resp = SyncResponse { vector_clock: vc };
        let encoded = encode_response(&resp).unwrap();
        let decoded = decode_response(&encoded).unwrap();
        assert_eq!(decoded.vector_clock.get(&actor).unwrap().physical_ms, 99);
        assert_eq!(decoded.vector_clock.get(&actor).unwrap().logical, 3);
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
