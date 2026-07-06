# Relay & NAT traversal

This page explains the one piece of the [iroh P2P transport](sync.md) that *isn't* peer-to-peer: the relay.
It's the part people get nervous about — "wait, my notes go through a server?" — so it's worth being precise about what a relay does, what it can see, what it can't, and why outl will eventually run its own at `relay.outl.app`.

If you just want the short version:

> The relay helps two devices behind NAT find each other and, when they can't connect directly, forwards their **already-encrypted** bytes.
> It never sees your notes.
> Today outl uses the public relays operated by [n0][n0] (iroh's authors); running our own is on the roadmap, and the config is already wired for it.

---

## Why a relay exists at all

Two devices on the open internet with public IPs could just open a QUIC connection and sync.
That's never the real world.

Real devices sit behind NAT — your laptop on home wifi, your phone on LTE, a machine in an office behind a corporate firewall.
None of them has a stable, dialable address.
Two peers behind two different NATs can't simply "connect to each other" because neither knows a reachable address for the other, and the NAT won't accept an unsolicited inbound packet.

This is the problem every P2P system hits, and the solution is decades old (STUN/TURN, WebRTC's ICE, Tailscale's DERP).
iroh's flavor:

1. **A rendezvous point both peers *can* reach** — the relay.
   Every endpoint, on startup, registers its "home relay" with the relay server.
   You saw this in your own log:

   ```
   INFO endpoint{id=35c8fc38bf}:relay-actor: home is now relay https://use1-1.relay.n0.iroh.link./, was None
   ```

   That endpoint just told a public n0 relay "I'm reachable here."

2. **Hole punching** — using the relay to exchange addresses and timing, the two peers fire packets at each other's NATs simultaneously, punching a path open.
   When it works (it usually does), they get a **direct** QUIC connection and the relay drops out of the data path entirely.

3. **Fallback relaying** — when hole punching fails (symmetric NAT, strict firewall), the relay forwards packets between the two peers so sync still works.
   Slower, uses the relay's bandwidth, but never fails closed.

For outl this matters because the whole pitch is *no server*.
The relay is the asterisk on that claim, so the rest of this page is about shrinking that asterisk to exactly its true size — and no bigger.

---

## What the relay can see (and what it can't)

This is the part that decides whether a relay is a privacy problem.

**The relay cannot read your notes.**
Every iroh connection is end-to-end encrypted with QUIC + TLS 1.3, keyed to the peers' identities.
The relay forwards opaque ciphertext.
It has no key, so a relay — n0's or yours — sees encrypted bytes and nothing else.
This is true *today*, on the public n0 relays, with no extra work.

**The relay can see metadata.**
Specifically, when traffic is relayed it observes:

| Visible to the relay | Not visible to the relay |
|----------------------|--------------------------|
| The two endpoint IDs (public keys) talking through it | The content of any op, block, or page |
| Connection timing — when devices sync, how often | Which pages/blocks changed |
| Packet sizes and traffic volume | Page titles, tags, backlinks, anything semantic |
| The IP each endpoint connects *from* | The op log, the CRDT state, the `.md` |

So the honest threat model is: a relay operator can learn **that** two specific devices sync, **when**, and roughly **how much** — never **what**.

And after a successful hole punch, even that shrinks: the data goes peer-to-peer and the relay only ever saw the coordination handshake, not the sync traffic.

---

## Today: n0's public relays

Out of the box, with `[sync] transport = "iroh"`, outl uses the relay infrastructure run by [n0][n0], the team that builds iroh.
Zero setup, works immediately, which is exactly what you want while the P2P transport is proving itself in beta.

The tradeoff is the one the table above describes: **n0 is a third party that can observe sync metadata.**
Not content — metadata.
For most people, most of the time, that's a fine default and the same trust model as using anyone's STUN/TURN/DERP servers.

But "a third party can observe when your devices talk" sits awkwardly next to outl's whole reason for existing: *your data, your machines, no provider in the loop.*
We removed the iCloud dependency to stop trusting Apple with your sync; leaning permanently on n0 for the coordination layer just swaps one third party for another at a different point in the stack.

That's why running our own relay is on the roadmap — not because n0's relays are untrustworthy, but because **"no third party, by default" is the product.**

---

## Roadmap: `relay.outl.app`

The plan is to run an outl-operated relay at `relay.outl.app` and make it the default for the iroh transport.

**What it buys**

- **Independence from n0.**
  Their public relays are best-effort, no SLA.
  A product that ships sync can't depend on infra it doesn't control being up, unthrottled, and free forever.
- **Metadata sovereignty.**
  The "who syncs with whom, and when" metadata stops going to a third party.
  It goes to *us* — and we publish what we log (ideally: nothing persistent).
  Same spirit as owning the relay instead of renting trust.
- **Region & latency.**
  A relay close to the user base lowers coordination latency.
  (Less important than it sounds, since most traffic ends up direct, but it helps first contact and the fallback path.)
- **The domain is already ours.**
  `outl.app` is owned; `relay.outl.app` is a DNS record and a box.

**What it does *not* buy**

- **Not more content privacy.**
  Content is already E2E encrypted on n0's relays.
  Running our own changes *who sees metadata*, not *whether content is private*.
  If a page reaches a relay in the clear, that's a bug, not a relay feature — it never happens.
- **Not full self-hosting on its own.**
  iroh has *two* services: the **relay** (NAT traversal + fallback) and **discovery** (mapping an endpoint ID to its current addresses, via pkarr / a DNS server).
  `relay.outl.app` covers the relay.
  Endpoint discovery is a separate service; self-hosting that too is a later step.
  Relay is the 80/20.

**When**

Not now.
The structure is ready (see below), but a relay is an always-on service — a VPS, TLS certs, monitoring, bandwidth, uptime.
That's ops overhead that buys nothing while the beta has a handful of users and n0's relays work fine for validating the sync itself.

The trigger to actually stand it up is one of:

- real users depend on sync and n0's best-effort relays become a liability, or
- n0 starts rate-limiting / gets flaky, or
- "no third party in your sync, by default" becomes a marketing/product line we want to make literally true.

---

## It's already a config flip

None of this needs a code change when the day comes — the wiring is already in place.
The iroh transport reads the relay URL from config and binds the long-lived sync endpoint against it:

```toml
[sync]
transport = "iroh"
relay_url = "https://relay.outl.app"   # empty / omitted = n0 default relays
```

Leave `relay_url` empty and you get n0's public relays (today's default).
Point it at `relay.outl.app` once it's up, and that endpoint uses our relay for home registration, hole-punch coordination, and fallback.

How it threads through: each client reads `[sync] relay_url` from the global config and passes it to `IrohSyncTransport::new`, which hands it to the endpoint builder (`bind::n0_builder_ipv4_only`).
`None` (or empty) keeps iroh's `presets::N0` relay; `Some(url)` swaps in `RelayMode::Custom`.
A malformed URL logs a warning and falls back to the n0 default rather than failing the bind, so a typo degrades gracefully instead of taking sync down.
Only the long-lived **sync** endpoint honors the custom relay today; the short-lived pairing / status / test endpoints stay on the n0 default (a custom-relay deployment cares about convergence, which the sync endpoint owns).

iroh also supports **mixing**: you can run your own relay *and* keep n0's as a fallback.
outl's current wiring sets a single custom relay (`RelayMode::Custom` with one URL).
Broadening that to a custom-plus-n0 list is a one-line change in `bind::n0_builder_ipv4_only` if we want it, so standing up `relay.outl.app` doesn't have to be a single point of failure on day one.

---

## Self-hosting a relay (when we get there)

For completeness, the shape of standing one up — this is the future-us checklist, not something you need today:

1. **Run `iroh-relay`** on a small always-on VPS.
   It's the relay server binary from the iroh project; one process, modest resources for op-log-sized traffic.
2. **DNS.**
   Point `relay.outl.app` (A / AAAA) at the box.
3. **TLS.**
   The relay terminates HTTPS; use ACME / Let's Encrypt for certs so the relay URL is `https://relay.outl.app`.
   (Confirm the exact `iroh-relay` flags against the current iroh release — the relay server's config surface moves faster than this doc.)
4. **Config.**
   Ship `relay_url = "https://relay.outl.app"` as the default in the iroh transport, keeping n0 as documented fallback.
5. **(Later) Discovery.**
   Stand up endpoint discovery (pkarr publisher / DNS) for full independence from n0's name resolution, not just its relays.

---

## Troubleshooting: restrictive networks (VPN, TLS-inspection proxy)

Corporate / deep-inspection networks and VPNs break relay connectivity in three distinct ways.
The first is fixed in code; the other two are environmental and need a config or pairing change.

**TLS handshake fails with `UnknownIssuer` (fixed).**
A network with a custom root CA in the OS keychain (a TLS-inspection proxy) used to have every relay handshake rejected.
iroh trusted only Mozilla's bundled roots, not your OS trust store, even though macOS / `curl` / Safari accepted the same cert.
outl now delegates relay-TLS trust to the OS keychain (`rustls-platform-verifier`, wired in `bind::n0_builder_ipv4_only`), so a keychain-trusted proxy cert is accepted.
No action needed.

**Relay WebSocket upgrade returns `502` (self-host the workaround).**
Some proxies allow plain HTTPS but block or rewrite the HTTP `Upgrade: websocket` the relay needs, so the connection fails with `expected HTTP 101 Switching Protocols, got status code 502`.
iroh 1.0.0 has no non-WebSocket relay transport to fall back to, so there is no code fix.
Point `[sync] relay_url` at a relay on a domain/port the proxy doesn't intercept (see "It's already a config flip" above) — a self-hosted `iroh-relay` on your own domain sidesteps the interception.

**Direct LAN sync stalls after pairing on a VPN (`MultipathNotNegotiated`).**
If you pair two devices while one is on a VPN, the captured peer address in `peers.json` picks up the VPN's tunnel IPs (`10.x`, `100.x` CGNAT, a public WAN addr) alongside the real `192.168.x` LAN address.
outl now drops direct addresses that aren't on any local subnet before dialing, so stale tunnel IPs no longer stall the connect.
If you still hit it on an old `peers.json`, **re-pair with the VPN off** for the cleanest capture, or manually strip the non-LAN `"Ip"` entries (keep only `192.168.x` and the `Relay` entry).

---

## See also

- [Sync, done right](sync.md) — the transport layer, the op log, the CRDT, and how the iroh transport plugs into `outl-actions::SyncTransport`.
- [Privacy](privacy.md) — what leaves your device and what never does.
- [Configuration](config.md) — the full `[sync]` config surface.

[n0]: https://n0.computer
[iroh]: https://www.iroh.computer
