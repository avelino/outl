# CLAUDE.md — outl-sync-iroh

iroh-based P2P transport for outl.
Implements `outl_actions::SyncTransport` using iroh QUIC + iroh-gossip.

## What this crate owns

- `IrohIdentity` — ed25519 keypair, stored at `~/.outl/identity.key`
- `PeersStore` — known paired peers, stored at `~/.outl/peers.json`
- `IrohSyncTransport` — implements `SyncTransport` trait, including the
  gossip-backed `announce_local_ops` hook (sync side → tokio task via an
  `mpsc` channel set up in `start()`) and the `peer_health()` reachability
  snapshot (see "One endpoint per identity" below)
- Wire protocol — ALPN `b"outl-sync/1"`, vector-clock delta sync
- Pairing (`pairing` module, ALPN `b"outl-sync/pair/1"`) — the two-sided handshake.
  The "ticket" is a base64 `EndpointAddr` (id + relay + direct addrs).
  Both sides exchange one pairing payload (carrying their **full** `EndpointAddr`) over a single bi stream and persist the remote to `peers.json`.
  **Two drivers, one handshake:**
  - **CLI** (`outl peer pair`, no running transport) → `host_pairing` / `join_pairing` bind a one-shot endpoint.
    No relay route to steal.
  - **GUI** (mobile / desktop, transport already running) → `IrohSyncTransport::pair_host` / `pair_join` reuse the **live sync endpoint** (see "One endpoint per identity").
    They never call `host_pairing` / `join_pairing`.
  The endpoint-agnostic handshake halves (`accept_host_handshake` / `run_join_handshake` in `pairing`) are shared by both paths.
  The GUI side is wired through `engine_pairing` (the `PairingHub` + `PairingProtocolHandler` mounted on the sync router).

## Workspace identity is a stable shared id, NOT the path (load-bearing)

**Two paired devices are "the same workspace" because they share one `outl_core::WorkspaceId`, never because their local paths match.**
Devices live at different paths (desktop `~/outl-p2p`, mobile `…/app.outl.mobile-app/outl`); deriving identity from the path made every device compute a different gossip topic, so cross-device gossip never connected and membership never propagated.

- **Home.**
  The id is a ULID persisted at `<root>/.outl/workspace-id` (plaintext, one line), read-or-generated on first transport `start()` via `WorkspaceId::read_or_create`.
  Migration: an existing workspace with no file gets one on first open, stable thereafter.
  It sits in `.outl/` next to `config.toml`/`peers.toml`, so it never pollutes the clean markdown and never reaches a rendered page.
  It is its **own** file, not a field in `.outl/config.toml`, because `config.toml` holds the **per-device** `actor_id` (must differ per device) whereas the workspace id must be **identical** across devices.
  Keeping them separate lets pairing-adoption rewrite the id without touching the device's actor.
- **Gossip topic.**
  `workspace_topic_id` = `blake3(workspace_id)`, so two devices on the same workspace land on the same topic regardless of path.
- **Sync request.**
  `SyncRequest.workspace_id` carries the id, and the serve side (`SyncProtocolHandler::serve`) **validates it against the local id and rejects a mismatch** (`workspace-mismatch` close).
  Genuinely-different workspaces never cross-merge op logs; legit same-workspace peers (now sharing an id) pass.
- **Pairing makes the joiner adopt the host's id.**
  The handshake `PairingPayload` carries each side's id.
  The **host advertises and keeps** its id; the **joiner adopts** the host's *before* the immediate post-pair `delta_sync` fires.
  Adoption is **persist-first**: `adopt_workspace_id` writes the host's id to `.outl/workspace-id` and only then flips the in-memory handle.
  If the disk write fails it does NOT adopt — the pair just doesn't take and a retry is safe.
  A half-adopted state (memory on the host's id, disk on the old one) would silently split the workspace if the process died before the next write, since the next start reads the stale id from disk.
  Both sides then compute the same topic and validate as one workspace, and their op logs CRDT-merge (content from both converges — expected).
  CLI `outl peer pair` neither advertises nor adopts (it edits the device-global `peers.json`, not one open workspace); adoption is a GUI concern, wired in `engine_pairing`.
- **Live handle.**
  The id lives behind a shared `RwLock` (`SharedWorkspaceId`) read at call time by `delta_sync` + serve, so an adopted id takes effect immediately for direct sync (boot connect, 8s catch-up, immediate post-pair dial — all carry the live id).
- **Gossip re-subscribes on id change (no restart).**
  Adoption fires a `tokio::sync::broadcast::Sender<WorkspaceId>` held in the `PairingHub` (`adopt_workspace_id` sends after persisting the file + updating the `RwLock`).
  The gossip supervisor (`engine_gossip::run_gossip`) holds one receiver and, on the signal, **drops its old-topic subscription and re-subscribes to `blake3(new id)`** — same `Gossip`/`Endpoint`, no second endpoint, no `start()` restart.
  Before this, the boot-time subscription stayed on the *pre-adoption* topic, so the two devices sat on different gossip topics and no real-time announce ever crossed (the "post-pair, nothing syncs again" bug).
- **Catch-up re-dials on id change.**
  A second receiver on the **same** broadcast channel (`PairingHub::subscribe_wid_changed`) feeds the catch-up loop; on the signal it **clears its per-session `synced` dedup** so it re-dials every peer under the adopted id.
  Without this, the single immediate post-pair `delta_sync` marked the peer synced and the loop never re-dialed it, so later host edits never pulled (gossip was the only live path, and it was dead per the point above).
  The direct delta-sync paths still carry content even if a signal is dropped (the `RwLock` is the source of truth); the signal only fixes the *real-time* + *re-dial* gaps.

## One endpoint per identity (load-bearing invariant)

**Only endpoints that SERVE the sync ALPN may share a device identity.**
A non-sync endpoint (status probe, a pairing-only endpoint) must NOT bind the device `SecretKey` while the transport is running.

The nuance (verified against the iroh 1.0.0 source — `endpoint.rs::same_endpoint_id_relay` + `iroh-relay` `clients.rs`):

- **Two endpoints with the same `node_id` that BOTH serve `SYNC_ALPN` coexist fine.**
  The relay keeps one `node_id → endpoint` route; the last to register holds it (LIFO), the other loses *inbound* but keeps working via *outbound* dial (catch-up).
  The loser does **not** reconnect-and-steal-back — the relay keeps its socket alive answering pings, so it never times out — so it's a **stable hijack, not a flap**.
  When the holder leaves, the route returns to the other.
  This is what lets the **GUI and the MCP server both bind the device identity at once** (see "Passive writers"): whichever holds the route serves inbound from the same `ops-*.jsonl` on disk, and the other still pushes/pulls on its own dials.
- **An endpoint that does NOT serve `SYNC_ALPN` is the dangerous case.**
  If the newcomer can't accept `SYNC_ALPN`, a dialer routed to it gets `CONNECTION_REFUSED` ("the server refused to accept a new connection") — sync breaks for real.
  That was the original device↔device bug: a transient status-probe endpoint (and, before consolidation, a separate pairing endpoint) stole the route from the sync endpoint and answered nothing.

**Why the route is single at all:** iroh's relay (and discovery) keep a single `node_id → endpoint` route.
When a second endpoint registers with the same secret key, the relay's `DashMap<EndpointId, ClientState>` *replaces* the active client.
All inbound relay datagrams then route to the newcomer and the original endpoint silently stops receiving traffic (iroh ships a test asserting exactly this: `endpoint.rs::same_endpoint_id_relay`).
If that newcomer doesn't accept `SYNC_ALPN`, the dialer's QUIC handshake is refused — **"the server refused to accept a new connection"** (`quinn` `CONNECTION_REFUSED`).
That was the device↔device "connection refused, nothing syncs" bug: a transient status-probe endpoint (and, before this consolidation, the GUI's separate pairing endpoint) stole the relay route from the sync endpoint.
The pairing case was real and observed in device relay logs: while the pairing screen was open the sync endpoint stopped receiving ("Another endpoint connected with the same endpoint id"), and sync recovered the instant the pairing endpoint closed.

**Three call sites, three rules:**

1. **Sync endpoint (`engine::run_iroh`)** — the one allowed long-lived endpoint.
   Router accepts `SYNC_ALPN` + gossip ALPN **+ `PAIRING_ALPN`** (the GUI pairing handler — see rule 3).
   The endpoint advertises both `SYNC_ALPN` and `PAIRING_ALPN` in its `.alpns()`.
   All catch-up / boot / gossip / pairing dials go out through *this* endpoint (the one bound in `run_iroh`); no helper spins up a second.
2. **Status (`status::probe_peers`)** — binds a transient endpoint with the device identity.
   **CLI-only — forbidden from the GUI.**
   The desktop / mobile `outl_peer_status` commands read reachability from the running transport's `peer_health()` instead (see below).
   `probe_peers` survives only for `outl peer status` in the CLI, which has no running transport.
3. **Pairing** — two paths, never a second endpoint while the transport runs:
   - **GUI** (mobile / desktop, transport running) → `IrohSyncTransport::pair_host` / `pair_join` reuse the **live sync endpoint**.
     The host (accept) side is the `PAIRING_ALPN` router handler (`engine_pairing::PairingProtocolHandler`), armed by `pair_host` via a shared `PairingHub`; the join side dials out on the same endpoint.
     After a successful pair the new peer is persisted to `peers.json` and an **immediate** `delta_sync` is fired against it (`engine_pairing::drain_pair_completions`), so it syncs without an app restart and without waiting for the 8s catch-up tick.
   - **CLI** (`outl peer pair`, no running transport) → `pairing::host_pairing` / `join_pairing` bind a one-shot endpoint with the device identity, then **close it** (`endpoint.close().await`) before returning.
     There is no live endpoint to conflict with, so the one-shot bind is safe; the close keeps the overlap with any concurrent endpoint bounded.
     **The GUI never calls these.**

## Status from the transport (`peer_health`)

`IrohSyncTransport` tracks per-peer reachability in an `Arc<Mutex<HashMap<EndpointId, PeerHealth>>>` (the `health` module).
The transport's own dials populate it: the **boot connect**, the **catch-up loop**, and **gossip-triggered sync** each record `record_success(nid, started)` / `record_failure(nid)` on their `delta_sync` outcome — no extra endpoint, no extra dials.

`SyncTransport::peer_health()` (a trait method in `outl-actions`, default `Vec::new()`) returns the snapshot as `Vec<outl_actions::PeerHealthSnapshot>` (`node_id`, `reachable`, `last_rtt_ms`).
`FileSyncTransport` uses the default (no peers).

The GUI status commands read it from the transport stored in Tauri state and merge it onto the full `peers.json` list, so a peer the transport hasn't dialed yet (or the file-transport case) shows offline.
The transport lives in desktop `state.iroh_transport` (`Arc<dyn SyncTransport>`, reached via the trait method) and mobile `state.iroh` (concrete `IrohSyncTransport`).
Desktop also keeps a **concrete** clone in `state.iroh_pairing` (`IrohSyncTransport`) so the pairing commands can call `pair_host` / `pair_join`.
Those aren't `SyncTransport` trait methods — the trait can't return `outl_sync_iroh::PeerEntry` without a dep cycle, and pairing isn't a generic-transport concern.
Both desktop slots are populated together in `wire_iroh_transport` and cleared together on workspace switch; mobile already stores the concrete type, so it reuses `state.iroh` for both status and pairing.
**Never** add a GUI status path that binds an endpoint with the device identity.

## Append-serialization invariant (load-bearing)

**Every op-log append the transport performs goes through one process-wide async append lock (`AppendLock`), held across open+`write_all`+`flush`+`sync_data` of each batch.**

`ingest_received_ops` is the single writer; it opens `ops-<actor>.jsonl` in append mode and writes the received batch.
Three concurrent paths reach it for the **same** file — the boot connect, the 8s catch-up loop, and gossip-triggered sync (all via `delta_sync`) — plus the inbound `serve` side.
Without serialization, two `write_all`s interleave at the syscall layer and glue two ops together with no separating newline (`…}}}{"ts":…`), corrupting the log.
That corruption is real: it was found on disk in an iCloud workspace (45 glued lines among 5261 valid ops).

Rules:

- `delta_sync` (initiator) and `SyncProtocolHandler::serve` (responder) both take/carry the **same** `AppendLock` clone and pass it to `ingest_received_ops`.
  Never add a new writer that appends to `ops-*.jsonl` without taking this lock.
- Write the whole per-actor batch in one `write_all`, then `flush()` + `sync_data()` **before releasing the lock**, so a concurrent reader (or `outl-core`'s `reload`) can never observe a partial line.
- **In-flight guard (defense in depth).**
  A shared `InFlightPeers` set (`try_acquire_in_flight` → RAII `InFlightGuard`) stops boot + catch-up + gossip from launching a second `delta_sync` for a peer that already has one running.
  This cuts redundant relay traffic and the pile-up of writers behind the lock.
  The catch-up loop's per-session `synced` `HashSet` is a separate, complementary dedup (it skips peers already fully reconciled this session).
- Read-side safety net: `outl-core`'s `JsonlStorage::reload` recovers glued lines that already exist on disk (see `docs/storage.md`).
  That recovers historic corruption; the lock prevents new corruption.
  Both are needed.

## Force-sync trigger (`sync_now`)

`SyncTransport::sync_now()` (a trait method in `outl-actions`, default no-op so `FileSyncTransport` is unaffected) lets the GUI force an **immediate** delta-sync pass against every known peer instead of waiting for the 8s catch-up tick.
It backs the mobile pull-to-refresh / refresh button and the desktop Sync panel's Refresh.

Wiring mirrors the `announce_tx` / pairing-hub pattern exactly:

- `IrohSyncTransport` holds a `sync_now_tx: Arc<Mutex<Option<UnboundedSender<()>>>>`, populated in `start()` (and `None` before start / after shutdown — that `Option` is the "runtime down" guard).
- `sync_now()` sends a unit on it; a send error means the runtime is down, so it's ignored (no-op), same contract as `announce_local_ops`.
- `run_iroh` moves the receiver into a `drain_sync_now` task (`engine_catchup`).
  Each signal runs `force_sync_all`, which dials **every** peer in the current `peers.json` (reusing the append lock + in-flight guard + health recording).

**`force_sync_all` deliberately does NOT respect the catch-up loop's per-session `synced` `HashSet`** — the whole point of a manual sync is to re-dial even healthy peers the catch-up loop leaves to gossip.
`delta_sync` is a cheap no-op on matching vector clocks, so a forced re-dial of an already-synced peer just confirms convergence.
It still honors `try_acquire_in_flight` (skip a peer already being dialed) and the `AppendLock`.

The GUI commands are `outl_sync_now` in each client's `commands/peers.rs` (mobile reads `state.iroh`, desktop reads `state.iroh_transport` `Arc<dyn SyncTransport>`).
The shared wrapper is `syncNow()` in `@outl/shared/api/commands`.

## Module layout (delta-sync wire vs. orchestration)

The crate's delta-sync **wire protocol** lives in `engine_sync.rs`:
`delta_sync` (initiator), `SyncProtocolHandler` (responder), and the framing helpers (`read_frame` + typed `read_*`).
It also owns the op-log read/write helpers (`local_vector_clock`, `ops_missing_for`, and `ingest_received_ops`, which owns the `AppendLock` write path).
`engine.rs` keeps **boot orchestration**: the `IrohSyncTransport` struct + channel wiring, `run_iroh`, the boot/catch-up/gossip task spawns, and the router setup.
`engine.rs` re-exports `delta_sync` + `SyncProtocolHandler` (`pub(crate) use crate::engine_sync::…`) so `crate::engine::delta_sync` keeps resolving for `engine_catchup`, `engine_gossip`, `engine_pairing`, and `test_support`.
The split was forced by the file-size guard; treat `engine_sync.rs` as the "on the wire" module and `engine.rs` as the "stand it up" module.

The **gossip supervisor** lives in `engine_gossip.rs` (`run_gossip` + `GossipCtx`).
It is one task that `select!`s over the op-announce drain, the periodic membership broadcast, the inbound gossip stream, AND the `wid_changed` signal — re-subscribing to the new topic on an id change (see "Gossip re-subscribes on id change").
It runs even with zero peers at boot, so a device that pairs later still gets a live subscription via the id-change path (the old inline block only subscribed when boot-time peers existed).

## Mesh membership auto-discovery (gossip)

Without it, a full mesh needs **every pair** of devices hand-paired; ops only reach a non-adjacent device through **transitive propagation** (A↔B↔C reconciles).
Membership gossip closes that: when A pairs with B and B already knows C, A **auto-discovers** C's reachability and then syncs C **directly**.

It lives in `engine_membership.rs` (build / parse / merge) plus the send + receive glue in `engine::run_iroh`'s gossip block.

**Message kind (tagged, back-compat with op-announce).**
The op-announce is the untagged `"workspace_id\nactor\nhlc"` format.
A membership message carries a distinct first line — `MEMBERSHIP_TAG` (`"outl-membership/1"`, versioned) — followed by a JSON array of `PeerEntry` (the same node_id + relay/`endpoint_addr` reachability `peers.json` stores).
The receive side checks `parse_membership` **first**; a non-membership message falls through to the existing announce parser, so the announce path is untouched.

**Broadcast.**
A periodic task (`MEMBERSHIP_INTERVAL`, 5s) reloads `peers.json` and broadcasts the current peer list over the **same** gossip topic.
It reuses the **same** `GossipSender` (wrapped in `Arc`, shared with the announce-drain task — no second topic handle, no second endpoint).
An empty peer list is not broadcast (nothing to share).
The task lives inside the same `if !bootstrap_ids.is_empty()` subscribe block as the announce drain, so a device with zero peers neither subscribes nor gossips.

**Merge / persist flow.**
On receipt, `merge_membership` merges **unknown** peers into `peers.json` through `PeersStore::merge_unknown` (dedup by node_id, ADD-only — an existing entry's locally-captured addr, e.g. from direct pairing, is never clobbered).
The existing catch-up loop reloads `peers.json` each tick and dials the freshly-merged peer; **no new dialing machinery** — membership only *adds reachability*, the append lock / in-flight guard / health map are all reused as-is.

**Trust model (load-bearing assumption).**
Every device subscribed to the workspace gossip topic is **already inside the trust domain**: the topic id is `blake3(workspace_id)` (`workspace_topic_id`), so only devices paired into this mesh by *someone* ever subscribe.
Membership gossip therefore only ever ADDS reachability for peers **already in the mesh** — it never invites a stranger (a non-member can't reach the topic to inject a peer, and a merged peer was already trusted by the member that gossiped it).
Conservative guards on the merge:

- **Never add self** (drop our own node_id from any incoming list).
- **Never add an unreachable peer** (skip an entry whose `PeerEntry::iroh_endpoint_addr` won't resolve — we don't store a peer we can't dial).
- **ADD-only dedup** (a known node_id keeps its current entry).

If membership ever needs to *gate* who can join (beyond "is on the topic"), that's a new trust surface — stop and design it; do not loosen these guards silently.

## Regression suite (Pilar 2)

Every bug hand-found during the sync saga has a NAMED, permanent test that fails red if it ever regresses.
The test name IS the bug, so a failure is self-explanatory.
Pure/deterministic guards live in `#[cfg(test)]` modules next to the code; over-the-wire (real QUIC, loopback) guards live in `tests/regression.rs`.
The shared seed/read/wait helpers stay in `tests/common/mod.rs` (read-only); saga-specific helpers live inside `regression.rs` itself.

| Saga bug | Guard test | Where |
|----------|-----------|-------|
| 1. Op-log corruption from concurrent appends (glued `…}}}{`) — append lock serializes inbound batches | `concurrent_appends_never_glue_ops_on_the_responder` (two initiators → one responder, shared `AppendLock`; asserts no `}{` on disk + every op parses) | `tests/regression.rs` |
| 1. (parser-recovery half) a hand-crafted glued `}}}{` line still loads both ops | `recovers_glued_ops_on_one_line` (pre-existing, core-side) | `outl-core` `storage/jsonl.rs` |
| 2. HLC far-future op skipped on ingest (±24h gate) | `far_future_hlc_op_is_skipped_on_ingest` (B sends a ~48h-ahead op + a valid op; only the valid one lands on A) | `tests/regression.rs` |
| 3. Workspace identity = stable id, not path (topic) | `same_workspace_id_yields_same_topic_across_paths` (pre-existing) | `tests/integration.rs` |
| 3. Workspace identity = stable id, not path (END-TO-END sync) | `different_paths_same_workspace_id_sync_as_one` (two devices at different paths, same id, converge) | `tests/regression.rs` |
| 3. Mismatched ids are rejected | `delta_sync_rejects_mismatched_workspace_id` (pre-existing) | `tests/integration.rs` |
| 4. Pairing adoption (joiner adopts host id, syncs as one) | `gui_pairing_over_live_sync_endpoint` (pre-existing; asserts adopted id persisted in both peers.json) | `tests/integration.rs` |
| 5. Single endpoint per identity (pair AND sync over the live sync endpoint, no relay hijack) | `gui_pairing_over_live_sync_endpoint` (pre-existing; pairing rides the live sync endpoint, no second bind) | `tests/integration.rs` |
| 6. Relay-only dial / reachability resolution order | `iroh_endpoint_addr_prefers_relay_only_and_drops_stale_direct_addrs`, `iroh_endpoint_addr_falls_back_to_stored_addrs_when_no_relay`, `iroh_endpoint_addr_falls_back_to_bare_node_id`, `iroh_endpoint_addr_recovers_from_corrupt_stored_addr` | `src/peers.rs` `#[cfg(test)]` |
| 7. Bidirectional push materializes on BOTH sides AND fires BOTH reload signals | `bidirectional_sync_fires_reload_signal_on_both_sides` (set convergence + `peer_ready_tx` on initiator AND responder) | `tests/regression.rs` |
| 7. (set-convergence half) both sides hold all ops | `bidirectional_delta_sync` (pre-existing) | `tests/integration.rs` |
| 8. Membership merge is ADD-only (never clobber a local entry, drop self, drop undialable) | `merge_unknown_never_clobbers_a_known_entry` (`src/peers.rs`) + `merge_skips_self` / `merge_adds_unknown_and_dedups_known` / `merge_skips_unreachable_peer` (pre-existing) | `src/peers.rs`, `src/engine_membership.rs` `#[cfg(test)]` |

Names map 1:1 to the saga checklist; do NOT delete one without deleting the bug it guards.

### Chaos/concurrency tests (Pilar 3)

The regression suite above pins one named bug per row; the **chaos battery** (`tests/chaos.rs`) instead *hammers* the same wire code with the failure modes a P2P op-log hits in the wild, as STRESS tests (N writers × M ops), not single cases.
It drives the real `delta_sync` initiator + the `SyncProtocolHandler` responder, both via `test_support`.
Every test runs over real QUIC on loopback (no faking, no relay — direct `127.0.0.1` addrs) under `#[tokio::test(flavor = "multi_thread")]`, so the concurrency is genuine.

| Failure mode | Chaos test | What it asserts |
|--------------|-----------|-----------------|
| Concurrent writers corrupting the op log (the `…}}}{` glue) | `concurrent_writers_never_corrupt_op_log` | 8 initiators push ops authored by ONE shared actor at ONE responder; every inbound `serve()` writes the same `ops-<actor>.jsonl` under the responder's single shared `AppendLock`. After: raw bytes show every physical line is exactly one JSON value, AND the op set is the exact union (no loss, no dup blow-up). |
| Reordered + duplicated delivery | `reordered_and_duplicated_delivery_converges` | 3 nodes (each both initiator + responder), 3 rounds of seeded-shuffled directed sync passes, ~half delivered twice. All three converge to the identical op set (dedup-by-id + HLC order holds under stress). |
| Partition + heal under load | `partition_then_heal_under_load` | B is "offline" while A and C make many edits and reconcile; B comes back and converges to the full A+B+C state. No lost ops, no glued line on the healed node. |
| Fan-out + redundant dials | `fan_out_to_many_peers_converges_without_double_dial` | 5 peers dial one hub, each launching its dial TWICE concurrently. Hub converges to the exact union; every op lands once despite the duplicate concurrent dials; no glued line on any actor file. |
| Single-endpoint invariant under concurrency | `concurrent_inbound_dials_on_single_endpoint_stay_clean` | 6 inbound dials land on one hub endpoint WHILE that same endpoint initiates an outbound `delta_sync`. Models the "pair handshake racing a sync on the same endpoint" case (relay-less loopback can't drive the real gossip/pairing swarm). Hub converges across both directions; no corruption. |

**Determinism (load-bearing — a flaky chaos test is worse than none).**
All "randomness" is a *seeded* xorshift (`chaos_helpers::Rng`): shuffles + duplicate counts reproduce from the per-test seed.
Network timing is the only true nondeterminism, and every wait goes through `common::STEP_TIMEOUT` (30s) + `wait_until` polling — never a fixed sleep we race.
Sizes are bounded (≤ 64 ops, ≤ 8 tasks) so the suite stays ~2.5s.

**Why raw bytes, not `all_ops`.**
`JsonlStorage::reload` *recovers* glued `…}}}{…` lines on read, so asserting through `all_ops` would MASK a real append-lock failure.
`chaos_helpers::assert_every_line_is_one_json_value` reads the `ops-<actor>.jsonl` bytes directly and asserts each physical line decodes to exactly one JSON value — only the raw bytes reveal whether the lock actually held.

**Helpers.**
Chaos-only helpers (the seeded `Rng`, the raw-bytes corruption assertion, the HLC-offset seeder) live in `tests/chaos_helpers/mod.rs` — a sibling module, NOT `tests/common/mod.rs` (which the integration/catchup suites own and stays read-only).
`common/mod.rs` is loaded by exactly ONE module per test binary (clippy `duplicate_mod`); `all_node_ids_on_disk` wraps `common`, so it lives in `chaos.rs` while the common-free helpers live in `chaos_helpers`.

## Sync invariants

- The op log (`ops-<actor>.jsonl`) IS the offline buffer.
- On reconnect: bidirectional vector-clock delta sync.
  Both sides exchange `Storage::last_ts_per_actor()` and stream missing ops.
- Ops from actor C received via peer B are stored as `ops-<C>.jsonl` locally.
  A can get C's ops via A↔B sync even if A never connects to C directly.
- HLC sanity gate: ops with timestamps more than 24h in the future are
  logged as warnings and skipped (not applied).

## Catch-up loop (initial full sync on pairing)

`run_iroh` spawns a periodic **catch-up loop** (`catch_up_loop` → `run_catch_up`)
in addition to the boot-time connect, the gossip subscribe, and the announce
drain.
It exists for one bug: a device paired AFTER `start()` writes its `PeerEntry` to `peers.json`, but the boot connect read the peer list once and never re-reads it.
So the new peer's op-log history is never pulled (only brand-new ops would trickle in via gossip).

- **Tick**: `CATCH_UP_INTERVAL` (8s). tokio's `interval` fires the first tick
  immediately, so a freshly paired peer syncs within one tick.
- **Each tick**: reload `PeersStore` from the SAME `peers.json` path the
  transport started with (threaded via `PeersStore::path()`), so peers paired
  after boot are picked up.
- **Dial**: build a full `iroh::EndpointAddr` from each `PeerEntry`
  (`PeerEntry::iroh_endpoint_addr`) — the stored full `endpoint_addr` (id +
  relay + **direct addrs**) first, then id + `relay_url`, then the bare id.
  The direct addrs make device↔device connect immediately on the same LAN
  instead of depending on n0 discovery resolving a route (the old bare-id
  connect is why the status dot showed offline).
  The boot-time connect loop and `probe_peers` (status dot) use the same builder.
- **Maintenance re-sync (the convergence safety net)**: each peer's last clean sync is timestamped in a `HashMap<EndpointId, Instant>`.
  A peer is (re)dialed when it's new this session, its last attempt failed (absent from the map), OR its last success is older than `MAINTENANCE_RESYNC` (10s).
  `delta_sync` is a cheap no-op on matching vector clocks, and the in-flight guard collapses a slow re-dial into the previous one, so the short interval doesn't thunder.
  **This is load-bearing**: it makes convergence independent of the real-time gossip path.
  The announce may never cross (flaky cross-network iroh), or a writer may never call `announce_local_ops` at all (the ephemeral CLI, see "Passive writers" below).
  Either way the loop re-pulls from every known peer within `MAINTENANCE_RESYNC` and converges.
  The earlier design marked a peer "synced once, never re-dial" and trusted gossip for live updates.
  When gossip was down (or the announce was missing), a later edit on the other device never pulled — the "paired, first sync worked, then nothing propagates" bug.
  Regression: `catch_up_resyncs_peer_after_interval` (new ops land with NO gossip and NO id-change signal, purely via the periodic re-pull).
  **The map is cleared when the workspace id changes** (pairing adoption fires the `wid_changed` broadcast — `run_catch_up` `select!`s on it), forcing an immediate re-dial of every peer under the adopted id.
- `PeerEntry` carries the peer's **full** `EndpointAddr` in `endpoint_addr`
  (base64 JSON), captured at pairing time after the endpoint came online —
  see "Reachability: full `EndpointAddr`" below.

`run_catch_up` is parameterized over the tick `period`, the `resync_after`
window, and a `resolve_peers` closure so `test_support::run_catch_up_loop` can
drive it over loopback (direct addrs, short tick, short re-sync) — see the
`catch_up_syncs_peer_paired_after_boot` and `catch_up_resyncs_peer_after_interval`
regression tests.

## Passive writers vs the MCP peer

Since two sync-serving endpoints can share the device identity without breaking (see "One endpoint per identity"), the rule splits by **process lifetime**, not by "is it a GUI":

- **The MCP server brings a real transport up.**
  `outl mcp serve` is long-lived (the whole Claude Desktop session), so it CAN hold an endpoint and push in real time.
  On the first workspace open it spins up `IrohSyncTransport` with the shared `~/.outl/identity.key` + `~/.outl/peers.json` **when the device has paired peers**.
  It announces after every mutating tool, drains peer pushes (reopening the workspace so it serves fresh ops), and shuts down on stdin close.
  If a GUI is also running, the two share the identity — stable hijack, both serve.
  Wired in `outl-cli` `mcp/mod.rs` (`ServerCtx::ensure_transport` / `announce_after_mutation` / `shutdown_transport`).
- **The ephemeral CLI stays a passive writer.**
  A `page`/`block`/`daily`/`batch`/`import` command runs in ~200ms — too short to establish a QUIC connection (seconds), so it writes `ops-<actor>.jsonl` and exits without touching iroh.
  Its ops converge via a co-resident long-lived peer (GUI / MCP) plus every device's `MAINTENANCE_RESYNC` re-pull.
  `outl sync` is the explicit flush: it brings a transport up, forces a push/pull pass, waits, and exits.
- **`outl peer pair` / `status`** use a transient endpoint they close before returning.
  `status`'s `probe_peers` is the one non-sync endpoint, so it is CLI-only and must stay off while any sync transport is live (see "One endpoint per identity").

Correctness never depends on the announce: the `MAINTENANCE_RESYNC` catch-up is the safety net that converges any writer's ops, announced or not.
The transport on MCP/GUI is a **latency** optimization (real-time vs next catch-up tick), now safe to run everywhere because the hijack is benign.

## Reachability: full `EndpointAddr` in `PeerEntry` (load-bearing)

`PeerEntry` persists the peer's **full** `iroh::EndpointAddr` — node id + relay
URL + **direct socket addrs** — base64-JSON-encoded in the `endpoint_addr`
field, not just the bare node id.

**Why:** a bare node id forces every connect (boot, catch-up, status probe) to depend on n0 discovery resolving a route, which does not resolve reliably between real devices.
The status dot showed offline and the first sync never pulled history.
Two devices on the same WiFi can connect instantly via direct addresses — but only if those addresses were captured and stored.
They weren't: the old `PeerEntry` stored `relay_url: null` and no addrs.

**Capture (`pairing::ready_addr`):** `endpoint.addr()` right after `bind()` is typically empty (no relay handshake, no net report yet).
Before generating the ticket (host) or sending the payload (joiner), both sides call `ready_addr`, which awaits `endpoint.online()` under a 5s timeout.
`online` pends forever with no relay/WAN, so the timeout is mandatory.
After that the addr carries the relay + the discovered LAN direct addrs.
On timeout we proceed anyway — the local net report has usually already filled in the direct addrs, which is all two devices on the same WiFi need.

**Exchange:** the host mints the ticket from its ready addr, so the joiner stores a reachable host; the joiner sends its own ready `EndpointAddr` in the pairing payload, so the host stores a reachable joiner.
Each side persists the other's full addr.

**Resolution order** (`PeerEntry::iroh_endpoint_addr`): stored `endpoint_addr` (full) → id + `relay_url` → bare id.
A corrupt `endpoint_addr` logs a warning and falls through, never failing the dial.

**Back-compat:** `endpoint_addr` is `#[serde(default)]`, so old `peers.json` entries (just `node_id` + `relay_url`) still deserialize and dial via the fallback.
`relay_url` is kept for display + back-compat.

The ticket codec and the `endpoint_addr` codec are the **same** function (`peers::encode_endpoint_addr` / `decode_endpoint_addr`).
`encode_ticket` / `decode_ticket` delegate to it — a pairing ticket IS a `PeerEntry.endpoint_addr`.

## STOPGAP: IPv4-only bind (iroh 1.0.0 multipath workaround)

**All four endpoints bind IPv4-only.**
This is a temporary workaround for an iroh 1.0.0 bug, owned by the `bind` module (`bind::n0_builder_ipv4_only`).

**The bug:** iroh 1.0.0's QUIC-multipath stack opens paths to **all** of a peer's candidate addresses concurrently.
Say a peer's `EndpointAddr` carries a global IPv6 direct addr that is "No route to host" on a LAN-only device.
Multipath then stalls on that dead path (`PTO expired`, `failed closing path err=MultipathNotNegotiated`).
The whole connect/accept times out (~30s) instead of converging on the working LAN-IPv4 or relay path.
iroh 1.0.0 exposes **no** public knob to disable multipath.
(`max_concurrent_multipath_paths` clamps to >= 9; `path_selector` is unstable + only picks among already-opened paths; no env var.)
Downgrade is blocked too — `iroh-gossip 0.101.0` requires `iroh = "1"`.

**The fix (Option A):** bind an **IPv4-only** UDP socket via `Endpoint::builder(presets::N0).clear_ip_transports().bind_addr("0.0.0.0:0")`.
The endpoint then never discovers/advertises a global IPv6 direct addr, so neither the dial side nor the accept side ever opens a dead IPv6 path.
`clear_ip_transports` only drops the IP transports; the `presets::N0` relay transport stays, so **LAN-IPv4 direct + n0 relay fallback are both preserved**.
See iroh-1.0.0 `src/endpoint.rs`: `Builder::bind_addr` / `Builder::clear_ip_transports`.
By default the builder pre-binds both `0.0.0.0` and `[::]`; `clear` + `bind_addr` re-adds IPv4 only.

Every endpoint goes through `bind::n0_builder_ipv4_only` so the dial and accept sides stay consistent — `run_iroh` (engine), `bind_pairing_endpoint` (pairing), `probe_peers` (status), and `bind_sync_endpoint` (test_support).
Dropping IPv6 on only one side would let the other still advertise a dead path.

**Revert condition:** delete the `bind` module once iroh > 1.0.0 ships the multipath fallback fix (a stalled path no longer blocks convergence on a healthy path).
Then let every call site go back to the plain dual-stack `Endpoint::builder(presets::N0)` builder.
Track iroh > 1.0.0.

## iroh 1.0.0 API notes (load-bearing)

The 1.0.0 surface differs from older tutorials — pin these:

- `iroh::SecretKey`, not `iroh::key::SecretKey`. `SecretKey::from_bytes(&[u8;32])`,
  `to_bytes() -> [u8;32]`, `public() -> PublicKey`, `generate(&mut rand::rng())`.
- `EndpointId = PublicKey` is the node identifier type. `iroh::PublicKey` is the
  concrete struct; parse from string with `.parse()`.
- `Endpoint::builder(presets::N0)` — the builder takes a discovery preset arg.
  `presets` lives at `iroh::endpoint::presets`.
- `ProtocolHandler::accept(&self, conn: Connection) -> Result<(), AcceptError>`
  is a **native async fn** and receives an already-accepted `Connection`
  (not `Connecting`, no manually-boxed future).
- `endpoint.connect(id, b"alpn")` takes the ALPN bytes directly.
- `SendStream::finish()` is sync and returns `Result`; `write_all`/`read_to_end`
  are async.
- Gossip: `gossip.subscribe(topic, peers).await?` returns a topic handle;
  `.split()` yields `(GossipSender, GossipReceiver)`.
  Events are `iroh_gossip::api::Event::Received(message)` with
  `message.content` and `message.delivered_from`.
  `StreamExt` comes from `n0_future`.
- `GossipSender::broadcast(&self, msg: bytes::Bytes) -> Result<(), ApiError>`
  takes `bytes::Bytes` and `&self` (not `&mut`), so the sender can live in a
  drain task that `announce_local_ops` feeds via an `mpsc` channel.
- **No `NodeTicket` type ships in iroh 1.0.0 / iroh-base 1.0.0.**
  The pairing "ticket" is a base64 of `serde_json(EndpointAddr)`.
  `EndpointAddr` is `Serialize`/`Deserialize` with public `id: EndpointId`
  and `addrs: BTreeSet<TransportAddr>`; `endpoint.addr()` returns the current
  one, and `endpoint.connect(addr, alpn)` takes `impl Into<EndpointAddr>`, so
  the decoded value feeds straight back into `connect`.
- Accept loop (host side): `endpoint.accept().await -> Option<Incoming>`,
  then `incoming.accept()? -> Accepting`, then `.await -> Connection`.
  `Connection::accept_bi()` / `open_bi()` / `close(VarInt, &[u8])` as usual.

## What this crate does NOT own

- CRDT logic — lives in `outl-core`
- Workspace reload / md projection — lives in `outl-actions::SyncEngine`
- iCloud / filesystem transport — lives in `outl-actions::FileSyncTransport`
