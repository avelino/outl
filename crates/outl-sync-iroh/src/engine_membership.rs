//! Mesh membership auto-discovery over the existing gossip topic.
//!
//! ## Why this module exists
//!
//! Without it, the mesh only converges through **transitive op propagation**
//! (A↔B↔C reconciles ops) plus **manually pairing every pair** of devices to
//! get direct links. Item 5 closes that gap: when A pairs with B and B already
//! knows C, A should learn C's reachability automatically, so the user never
//! has to hand-pair every pair to get a full mesh.
//!
//! ## How
//!
//! Each device periodically broadcasts its **known peer list** (the same
//! node_id + relay/endpoint_addr reachability `peers.json` stores) over the
//! existing workspace gossip topic, as a message kind *distinct* from the
//! op-announcement. On receiving a membership message, a device merges any
//! **unknown** peers into its local [`PeersStore`] and persists `peers.json`.
//! The existing catch-up loop (which reloads `peers.json` every tick) then dials
//! the newly-merged peers — no extra dialing machinery here.
//!
//! ## Message kind (tagged, back-compat with op-announce)
//!
//! The op-announce message is the untagged `"workspace_id\nactor\nhlc"` format
//! parsed in [`crate::engine::run_iroh`]. Membership messages carry a distinct
//! first line — [`MEMBERSHIP_TAG`] — so the receive side routes them before
//! falling through to the announce parser. An announce's first token is a
//! workspace slug (a directory name), which never equals the literal
//! `"outl-membership/1"`, so the two kinds never collide.
//!
//! Wire format:
//!
//! ```text
//! outl-membership/1\n<json array of PeerEntry>
//! ```
//!
//! ## Trust model (load-bearing)
//!
//! **Every device subscribed to the workspace gossip topic is already inside the
//! trust domain.** The topic id is `blake3(workspace_root)` (see
//! [`crate::engine::workspace_topic_id`]) — only devices that were paired into
//! this mesh by *someone* ever subscribe to it. Membership gossip therefore only
//! ever ADDS reachability for peers that are *already mesh members*; it never
//! invites a stranger. A device that isn't on the topic can't inject a peer, and
//! a peer we merge was already trusted by the device that gossiped it.
//!
//! Conservative guards on the merge:
//!
//! - **Never add self.** A device drops its own node_id from any incoming list.
//! - **Never add an unreachable peer.** An entry without a usable
//!   relay/endpoint_addr (its [`PeerEntry::iroh_endpoint_addr`] won't resolve) is
//!   skipped — we don't store a peer we can't dial.
//! - **Dedup by node_id; only ADD unknown peers.** A node_id already in
//!   `peers.json` is left untouched (its locally-captured addr, e.g. from direct
//!   pairing, may be fresher than the gossiped one). See
//!   [`PeersStore::merge_unknown`].

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::peers::{PeerEntry, PeersStore};

/// First-line tag marking a gossip message as a membership broadcast (as opposed
/// to the untagged op-announce). Versioned so the format can evolve.
pub(crate) const MEMBERSHIP_TAG: &str = "outl-membership/1";

/// How often a device re-broadcasts its known peer list over gossip.
///
/// Short enough that a device paired into the mesh learns the rest of the mesh
/// within a couple of ticks, long enough that the chatter is negligible (the
/// payload is a handful of small JSON entries). The catch-up loop's 8s tick then
/// dials whatever this merges, so end-to-end discovery settles in well under a
/// catch-up cycle plus a membership tick.
pub(crate) const MEMBERSHIP_INTERVAL: Duration = Duration::from_secs(5);

/// Build the membership broadcast payload from the current peer list on disk.
///
/// Reloads `peers.json` so the broadcast always reflects the latest known set
/// (including peers paired after boot). Returns the tagged bytes ready for
/// `GossipSender::broadcast`. Returns `Ok(None)` when there are no peers to share
/// (nothing to gossip — don't spam an empty list).
pub(crate) fn build_membership_payload(peers_path: &Path) -> Result<Option<bytes::Bytes>> {
    let store = PeersStore::load_or_default(peers_path).context("reload peers.json for gossip")?;
    let peers = store.list();
    if peers.is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_string(peers).context("serialize membership peer list")?;
    let payload = format!("{MEMBERSHIP_TAG}\n{json}");
    Ok(Some(bytes::Bytes::from(payload)))
}

/// If `content` is a membership message, return the decoded peer list it carries.
///
/// Returns `None` when `content` is **not** a membership message (so the caller
/// falls through to the op-announce parser). A membership message with a
/// malformed body returns `Some(Err(..))` so the caller can log it.
pub(crate) fn parse_membership(content: &str) -> Option<Result<Vec<PeerEntry>>> {
    let body = content.strip_prefix(MEMBERSHIP_TAG)?;
    // The tag must be a full first line: either the whole message is just the
    // tag (no body → empty list) or the tag is followed by a newline + body.
    let body = match body.strip_prefix('\n') {
        Some(rest) => rest,
        None if body.is_empty() => return Some(Ok(Vec::new())),
        // Tag is a prefix of a longer token (e.g. a workspace slug that happens
        // to start with the tag) — not a membership message.
        None => return None,
    };
    Some(serde_json::from_str::<Vec<PeerEntry>>(body).context("decode membership peer list"))
}

/// Merge a gossiped peer list into the local store, persisting `peers.json`.
///
/// Conservative by design (see the module-level trust model): drops self, drops
/// peers with no usable reachability, and only ADDS node_ids not already known
/// (an existing entry's locally-captured addr is left untouched).
///
/// Returns the number of peers newly added (0 means nothing changed, so the
/// caller can skip logging / re-dial hints).
pub(crate) fn merge_membership(
    peers_path: &Path,
    self_node_id: &str,
    incoming: Vec<PeerEntry>,
) -> Result<usize> {
    // Drop self and any peer we can't actually reach before touching the store.
    let candidates: Vec<PeerEntry> = incoming
        .into_iter()
        .filter(|p| p.node_id != self_node_id)
        .filter(|p| p.iroh_endpoint_addr().is_ok())
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }
    let mut store = PeersStore::load_or_default(peers_path)
        .context("reload peers.json for membership merge")?;
    let added = store.merge_unknown(candidates)?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(node_id: &str) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            alias: None,
            // A bare relay url so `iroh_endpoint_addr` resolves (reachable).
            relay_url: Some("https://relay.example/".to_string()),
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// A real node id (valid public key) so `iroh_endpoint_addr` parsing passes
    /// even without a relay url.
    fn real_node_id() -> String {
        iroh::SecretKey::generate().public().to_string()
    }

    #[test]
    fn parse_membership_ignores_op_announce() {
        // The untagged op-announce format must NOT parse as membership.
        let announce = "my-workspace\nsome-actor\n{\"physical_ms\":1}";
        assert!(parse_membership(announce).is_none());
    }

    #[test]
    fn parse_membership_decodes_tagged_payload() {
        let peers = vec![entry("abc")];
        let json = serde_json::to_string(&peers).unwrap();
        let msg = format!("{MEMBERSHIP_TAG}\n{json}");
        let decoded = parse_membership(&msg)
            .expect("is membership")
            .expect("decodes");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].node_id, "abc");
    }

    #[test]
    fn parse_membership_tag_prefix_of_longer_token_is_not_membership() {
        // A workspace slug that merely starts with the tag text (no newline) is
        // an op-announce, not a membership message.
        let msg = format!("{MEMBERSHIP_TAG}-not-really\nactor\nhlc");
        assert!(parse_membership(&msg).is_none());
    }

    #[test]
    fn merge_skips_self() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();
        let mut e = entry(&me);
        e.relay_url = None; // force resolution via node id only
        let added = merge_membership(&path, &me, vec![e]).unwrap();
        assert_eq!(added, 0, "must never add self");
    }

    #[test]
    fn merge_adds_unknown_and_dedups_known() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();

        let p1 = real_node_id();
        let added = merge_membership(&path, &me, vec![entry(&p1)]).unwrap();
        assert_eq!(added, 1, "p1 is new");

        // Re-merging p1 plus a new p2: only p2 is added.
        let p2 = real_node_id();
        let added = merge_membership(&path, &me, vec![entry(&p1), entry(&p2)]).unwrap();
        assert_eq!(added, 1, "p1 already known, only p2 added");

        let store = PeersStore::load_or_default(&path).unwrap();
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn merge_skips_unreachable_peer() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();
        // node_id that won't parse as a public key AND no relay → unreachable.
        let mut bad = entry("not-a-real-key");
        bad.relay_url = None;
        let added = merge_membership(&path, &me, vec![bad]).unwrap();
        assert_eq!(added, 0, "an unreachable peer is never stored");
    }
}
