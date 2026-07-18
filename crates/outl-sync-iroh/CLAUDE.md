# CLAUDE.md — outl-sync-iroh

iroh-based P2P transport for outl.
Implements `outl_actions::SyncTransport` using iroh QUIC + iroh-gossip.

## What this crate owns

- `IrohIdentity` — ed25519 keypair, stored at `~/.outl/identity.key` (per **device**, one node id per machine)
- `PeersStore` — known paired peers, stored at `<workspace>/.outl/peers.json` (per **graph**, the pair belongs to the workspace, not the OS).
  `workspace_peers_path(root)` builds the path; `migrate_global_peers_if_absent(root)` does a one-time best-effort copy of any legacy global `~/.outl/peers.json` into the workspace on first open (never deletes the global).
  Every client calls `migrate_*` then `PeersStore::load_or_default(workspace_peers_path(root))`.
- `IrohSyncTransport` — implements `SyncTransport` trait, including the
  gossip-backed `announce_local_ops` hook (sync side → tokio task via an
  `mpsc` channel set up in `start()`) and the `peer_health()` reachability
  snapshot (see "One endpoint per identity" below)
- Wire protocol — ALPN `b"outl-sync/2"`, vector-clock delta sync with per-actor `ActorClock { max, count }` gap detection (see "Sync invariants"; the v2 bump makes an old↔new dial fail cleanly, no compat shim)
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
  The id is a ULID persisted at `<root>/.outl/workspace-id` (plaintext, one line), read-or-generated on first transport `start()` via `WorkspaceId::read_or_create` (an existing workspace gets one on first open, stable thereafter).
  It lives in `.outl/` (never pollutes the clean markdown) as its **own** file, not a `config.toml` field.
  `config.toml` holds the **per-device** `actor_id` while the workspace id must be **identical** across devices; keeping them separate lets pairing-adoption rewrite the id without touching the actor.
- **Gossip topic.**
  `workspace_topic_id` = `blake3(workspace_id)`, so two devices on the same workspace land on the same topic regardless of path.
- **Sync request.**
  `SyncRequest.workspace_id` carries the id; `SyncProtocolHandler::serve` **validates it against the local id** (`workspace-mismatch` close) **and checks `remote_id()` is in `peers.json`** (read fresh per connection; `unknown-peer` close).
  Issue #158: the id only proves the peer thinks it belongs; a removed device still knows it, so revocation needs the peer check.
- **Pairing makes the joiner adopt the host's id.**
  The handshake `PairingPayload` carries each side's id; the **host keeps** its id and the **joiner adopts** it *before* the immediate post-pair `delta_sync` fires.
  Adoption is **persist-first**: `adopt_workspace_id` writes the host's id to `.outl/workspace-id` and only then flips the in-memory handle; a failed disk write does NOT adopt (retry-safe).
  A half-adopted state (memory new, disk old) would silently split the workspace on the next start, which reads the stale id from disk.
  Both sides then compute the same topic, validate as one workspace, and their op logs CRDT-merge (content from both converges — expected).
  CLI `outl peer pair` neither advertises nor adopts (it edits `peers.json`, not the live workspace); adoption is a GUI concern, wired in `engine_pairing`.
- **Live handle.**
  The id lives behind a shared `RwLock` (`SharedWorkspaceId`) read at call time by `delta_sync` + serve, so an adopted id takes effect immediately for direct sync (boot connect, 8s catch-up, immediate post-pair dial — all carry the live id).
- **Gossip re-subscribes on id change (no restart).**
  Adoption fires a `tokio::sync::broadcast::Sender<WorkspaceId>` held in the `PairingHub` (`adopt_workspace_id` sends after persisting the file + updating the `RwLock`).
  The gossip supervisor (`engine_gossip::run_gossip`) holds one receiver and, on the signal, **drops its old-topic subscription and re-subscribes to `blake3(new id)`** — same `Gossip`/`Endpoint`, no second endpoint, no `start()` restart.
  Before this, the boot-time subscription stayed on the *pre-adoption* topic, so the two devices sat on different gossip topics and no real-time announce ever crossed (the "post-pair, nothing syncs again" bug).
- **Catch-up re-dials on id change.**
  A second receiver on the **same** broadcast channel (`PairingHub::subscribe_wid_changed`) makes the catch-up loop **clear its per-session `synced` dedup** and re-dial every peer under the adopted id.
  Without it, the post-pair `delta_sync` marked the peer synced forever and later host edits never pulled (gossip was the only live path, dead per the point above).
  A dropped signal is safe — the `RwLock` is the source of truth; the signal only fixes the *real-time* + *re-dial* gaps.

## One endpoint per identity (load-bearing invariant)

**Only endpoints that SERVE the sync ALPN may share a device identity.**
A non-sync endpoint (status probe, a pairing-only endpoint) must NOT bind the device `SecretKey` while the transport is running.

The nuance (verified against the iroh 1.0.0 source — `endpoint.rs::same_endpoint_id_relay` + `iroh-relay` `clients.rs`):

- **Two endpoints with the same `node_id` that BOTH serve `SYNC_ALPN` coexist fine.**
  The relay keeps one `node_id → endpoint` route; the last to register holds it, the other loses *inbound* but keeps *outbound* dials (catch-up).
  The loser never steals back (the holder's socket keeps answering pings) — a **stable hijack, not a flap** — and the route returns when the holder leaves.
  This lets the **GUI and the MCP server both bind the device identity at once** (see "Passive writers"): whichever holds the route serves inbound from the same `ops-*.jsonl`, the other pushes/pulls on its own dials.
- **An endpoint that does NOT serve `SYNC_ALPN` is the dangerous case.**
  If the newcomer can't accept `SYNC_ALPN`, a dialer routed to it gets `CONNECTION_REFUSED` — sync breaks for real (the original device↔device bug, detailed below).

**Why the route is single:** a second endpoint registering the same secret key *replaces* the active client in the relay's `DashMap<EndpointId, ClientState>`.
All inbound datagrams then route to the newcomer and the original silently stops receiving (`endpoint.rs::same_endpoint_id_relay` asserts this).
If the newcomer doesn't accept `SYNC_ALPN`, the dialer gets `quinn` `CONNECTION_REFUSED` — the "connection refused, nothing syncs" bug (a transient status-probe or the GUI's old pairing endpoint stealing the route).

**Three call sites, three rules:**

1. **Sync endpoint (`engine::run_iroh`)** — the one allowed long-lived endpoint.
   Router accepts `SYNC_ALPN` + gossip ALPN **+ `PAIRING_ALPN`** (rule 3) **+ `SNAPSHOT_ALPN`** (see "Snapshot sync"), all advertised in its `.alpns()`.
   All catch-up / boot / gossip / pairing dials go out through *this* endpoint (the one bound in `run_iroh`); no helper spins up a second.
2. **Status (`status::probe_peers`)** — binds a transient endpoint with the device identity.
   **CLI-only — forbidden from the GUI.**
   The desktop / mobile `outl_peer_status` commands read reachability from the running transport's `peer_health()` instead (see below).
   `probe_peers` survives only for `outl peer status` in the CLI, which has no running transport.
3. **Pairing** — two paths, never a second endpoint while the transport runs:
   - **GUI** (mobile / desktop, transport running) → `IrohSyncTransport::pair_host` / `pair_join` reuse the **live sync endpoint**.
     The host (accept) side is the `PAIRING_ALPN` router handler (`engine_pairing::PairingProtocolHandler`), armed by `pair_host` via a shared `PairingHub`; the join side dials out on the same endpoint.
     After a successful pair the new peer is persisted to `peers.json` and an **immediate** `delta_sync` is fired against it (`engine_pairing::drain_pair_completions`) — no app restart, no 8s catch-up wait.
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
Desktop also keeps a **concrete** clone in `state.iroh_pairing` for `pair_host` / `pair_join` (not `SyncTransport` methods — the trait can't return `PeerEntry` without a dep cycle).
Both desktop slots are wired/cleared together in `wire_iroh_transport`; mobile reuses `state.iroh` for both status and pairing.
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
- **Cross-process flock (`ops/.append.lock`).**
  The tokio mutex is per-process, but a device runs several transports at once (GUI + MCP server + `outl sync`), and their interleaved batches produced the timestamp retrocessions behind the watermark-gap bug.
  `write_deduped_batch` therefore also takes a blocking advisory flock on `ops/.append.lock` (same mechanism as `outl-core`'s `ops/.lock-<actor>`), acquired AFTER the in-process lock and held across the whole batch, on a `spawn_blocking` thread.
  The lockfile is ephemeral and recreated on demand, so a file transport dropping the dotfile is harmless (flock state is kernel-local and never syncs).
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

- `IrohSyncTransport` holds `sync_now_tx: Arc<Mutex<Option<UnboundedSender<()>>>>`, populated in `start()` (`None` before start / after shutdown = the "runtime down" guard).
- `sync_now()` sends a unit; a send error means the runtime is down, ignored (no-op), same contract as `announce_local_ops`.
- `run_iroh` moves the receiver into a `drain_sync_now` task (`engine_catchup`); each signal runs `force_sync_all`, dialing **every** peer in `peers.json` (reusing the append lock + in-flight guard + health recording).

**`force_sync_all` deliberately does NOT respect the catch-up loop's per-session `synced` `HashSet`** — the whole point of a manual sync is to re-dial even healthy peers the catch-up loop leaves to gossip.
`delta_sync` is a cheap no-op on matching vector clocks, so a forced re-dial of an already-synced peer just confirms convergence.
It still honors `try_acquire_in_flight` (skip a peer already being dialed) and the `AppendLock`.

The GUI commands are `outl_sync_now` in each client's `commands/peers.rs` (mobile reads `state.iroh`, desktop reads `state.iroh_transport` `Arc<dyn SyncTransport>`).
The shared wrapper is `syncNow()` in `@outl/shared/api/commands`.

**Observing completion:** `completed_sync_passes()` is a monotonic counter bumped after each finished pass (every peer dialed, succeeded *or* failed — no reachability promise).
A waiter (the iOS `bg_sync.rs` FFI) snapshots it, fires `sync_now()`, and polls until it advances or a cap elapses; any pass after the snapshot counts.

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
A periodic task (`MEMBERSHIP_INTERVAL`, 5s) reloads `peers.json` and broadcasts the peer list (never an empty one) over the **same** gossip topic.
It reuses the **same** `Arc`-wrapped `GossipSender` as the announce drain — no second topic handle or endpoint.
It lives in the same `if !bootstrap_ids.is_empty()` block as the announce drain, so a zero-peer device neither subscribes nor gossips.

**Merge / persist flow.**
On receipt, `merge_membership` merges **unknown** peers into `peers.json` through `PeersStore::merge_unknown` (dedup by node_id, ADD-only — an existing entry's locally-captured addr, e.g. from direct pairing, is never clobbered).
The catch-up loop reloads `peers.json` each tick and dials the freshly-merged peer — **no new dialing machinery**; the append lock / in-flight guard / health map are reused as-is.

**Trust model (load-bearing assumption).**
Every device subscribed to the workspace gossip topic is **already inside the trust domain**: the topic id is `blake3(workspace_id)` (`workspace_topic_id`), so only devices paired into this mesh by *someone* ever subscribe.
Membership gossip therefore only ever ADDS reachability for peers **already in the mesh** — it never invites a stranger (a non-member can't reach the topic to inject a peer, and a merged peer was already trusted by the member that gossiped it).
Conservative guards on the merge:

- **Never add self** (drop our own node_id from any incoming list).
- **Never add an unreachable peer** (skip an entry whose `PeerEntry::iroh_endpoint_addr` won't resolve — we don't store a peer we can't dial).
- **ADD-only dedup** (a known node_id keeps its current entry).

If membership ever needs to *gate* who can join (beyond "is on the topic"), that's a new trust surface — stop and design it; do not loosen these guards silently.

## Snapshot sync (Phase 2, ALPN `outl-snapshot/1`)

A freshly-paired device that would otherwise receive + replay a huge op log (76 MB / 200k+ ops) instead pulls a peer's materialized **snapshot** (`snap-<actor>.bin`) and boots from it via `outl_core::snapshot::read_best_from_disk`.
This crate owns **only the transport** (in `engine_snapshot.rs`); the boot-adoption half is done in `outl-core` and never touched here.

- **Responder** (`SnapshotProtocolHandler`, under `SNAPSHOT_ALPN`) reads this device's own `snap-<self.actor>.bin` off disk and ships it as one length-prefixed frame (`protocol::encode_blob_frame`; empty when absent → peer skips).
  No workspace lock — it reads a cache file and holds no `Workspace`.
- **Puller** (`pull_snapshot_from_peer`, fired from `drain_pair_completions` right after the immediate `delta_sync`, still inside the peer's in-flight guard) dials `SNAPSHOT_ALPN` and reads the frame.
  It `SnapshotBody::decode`s the blob (validate + learn actor id), writes it to `snap-<peer-actor>.bin` via `snapshot::write_to_disk` (atomic tmp+rename), and fires `peer_ready_tx` so boot adopts it.
- Not an op-log write → **no `AppendLock` / flock**; it's a local boot cache in the dotfile dir `.outl/snapshots/`, off the file-sync surface (the transfer is the only way it crosses devices).
- Best-effort: an absent / corrupt / undecodable peer snapshot is skipped, never fatal — the op log stays source of truth and boot falls back to full replay.

## Regression suite (Pilar 2)

Every bug hand-found during the sync saga has a NAMED, permanent test — the name IS the bug, so a failure is self-explanatory.
Pure guards live in `#[cfg(test)]` next to the code; over-the-wire (real QUIC, loopback) guards in `tests/regression.rs`.
Shared seed/read/wait helpers stay in `tests/common/mod.rs` (read-only); saga-specific helpers live inside `regression.rs`.

| Saga bug | Guard test | Where |
|----------|-----------|-------|
| 1. Op-log corruption from concurrent appends (glued `…}}}{`) — append lock serializes inbound batches | `concurrent_appends_never_glue_ops_on_the_responder` (asserts no `}{` on disk + every op parses) | `tests/regression.rs` |
| 1. (parser-recovery half) a hand-crafted glued `}}}{` line still loads both ops | `recovers_glued_ops_on_one_line` (pre-existing, core-side) | `outl-core` `storage/jsonl.rs` |
| 2. HLC far-future op skipped on ingest (±24h gate) | `far_future_hlc_op_is_skipped_on_ingest` (B sends a ~48h-ahead op + a valid op; only the valid one lands on A) | `tests/regression.rs` |
| 3. Workspace identity = stable id, not path (topic) | `same_workspace_id_yields_same_topic_across_paths` (pre-existing) | `tests/integration.rs` |
| 3. Workspace identity = stable id, not path (END-TO-END sync) | `different_paths_same_workspace_id_sync_as_one` (two devices at different paths, same id, converge) | `tests/regression.rs` |
| 3. Mismatched ids are rejected | `delta_sync_rejects_mismatched_workspace_id` (pre-existing) | `tests/integration.rs` |
| 3b. Removed/unknown peer denied (issue #158) | `removed_peer_is_denied_sync` | `tests/regression.rs` |
| 4. Pairing adoption (joiner adopts host id, syncs as one) | `gui_pairing_over_live_sync_endpoint` (pre-existing; asserts adopted id persisted in both peers.json) | `tests/integration.rs` |
| 5. Single endpoint per identity (pair AND sync over the live sync endpoint, no relay hijack) | `gui_pairing_over_live_sync_endpoint` (pre-existing; pairing rides the live sync endpoint, no second bind) | `tests/integration.rs` |
| 6. Reachability resolution + off-LAN/IPv6 direct-addr filter (issue #133) | `iroh_endpoint_addr_*` + `is_reachable_lan_ipv4_*` (keep on-LAN IPv4, drop IPv6 + stale VPN IPs, fall back stored/bare/corrupt) | `src/peers.rs` |
| 7. Bidirectional push materializes on BOTH sides AND fires BOTH reload signals | `bidirectional_sync_fires_reload_signal_on_both_sides` (set convergence + `peer_ready_tx` on initiator AND responder) | `tests/regression.rs` |
| 7. (set-convergence half) both sides hold all ops | `bidirectional_delta_sync` (pre-existing) | `tests/integration.rs` |
| 8. Membership merge is ADD-only (never clobber a local entry, drop self, drop undialable) | `merge_unknown_never_clobbers_a_known_entry` + `merge_skips_self` / `merge_adds_unknown_and_dedups_known` / `merge_skips_unreachable_peer` | `src/peers.rs`, `src/engine_membership.rs` `#[cfg(test)]` |
| 9. Watermark gap — ops below a receiver's max-HLC stayed permanently invisible after out-of-order ingest; the v2 `ActorClock` count detects the gap, the full-log fallback + ingest dedup converge without duplicating | `backlog_below_watermark_crosses_after_gap_detected` / `ingest_dedups_already_present_ops` / `full_actor_resend_converges_and_dedups` | `tests/regression.rs` |
| 10. Snapshot sync — peer snapshot transferred on pair (byte-identical, reload fired); absent snapshot harmless, op-sync still works | `snapshot_transfers_from_peer_on_pair` / `snapshot_pull_absent_is_harmless` | `tests/regression.rs` |

Names map 1:1 to the saga checklist; do NOT delete one without deleting the bug it guards.

### Chaos/concurrency tests (Pilar 3)

The regression suite pins one named bug per row; the **chaos battery** (`tests/chaos.rs`) instead *hammers* the same wire code (real `delta_sync` + `SyncProtocolHandler` via `test_support`) with STRESS loads (N writers × M ops).
Every test runs over real QUIC on loopback under `#[tokio::test(flavor = "multi_thread")]`.

| Failure mode | Chaos test | Asserts |
|--------------|-----------|---------|
| Concurrent writers gluing the op log | `concurrent_writers_never_corrupt_op_log` | 8 initiators push one actor's ops at one responder under the shared `AppendLock`; every line is one JSON value + exact union |
| Reordered + duplicated delivery | `reordered_and_duplicated_delivery_converges` | 3 nodes, seeded-shuffled passes, ~half twice; all converge |
| Partition + heal under load | `partition_then_heal_under_load` | B offline while A/C edit; B rejoins and converges, no glued line |
| Fan-out + redundant dials | `fan_out_to_many_peers_converges_without_double_dial` | 5 peers dial one hub, each dial twice; exact union, every op once |
| Single-endpoint invariant under concurrency | `concurrent_inbound_dials_on_single_endpoint_stay_clean` | 6 inbound dials on one hub endpoint while it dials out; converges both ways, no corruption |

**Determinism** (a flaky chaos test is worse than none): randomness is a seeded xorshift (`chaos_helpers::Rng`); the only true nondeterminism is network timing, so every wait uses `common::STEP_TIMEOUT` + `wait_until`, never a fixed sleep.
Sizes are bounded (≤ 64 ops, ≤ 8 tasks).

**Why raw bytes, not `all_ops`:** `JsonlStorage::reload` recovers glued `…}}}{…` lines on read, so `all_ops` would MASK an append-lock failure.
`chaos_helpers::assert_every_line_is_one_json_value` reads the `ops-<actor>.jsonl` bytes directly — the only thing that reveals whether the lock held.

**Helpers** live in `tests/chaos_helpers/mod.rs`, not `tests/common/mod.rs` (clippy `duplicate_mod` allows one `common` loader per test binary).

## Sync invariants

- The op log (`ops-<actor>.jsonl`) IS the offline buffer.
- On reconnect: bidirectional vector-clock delta sync.
  Both sides exchange a per-actor `ActorClock { max: Hlc, count: u64 }` — max + DISTINCT-op count, derived from `all_ops` in `engine_sync::local_vector_clock` (the `Storage` trait is untouched) — and stream missing ops.
- **Gap detection (v2).**
  A bare max-HLC watermark assumes gapless delivery; an op landing ahead of a pending backlog made everything below the watermark permanently invisible (the Mac↔iPhone non-convergence).
  If the sender holds more distinct ops `<= max_r` than the receiver's `count`, it resends that actor's FULL log; the receiver's ingest dedup (present-set read under the append locks) absorbs the overlap and never appends an op twice.
  Accepted limit: equal counts over different subsets are indistinguishable — convergence still lands via each op's origin device, which always holds its own actor's complete log.
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
  A peer is (re)dialed when new this session, when its last attempt failed (absent from the map), or when its last success is older than `MAINTENANCE_RESYNC` (10s).
  `delta_sync` is a cheap no-op on matching vector clocks and the in-flight guard collapses a slow re-dial into the previous one, so the short interval doesn't thunder.
  **Load-bearing**: convergence must not depend on the real-time gossip path, since the announce may never cross (flaky cross-network iroh) or never be sent at all (the ephemeral CLI, see "Passive writers").
  The loop re-pulls every known peer within `MAINTENANCE_RESYNC` regardless.
  The earlier "synced once, never re-dial" design broke exactly there ("paired, first sync worked, then nothing propagates"); regression: `catch_up_resyncs_peer_after_interval`.
  **The map is cleared when the workspace id changes** (`run_catch_up` `select!`s on the `wid_changed` broadcast), forcing an immediate re-dial of every peer under the adopted id.
- `PeerEntry` carries the peer's **full** `EndpointAddr` in `endpoint_addr`
  (base64 JSON), captured at pairing time after the endpoint came online —
  see "Reachability: full `EndpointAddr`" below.

`run_catch_up` is parameterized over `period`, `resync_after`, and a `resolve_peers` closure so `test_support::run_catch_up_loop` drives it over loopback (regressions `catch_up_syncs_peer_paired_after_boot`, `catch_up_resyncs_peer_after_interval`).

## Passive writers vs the MCP peer

Since two sync-serving endpoints can share the device identity without breaking (see "One endpoint per identity"), the rule splits by **process lifetime**, not by "is it a GUI":

- **The MCP server brings a real transport up.**
  `outl mcp serve` is long-lived (the whole Claude Desktop session), so it CAN hold an endpoint and push in real time.
  On first workspace open it spins up `IrohSyncTransport` (shared `~/.outl/identity.key` + workspace `.outl/peers.json`) **when the workspace has paired peers**.
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

**Why:** a bare node id makes every connect depend on n0 discovery resolving a route, unreliable between real devices (offline dot, first sync never pulled history).
Same-WiFi devices connect instantly via direct addrs — but only if captured; the old `PeerEntry` stored `relay_url: null` and no addrs.

**Capture (`pairing::ready_addr`):** `endpoint.addr()` right after `bind()` is typically empty, so both sides call `ready_addr` before minting the ticket / sending the payload.
It awaits `endpoint.online()` under a mandatory 5s timeout, after which the addr carries relay + LAN direct addrs; on timeout we proceed anyway (the local net report usually filled them).

**Exchange:** host mints the ticket from its ready addr (joiner stores a reachable host); joiner sends its ready `EndpointAddr` in the payload (host stores a reachable joiner).

**Resolution order** (`PeerEntry::iroh_endpoint_addr`): stored `endpoint_addr` → keep the relay + the **on-LAN IPv4** direct addrs, **drop IPv6** and **drop off-LAN IPv4**; else id + `relay_url`; else bare id.
A corrupt `endpoint_addr` logs a warning and falls through, never failing the dial.
Not relay-only (flaky relay breaks same-WiFi sync), not all-addrs (a dead IPv6/off-LAN direct stalls multipath) — on-LAN IPv4 + relay is the balance.

**Off-LAN IPv4 drop (issue #133):** a VPN-paired peer stores tunnel IPs (`10.x`, `100.x`, WAN) beside its LAN addr, stalling iroh's multipath on the dead ones.
`iroh_endpoint_addr` keeps only IPv4 on a **local** subnet (`is_reachable_lan_ipv4` + `if-addrs`; injectable `iroh_endpoint_addr_with_ifaces`), fail-open on error.

**Back-compat:** `endpoint_addr` is `#[serde(default)]`, so old `peers.json` entries (id + `relay_url` only) still deserialize + dial via the fallback.

Ticket codec == `endpoint_addr` codec (`encode_ticket` / `decode_ticket` delegate to `peers::{encode,decode}_endpoint_addr`) — a pairing ticket IS a `PeerEntry.endpoint_addr`.

**Self-heal on inbound connect (`peers::refresh_peer_direct_addr`):** a stored addr goes stale when a peer's DHCP lease moves (stalling multipath like an off-LAN one).
On an **accepted** inbound connection `serve()` reads the live remote socket off `Connection::paths()` and rewrites the stored `endpoint_addr` to *only* that socket + the known relay, so the next catch-up dial uses the fresh route without a re-pair.
Conservative: only an **already-paired** peer (unknown id → no-op), no write when unchanged.
Regression: `refresh_peer_direct_addr_replaces_stale_keeps_relay_and_is_idempotent`.

## STOPGAP: IPv4-only bind (iroh 1.0.0 multipath workaround)

**All four endpoints bind IPv4-only** — a temporary workaround for an iroh 1.0.0 bug, owned by the `bind` module (`bind::n0_builder_ipv4_only`).

**The bug + fix (full rationale in the `bind` module docs).**
iroh 1.0.0 multipath opens paths to **all** of a peer's candidate addrs at once, so a dead global IPv6 direct addr stalls the whole connect/accept (`MultipathNotNegotiated`, ~30s) instead of converging on the working LAN-IPv4/relay path.
The fix binds an **IPv4-only** socket (`clear_ip_transports().bind_addr("0.0.0.0:0")`) so the endpoint never advertises a global IPv6 direct addr; the relay transport is kept, so **LAN-IPv4 direct + relay fallback both survive**.

Every endpoint goes through `bind::n0_builder_ipv4_only` so dial and accept stay consistent — `run_iroh`, `bind_pairing_endpoint`, `probe_peers`, `bind_sync_endpoint`.
Dropping IPv6 on only one side would let the other advertise a dead path.

### Configurable relay (default: outl's own)

`n0_builder_ipv4_only(relay_url: Option<&str>)` picks the relay on top of the IPv4-only STOPGAP.
Default is outl's own dedicated relay, `DEFAULT_RELAY_URL` (`use1-1.relay.avelino.outl.iroh.link`, via `RelayMode::custom`) — the n0 public relay proved slow/unreachable on some networks.
A non-empty `[sync] relay_url` overrides it; a parse error falls back to `presets::N0` with a warning.
Only the long-lived **sync** endpoint threads it (`run_iroh` ← `IrohSyncTransport::new` ← `outl_config::load().sync.relay_url()`); pairing/status/test pass `None`.
See `docs/relay.md` / `docs/config.md`.

**Revert condition:** delete the `bind` module once iroh > 1.0.0 ships the multipath fallback fix, and let every call site go back to the plain dual-stack `Endpoint::builder(presets::N0)` builder (details in the module docs).

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
