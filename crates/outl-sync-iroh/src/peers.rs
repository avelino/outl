//! Trusted peer store — read/written at `~/.outl/peers.json`.

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single trusted peer entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// Peer's iroh node id (hex-encoded public key).
    pub node_id: String,
    /// Human-readable label (e.g. "iPhone 15").
    pub alias: Option<String>,
    /// Relay URL for initial contact (may be outdated; iroh uses it as a hint).
    ///
    /// Kept for display + back-compat with peers.json files written before the
    /// full `endpoint_addr` field existed. The relay URL captured here is a
    /// subset of what `endpoint_addr` carries; prefer the latter for dialing.
    pub relay_url: Option<String>,
    /// Peer's **full** [`iroh::EndpointAddr`] (node id + relay URL + direct
    /// socket addrs), base64-encoded JSON.
    ///
    /// This is the field that makes device↔device connect reliable: it carries
    /// the peer's direct addresses (so two devices on the same LAN connect
    /// immediately) and its home relay, captured at pairing time *after* the
    /// endpoint came online. Older peers.json entries (pre-0.6) lack it and
    /// deserialize as `None` (see `#[serde(default)]`); the dial then falls back
    /// to node id + `relay_url`, then to a bare id.
    #[serde(default)]
    pub endpoint_addr: Option<String>,
    /// ISO 8601 timestamp when pairing occurred.
    pub added_at: String,
}

/// Base64-encode an [`iroh::EndpointAddr`] (as JSON) for storage in a
/// [`PeerEntry`] or a pairing ticket.
pub fn encode_endpoint_addr(addr: &iroh::EndpointAddr) -> Result<String> {
    let json = serde_json::to_vec(addr).context("serialize endpoint addr")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

/// Decode a base64 JSON [`iroh::EndpointAddr`] produced by
/// [`encode_endpoint_addr`].
pub fn decode_endpoint_addr(encoded: &str) -> Result<iroh::EndpointAddr> {
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("base64-decode endpoint addr")?;
    serde_json::from_slice(&json).context("deserialize endpoint addr")
}

impl PeerEntry {
    /// Build a [`PeerEntry`] from a peer's full [`iroh::EndpointAddr`], stamping
    /// `added_at` with the current wall-clock time.
    ///
    /// Captures the full reachability info (relay + direct addrs) so a later
    /// dial does not depend on n0 discovery resolving a bare node id.
    pub fn from_endpoint_addr(addr: &iroh::EndpointAddr, alias: Option<String>) -> Result<Self> {
        Ok(Self {
            node_id: addr.id.to_string(),
            alias,
            relay_url: addr.relay_urls().next().map(|u| u.to_string()),
            endpoint_addr: Some(encode_endpoint_addr(addr)?),
            added_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Parse the node id back to an iroh `EndpointId`.
    pub fn iroh_node_id(&self) -> Result<iroh::EndpointId> {
        self.node_id.parse().context("parse iroh EndpointId")
    }

    /// Build a full [`iroh::EndpointAddr`] for dialing this peer.
    ///
    /// Resolution order, most-reachable first:
    /// 1. The stored full `endpoint_addr` (node id + relay + **direct addrs**) —
    ///    captured at pairing time after the endpoint came online. Two devices
    ///    on the same LAN connect via the direct addrs without any relay or n0
    ///    discovery round-trip — this is what fixes the "offline dot" bug.
    /// 2. Node id + `relay_url` (legacy entries, or if the full addr won't
    ///    decode) — connecting via the relay still beats a bare id.
    /// 3. Bare node id — last resort, relies on n0 discovery resolving a route.
    pub fn iroh_endpoint_addr(&self) -> Result<iroh::EndpointAddr> {
        let id = self.iroh_node_id()?;
        // Dial keys, not IPs. When we have a relay, dial node_id + relay ONLY.
        // Pairing-time direct addrs go stale on every network change (peer on
        // 192.168.x while we're on 10.x is unreachable), and iroh's multipath
        // stalls for MINUTES retrying those dead paths (`MultipathNotNegotiated`
        // + timeout) before it gives up. The relay is the stable contact point;
        // iroh hole-punches the peer's CURRENT direct addrs once the relay link
        // is up. Stored direct addrs are used only when there's no relay (the
        // loopback integration tests, which dial 127.0.0.1 with no relay).
        if let Some(relay) = &self.relay_url {
            match relay.parse::<iroh::RelayUrl>() {
                Ok(url) => return Ok(iroh::EndpointAddr::new(id).with_relay_url(url)),
                Err(e) => tracing::debug!("ignoring unparseable relay url {relay:?}: {e}"),
            }
        }
        match &self.endpoint_addr {
            Some(encoded) => match decode_endpoint_addr(encoded) {
                Ok(a) => Ok(a),
                Err(e) => {
                    tracing::warn!("stored endpoint_addr won't decode, falling back: {e}");
                    Ok(iroh::EndpointAddr::new(id))
                }
            },
            None => Ok(iroh::EndpointAddr::new(id)),
        }
    }
}

/// Persisted list of trusted peers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PeersFile {
    peers: Vec<PeerEntry>,
}

/// In-memory peer store backed by `~/.outl/peers.json`.
pub struct PeersStore {
    path: PathBuf,
    inner: PeersFile,
}

impl PeersStore {
    /// Load the peer store from `path`, or start with an empty list.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        let inner = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("read peers.json from {}", path.display()))?;
            serde_json::from_str(&text).context("parse peers.json")?
        } else {
            PeersFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            inner,
        })
    }

    /// List all trusted peers.
    pub fn list(&self) -> &[PeerEntry] {
        &self.inner.peers
    }

    /// Path this store reads from / writes to (`~/.outl/peers.json` in prod).
    ///
    /// The transport keeps the path so its catch-up loop can reload peers added
    /// by pairing *after* the transport booted (pairing writes this same file).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Add a peer and persist to disk.
    pub fn add(&mut self, entry: PeerEntry) -> Result<()> {
        // Deduplicate by node_id.
        self.inner.peers.retain(|p| p.node_id != entry.node_id);
        self.inner.peers.push(entry);
        self.save()
    }

    /// Merge a batch of peers, adding **only** node_ids not already present, and
    /// persist once if anything changed. Returns the number of peers added.
    ///
    /// Unlike [`add`](Self::add), this never overwrites an existing entry: a
    /// node_id already known keeps its current `PeerEntry` (its locally-captured
    /// `endpoint_addr`, e.g. from direct pairing, may be fresher than a gossiped
    /// one). This is the merge primitive membership auto-discovery uses — it only
    /// ever *adds reachability* for peers already inside the mesh.
    pub fn merge_unknown(
        &mut self,
        incoming: impl IntoIterator<Item = PeerEntry>,
    ) -> Result<usize> {
        let mut added = 0usize;
        for entry in incoming {
            if self.inner.peers.iter().any(|p| p.node_id == entry.node_id) {
                continue;
            }
            self.inner.peers.push(entry);
            added += 1;
        }
        if added > 0 {
            self.save()?;
        }
        Ok(added)
    }

    /// Remove a peer by node_id prefix. Returns true if found.
    pub fn remove(&mut self, node_id_prefix: &str) -> Result<bool> {
        let before = self.inner.peers.len();
        self.inner
            .peers
            .retain(|p| !p.node_id.starts_with(node_id_prefix));
        let removed = self.inner.peers.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.inner)?;
        std::fs::write(&self.path, text)
            .with_context(|| format!("write peers.json to {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real, encodable `EndpointAddr` carrying a relay + a direct addr,
    /// so the stored `endpoint_addr` blob round-trips and we can prove the
    /// resolution order picks relay-only OVER the stored direct addr.
    fn addr_with_relay_and_direct() -> iroh::EndpointAddr {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://relay.example/".parse().expect("relay url");
        iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("192.168.7.7:4242".parse().expect("direct addr"))
    }

    /// Bug #6 (reachability resolution, relay branch): when a relay URL is
    /// present, `iroh_endpoint_addr` must dial node_id + relay ONLY and DROP the
    /// stored direct addrs (they go stale on every network change and iroh's
    /// multipath stalls for minutes on a dead direct path). The relay is the
    /// stable contact point; iroh hole-punches the peer's *current* direct addrs
    /// once the relay link is up.
    #[test]
    fn iroh_endpoint_addr_prefers_relay_only_and_drops_stale_direct_addrs() {
        let full = addr_with_relay_and_direct();
        let entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");

        // Sanity: the stored full addr really does carry a direct addr...
        let stored = decode_endpoint_addr(entry.endpoint_addr.as_ref().expect("has endpoint_addr"))
            .expect("decode stored addr");
        assert!(
            stored.ip_addrs().next().is_some(),
            "fixture must carry a stored direct addr to prove it's dropped"
        );

        // ...but the resolved dial addr is relay-only: same node id, the relay,
        // and NO direct ip addresses.
        let resolved = entry.iroh_endpoint_addr().expect("resolve");
        assert_eq!(
            resolved.id, full.id,
            "resolved must target the same node id"
        );
        assert_eq!(
            resolved.relay_urls().count(),
            1,
            "resolved must carry the relay url"
        );
        assert_eq!(
            resolved.ip_addrs().count(),
            0,
            "resolved must DROP the stale stored direct addrs when a relay is present"
        );
    }

    /// Bug #6 (reachability resolution, no-relay branch): with no relay url, fall
    /// back to the stored full `endpoint_addr` (the loopback case — direct addrs
    /// are all we have, e.g. the integration tests dialing 127.0.0.1). Those
    /// stored direct addrs MUST survive here, since there's no relay to prefer.
    #[test]
    fn iroh_endpoint_addr_falls_back_to_stored_addrs_when_no_relay() {
        let full = addr_with_relay_and_direct();
        let mut entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");
        // No relay → the resolution order must drop through to endpoint_addr.
        entry.relay_url = None;

        let resolved = entry.iroh_endpoint_addr().expect("resolve");
        assert_eq!(resolved.id, full.id);
        assert!(
            resolved.ip_addrs().next().is_some(),
            "with no relay, the stored direct addrs must be used to dial"
        );
    }

    /// Bug #6 (last resort): no relay AND no stored `endpoint_addr` → dial the
    /// bare node id (relies on n0 discovery). Must still resolve, never error.
    #[test]
    fn iroh_endpoint_addr_falls_back_to_bare_node_id() {
        let id = iroh::SecretKey::generate().public();
        let entry = PeerEntry {
            node_id: id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let resolved = entry.iroh_endpoint_addr().expect("resolve bare id");
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.relay_urls().count(), 0);
        assert_eq!(resolved.ip_addrs().count(), 0);
    }

    /// Bug #6 (corrupt blob never fails the dial): a garbage `endpoint_addr` that
    /// won't decode must fall through to the bare node id with a warning, not
    /// propagate an error that would skip the peer entirely.
    #[test]
    fn iroh_endpoint_addr_recovers_from_corrupt_stored_addr() {
        let id = iroh::SecretKey::generate().public();
        let entry = PeerEntry {
            node_id: id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: Some("!!! not base64 json !!!".to_string()),
            added_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let resolved = entry
            .iroh_endpoint_addr()
            .expect("corrupt addr must still resolve to a dialable bare id");
        assert_eq!(resolved.id, id);
    }

    /// Bug #8 (membership merge is ADD-only): `merge_unknown` must NEVER clobber a
    /// locally-captured entry (e.g. a direct-pairing `endpoint_addr`) with a
    /// gossiped one for the same node_id. A known node keeps its existing entry.
    #[test]
    fn merge_unknown_never_clobbers_a_known_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("peers.json");
        let node = iroh::SecretKey::generate().public().to_string();

        // Local entry captured at pairing: full endpoint_addr present.
        let local = addr_with_relay_and_direct();
        let mut local_entry =
            PeerEntry::from_endpoint_addr(&local, Some("local".into())).expect("local entry");
        local_entry.node_id = node.clone();

        let mut store = PeersStore::load_or_default(&path).expect("load");
        store.add(local_entry.clone()).expect("seed local entry");

        // A gossiped entry for the SAME node, but stripped of reachability.
        let gossiped = PeerEntry {
            node_id: node.clone(),
            alias: Some("gossiped".into()),
            relay_url: None,
            endpoint_addr: None,
            added_at: "2030-01-01T00:00:00Z".to_string(),
        };
        let added = store.merge_unknown([gossiped]).expect("merge");

        assert_eq!(added, 0, "a known node_id must not be re-added");
        let kept = store
            .list()
            .iter()
            .find(|p| p.node_id == node)
            .expect("entry still present");
        assert_eq!(
            kept.alias.as_deref(),
            Some("local"),
            "merge_unknown must keep the locally-captured entry, not the gossiped one"
        );
        assert!(
            kept.endpoint_addr.is_some(),
            "merge_unknown must NOT clobber the locally-captured endpoint_addr"
        );
    }
}
