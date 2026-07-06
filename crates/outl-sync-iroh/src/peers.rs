//! Trusted peer store — read/written at `<workspace>/.outl/peers.json`.
//!
//! The paired-peer list belongs to the **graph** (workspace), not the OS:
//! pairing device B into workspace X must not silently expose B to workspace Y
//! on the same machine. The device *identity* (`identity.key`) stays global —
//! one node id per device — but the trust list is per-workspace.

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

/// One local IPv4 interface (address + netmask), used to decide whether a peer's
/// stored direct addr shares a subnet with this machine — i.e. is reachable over
/// the LAN at all. See [`is_reachable_lan_ipv4`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalV4 {
    ip: Ipv4Addr,
    mask: Ipv4Addr,
}

/// Enumerate this machine's IPv4 interfaces (address + netmask).
///
/// Returns an empty `Vec` on any enumeration error — callers treat "no known
/// interfaces" as **fail-open** (keep every IPv4 direct addr, the pre-filter
/// behaviour) rather than dropping reachability on a transient syscall failure.
///
/// This walks the OS interface list (`getifaddrs`), so a caller resolving a
/// **batch** of peers (the catch-up resolver, `force_sync_all`) should call it
/// **once** and pass the result to each
/// [`PeerEntry::iroh_endpoint_addr_with_ifaces`], instead of letting the
/// per-peer [`PeerEntry::iroh_endpoint_addr`] re-enumerate on every entry.
pub(crate) fn local_v4_ifaces() -> Vec<LocalV4> {
    match if_addrs::get_if_addrs() {
        Ok(ifaces) => ifaces
            .into_iter()
            .filter_map(|iface| match iface.addr {
                if_addrs::IfAddr::V4(v4) => Some(LocalV4 {
                    ip: v4.ip,
                    mask: v4.netmask,
                }),
                if_addrs::IfAddr::V6(_) => None,
            })
            .collect(),
        Err(e) => {
            warn!("could not enumerate local interfaces ({e}); keeping all IPv4 direct addrs");
            Vec::new()
        }
    }
}

/// Does `peer` share a subnet with `local` (i.e. `peer & mask == local & mask`)?
fn same_subnet_v4(peer: Ipv4Addr, local: &LocalV4) -> bool {
    let mask = u32::from(local.mask);
    (u32::from(peer) & mask) == (u32::from(local.ip) & mask)
}

/// Is `peer` on the same LAN subnet as any local interface?
///
/// This is the load-bearing filter for a peer's **stored** direct addrs: a
/// direct addr on a subnet no local interface belongs to can only ever be a
/// stale capture (a VPN/tunnel IP grabbed at pairing time, `100.x` CGNAT, a
/// public WAN addr, …). Dialing it can never establish a direct path — the
/// relay already covers cross-network reachability — but iroh 1.0.0's multipath
/// opens a path to it anyway and stalls the whole connect on the dead path
/// (`MultipathNotNegotiated`, ~30s). Dropping it loses nothing and removes the
/// stall.
///
/// **Loopback (`127.0.0.0/8`) is always kept**, independent of the interface
/// list, so loopback dials (tests, same-host peers) never drop even if the OS
/// enumeration omits `lo0` or reports it with an odd netmask.
/// **`ifaces` empty ⇒ keep everything** (fail-open — see [`local_v4_ifaces`]).
fn is_reachable_lan_ipv4(peer: Ipv4Addr, ifaces: &[LocalV4]) -> bool {
    peer.is_loopback()
        || ifaces.is_empty()
        || ifaces.iter().any(|iface| same_subnet_v4(peer, iface))
}

/// Build the per-workspace peers path: `<workspace_root>/.outl/peers.json`.
///
/// The `.outl/` directory already holds the workspace's `workspace-id` and
/// `config.toml`, so the peer list sits next to the other graph-scoped state.
pub fn workspace_peers_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".outl").join("peers.json")
}

/// One-time migration: when a workspace has no `peers.json` yet but the legacy
/// **global** `~/.outl/peers.json` exists, copy the global list into the
/// workspace so an already-paired user keeps their peers after the move from
/// device-global to per-workspace storage.
///
/// Best-effort: any failure is logged and swallowed (the workspace just starts
/// with an empty peer list, recoverable by re-pairing). The global file is
/// **never** deleted — other not-yet-migrated workspaces may still read it, and
/// a fresh per-workspace copy is the safe outcome on a partial migration.
///
/// Idempotent: once the workspace file exists this is a no-op, so the copy
/// happens exactly once per workspace and later edits stay workspace-local.
pub fn migrate_global_peers_if_absent(workspace_root: &Path) {
    let dest = workspace_peers_path(workspace_root);
    if dest.exists() {
        return;
    }
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let global = home.join(".outl").join("peers.json");
    if !global.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!(
                "peers migration: create {} failed, starting empty: {e}",
                parent.display()
            );
            return;
        }
    }
    match std::fs::copy(&global, &dest) {
        Ok(_) => debug!(
            "peers migration: copied {} -> {}",
            global.display(),
            dest.display()
        ),
        Err(e) => warn!(
            "peers migration: copy {} -> {} failed, starting empty: {e}",
            global.display(),
            dest.display()
        ),
    }
}

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
    ///
    /// Stored direct addrs are additionally filtered to the local machine's
    /// current LAN: an IPv4 addr on a subnet no local interface belongs to (a
    /// stale VPN/tunnel IP captured at pairing time) is dropped, because dialing
    /// it can never form a direct path yet stalls iroh's multipath. See
    /// `iroh_endpoint_addr_with_ifaces` for the injectable, unit-tested filter.
    pub fn iroh_endpoint_addr(&self) -> Result<iroh::EndpointAddr> {
        self.iroh_endpoint_addr_with_ifaces(&local_v4_ifaces())
    }

    /// [`iroh_endpoint_addr`](Self::iroh_endpoint_addr) with the local IPv4
    /// interface list injected, so the LAN-reachability filter is unit-testable
    /// without depending on the host's real network config.
    ///
    /// Batch callers (the catch-up resolver, `force_sync_all`) call
    /// [`local_v4_ifaces`] once and pass the result here for every peer, so the
    /// interface list is enumerated once per pass instead of once per peer.
    pub(crate) fn iroh_endpoint_addr_with_ifaces(
        &self,
        ifaces: &[LocalV4],
    ) -> Result<iroh::EndpointAddr> {
        let id = self.iroh_node_id()?;
        // Dial the relay AND the IPv4 direct addrs. On the same LAN the direct
        // IPv4 path connects without touching the relay — which is what saves
        // the sync when the public relay is flaky (its ping times out). The
        // relay stays as the cross-network fallback.
        //
        // We DROP IPv6 direct addrs: a dead global-IPv6 path stalls iroh's
        // multipath for minutes (`MultipathNotNegotiated` + timeout), while a
        // stale IPv4 fails fast (No route to host) and yields to the relay. The
        // IPv4-only endpoint bind means we rarely even store an IPv6 path; this
        // filter is belt-and-suspenders for older `peers.json` entries captured
        // before that bind. (The old code dialed relay-only to dodge the IPv6
        // stall, but that also threw away the LAN-direct path and made every
        // connect hostage to the relay.)
        //
        // We ALSO drop IPv4 direct addrs that are NOT on any local subnet: a
        // peer paired while on a VPN captures its tunnel IPs (`10.x`, `100.x`
        // CGNAT, a public WAN addr) into `endpoint_addr` alongside the real LAN
        // address. Those are unreachable from this machine's LAN, but iroh 1.0.0
        // opens a multipath path to each anyway and stalls the whole connect on
        // the dead paths — even when the real `192.168.x` addr is right there.
        // `is_reachable_lan_ipv4` keeps only addrs sharing a subnet with a local
        // interface; the relay still covers genuine cross-network peers.
        let relay = self
            .relay_url
            .as_ref()
            .and_then(|r| r.parse::<iroh::RelayUrl>().ok());

        if let Some(encoded) = &self.endpoint_addr {
            match decode_endpoint_addr(encoded) {
                Ok(stored) => {
                    let mut addr = iroh::EndpointAddr::new(id);
                    if let Some(url) = relay.clone() {
                        addr = addr.with_relay_url(url);
                    }
                    for sock in stored.ip_addrs() {
                        if matches!(sock, SocketAddr::V4(v4) if is_reachable_lan_ipv4(*v4.ip(), ifaces))
                        {
                            addr = addr.with_ip_addr(*sock);
                        }
                    }
                    return Ok(addr);
                }
                Err(e) => tracing::warn!("stored endpoint_addr won't decode, falling back: {e}"),
            }
        }
        // No full addr: dial node_id + relay only (iroh hole-punches current
        // direct addrs once the relay link is up). Bare id when there's no relay
        // either (loopback tests dial 127.0.0.1 via the stored addr above).
        match relay {
            Some(url) => Ok(iroh::EndpointAddr::new(id).with_relay_url(url)),
            None => Ok(iroh::EndpointAddr::new(id)),
        }
    }
}

/// Persisted list of trusted peers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PeersFile {
    peers: Vec<PeerEntry>,
}

/// In-memory peer store backed by `<workspace>/.outl/peers.json`.
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

    /// Path this store reads from / writes to
    /// (`<workspace>/.outl/peers.json` in prod).
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

    /// A fake local interface, so the LAN-reachability filter is deterministic
    /// regardless of the host running the test.
    fn iface(ip: &str, mask: &str) -> LocalV4 {
        LocalV4 {
            ip: ip.parse().expect("iface ip"),
            mask: mask.parse().expect("iface mask"),
        }
    }

    /// The `192.168.7.0/24` LAN the `addr_with_relay_and_direct` direct addr
    /// (`192.168.7.7`) lives on, so it survives the reachability filter.
    fn lan_192_168_7() -> Vec<LocalV4> {
        vec![iface("192.168.7.1", "255.255.255.0")]
    }

    /// Issue #3: `same_subnet_v4` / `is_reachable_lan_ipv4` — the pure filter
    /// that drops stale VPN/tunnel IPv4 while keeping same-LAN and loopback.
    #[test]
    fn is_reachable_lan_ipv4_matches_only_local_subnets() {
        let ifaces = vec![
            iface("192.168.1.50", "255.255.255.0"), // home WiFi
            iface("127.0.0.1", "255.0.0.0"),        // loopback
        ];
        // Same LAN + loopback: reachable.
        assert!(is_reachable_lan_ipv4(
            "192.168.1.83".parse().unwrap(),
            &ifaces
        ));
        assert!(is_reachable_lan_ipv4("127.0.0.1".parse().unwrap(), &ifaces));
        // VPN / CGNAT / WAN captured on another network: not reachable.
        assert!(!is_reachable_lan_ipv4(
            "10.71.22.9".parse().unwrap(),
            &ifaces
        ));
        assert!(!is_reachable_lan_ipv4(
            "100.78.230.122".parse().unwrap(),
            &ifaces
        ));
        assert!(!is_reachable_lan_ipv4(
            "188.37.137.132".parse().unwrap(),
            &ifaces
        ));
        // Empty iface list ⇒ fail-open (keep everything).
        assert!(is_reachable_lan_ipv4("10.71.22.9".parse().unwrap(), &[]));
        // Loopback is kept even when the iface list has NO loopback entry —
        // the allow-list is explicit, not dependent on enumeration.
        let wifi_only = vec![iface("192.168.1.50", "255.255.255.0")];
        assert!(is_reachable_lan_ipv4(
            "127.0.0.1".parse().unwrap(),
            &wifi_only
        ));
    }

    /// Bug #6 (reachability resolution, relay branch): with a relay present, the
    /// dial addr keeps the relay AND the IPv4 direct addrs — so a same-LAN peer
    /// connects directly without the (possibly flaky) relay — but DROPS global
    /// IPv6 direct addrs, because a dead one stalls iroh's multipath for minutes.
    /// (Earlier this was relay-only, which threw away the LAN-direct path and
    /// made every connect hostage to the relay.)
    #[test]
    fn iroh_endpoint_addr_keeps_relay_and_ipv4_but_drops_ipv6() {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://relay.example/".parse().expect("relay url");
        let full = iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("192.168.7.7:4242".parse().expect("ipv4")) // kept
            .with_ip_addr("[2001:db8::1]:4242".parse().expect("ipv6")); // dropped
        let entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");

        // Inject the peer's LAN as a local interface so the IPv4 direct addr is
        // reachable and the assertion isolates the IPv6-drop behaviour.
        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&lan_192_168_7())
            .expect("resolve");
        assert_eq!(resolved.id, id, "resolved must target the same node id");
        assert_eq!(
            resolved.relay_urls().count(),
            1,
            "resolved must keep the relay url"
        );
        let ips: Vec<_> = resolved.ip_addrs().copied().collect();
        assert_eq!(ips.len(), 1, "resolved keeps exactly the IPv4 direct addr");
        assert!(
            ips[0].is_ipv4(),
            "the surviving direct addr is the IPv4 one"
        );
    }

    /// Issue #3 (stale VPN/tunnel IPs in `peers.json`): a peer paired while on a
    /// VPN captured its tunnel IPs alongside the real LAN address. On resolution
    /// the dial must keep ONLY the same-LAN direct addr (`192.168.1.83`) and the
    /// relay, dropping every unreachable tunnel/CGNAT/WAN IPv4 — otherwise iroh's
    /// multipath stalls on the dead paths (`MultipathNotNegotiated`).
    #[test]
    fn iroh_endpoint_addr_drops_stale_vpn_ipv4_keeps_lan() {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://euc1-1.relay.n0.iroh.link./"
            .parse()
            .expect("relay url");
        // The exact `endpoint_addr` payload from the issue: one real LAN addr
        // buried among five stale tunnel/CGNAT/WAN addrs.
        let full = iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("10.71.22.9:62858".parse().unwrap())
            .with_ip_addr("100.78.230.122:62858".parse().unwrap())
            .with_ip_addr("100.85.18.51:62858".parse().unwrap())
            .with_ip_addr("188.37.137.132:62858".parse().unwrap())
            .with_ip_addr("192.0.0.6:62858".parse().unwrap())
            .with_ip_addr("192.168.1.83:62858".parse().unwrap());
        let entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");

        // This machine is on the same home WiFi as the peer.
        let ifaces = vec![
            iface("192.168.1.50", "255.255.255.0"),
            iface("127.0.0.1", "255.0.0.0"),
        ];
        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&ifaces)
            .expect("resolve");

        let ips: Vec<_> = resolved.ip_addrs().copied().collect();
        assert_eq!(
            ips.len(),
            1,
            "only the same-LAN direct addr survives, got {ips:?}"
        );
        assert_eq!(
            ips[0],
            "192.168.1.83:62858".parse().unwrap(),
            "the survivor is the reachable LAN addr"
        );
        assert_eq!(
            resolved.relay_urls().count(),
            1,
            "the relay stays as the cross-network fallback"
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

        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&lan_192_168_7())
            .expect("resolve");
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

    /// The per-workspace path is `<root>/.outl/peers.json` — next to the
    /// workspace-id + config.toml the `.outl/` dir already holds.
    #[test]
    fn workspace_peers_path_is_under_dot_outl() {
        let root = std::path::Path::new("/tmp/ws");
        assert_eq!(
            workspace_peers_path(root),
            std::path::Path::new("/tmp/ws/.outl/peers.json")
        );
    }

    /// Migration is a no-op (idempotent) once the workspace already has a
    /// `peers.json`: it must NEVER clobber the workspace-local list with the
    /// global one. We seed a workspace file, then prove migrate leaves it byte
    /// for byte intact regardless of whatever the global file holds.
    #[test]
    fn migrate_is_noop_when_workspace_peers_exist() {
        let ws = tempfile::tempdir().expect("tempdir");
        let dest = workspace_peers_path(ws.path());
        std::fs::create_dir_all(dest.parent().unwrap()).expect("mkdir .outl");
        let sentinel = r#"{"peers":[{"node_id":"local-only","alias":null,"relay_url":null,"added_at":"2026-01-01T00:00:00Z"}]}"#;
        std::fs::write(&dest, sentinel).expect("seed ws peers");

        migrate_global_peers_if_absent(ws.path());

        let after = std::fs::read_to_string(&dest).expect("read ws peers");
        assert_eq!(
            after, sentinel,
            "migration must not overwrite an existing workspace peers.json"
        );
    }
}
