# RFC 0155 — A paired peer is not a trusted peer

| | |
|---|---|
| **Status** | Accepted (two of four holes closed — see Scope) |
| **Issue** | [#155](https://github.com/avelino/outl/issues/155), [#160](https://github.com/avelino/outl/issues/160); open: [#158](https://github.com/avelino/outl/issues/158), [#159](https://github.com/avelino/outl/issues/159) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [sync.md](../sync.md), [privacy.md](../privacy.md) |
| **Invariant** | root `CLAUDE.md` invariant 7 (`peers.json` is deliberately outside the op log — see The opposite direction); `outl-sync-iroh/CLAUDE.md` → Sync request, Membership merge |
| **Guarded by** | `frame_body_length_is_capped` (`crates/outl-sync-iroh/src/engine_sync.rs`), `concurrent_saves_never_lose_an_entry_or_tear_the_file` (`crates/outl-sync-iroh/src/peers_lock.rs`) |

## Why

The sync layer was written as if pairing were a security boundary.
It is not, and two of the ways it is not were exploitable from a single paired device.

**One peer could kill the other's app, repeatedly** ([#155](https://github.com/avelino/outl/issues/155)).
Every sync frame starts with a 4-byte length header.
The reader trusted that number and sized its buffer from it — before reading a byte of body, and before checking whether the sender was even on the same workspace.
A header of `0xFFFFFFFF` asked the receiver to reserve ~4 GiB.
On mobile that is an immediate OS kill; on desktop it is an OOM crash.
The device on the other end does not have to be malicious, only buggy, and the crash repeats on every reconnect.
The sharpest detail is that the pairing handshake in the *same crate* already capped its payload at 64 KiB (`pairing.rs:212`) — the sync read path simply never got the same guard.

**The peer list silently lost entries** ([#160](https://github.com/avelino/outl/issues/160)).
`peers.json` was written with a plain overwrite: no atomic replace, no lock.
Four writers race for it in normal operation — pairing persistence, the ~5s membership gossip tick, the inbound address refresh, and, across processes, the GUI plus the MCP server plus `outl sync` running against one workspace.
Two of them doing read-modify-write means one saves its stale copy over the other's change.
A freshly paired device disappears, or a removal undoes itself, and nothing reports it.
A reader catching the file mid-write gets truncated JSON and drops that cycle's work.
The op log solved exactly this for itself years earlier with serialized atomic appends; `peers.json` was the one piece of persistent state left out.

Two further holes were found in the same audit and are **still open**: `peer remove` does not revoke ([#158](https://github.com/avelino/outl/issues/158)) and pairing accepts an unverified identity ([#159](https://github.com/avelino/outl/issues/159)).
They are why this RFC is `Accepted` and not `Shipped`.

## What we chose

**`checked_frame_body_len` is the single owner of "is this declared length believable?"** (`crates/outl-sync-iroh/src/engine_sync.rs:360`).
It validates the 4 header bytes against `MAX_FRAME_BODY` (256 MiB) and returns an error before any allocation happens.
Every frame read in the crate goes through it, so there is no second opinion about the ceiling.

The allocation itself is also no longer sized from the header.
`Vec::with_capacity(4 + body_len.min(64 * 1024))` reserves at most 64 KiB up front and grows as bytes actually arrive.
That is the load-bearing half: the cap stops the absurd claim, and incremental growth means even a *legal* 256 MiB header cannot be used to make the receiver reserve 256 MiB for a body the sender never sends.

**`PeersStore::mutate_locked` is the single owner of every `peers.json` write** (`crates/outl-sync-iroh/src/peers.rs:486`).
The sequence is: take a cross-process `PeersWriteLock` flock on a sibling `.peers.lock`, **re-read the current file from disk inside the lock**, apply the mutation to that fresh copy, then write it back through `atomic_write_json`.
Re-reading inside the lock is what closes the lost update — an in-memory copy taken before the lock is stale by definition, and serializing writes on a stale copy still loses data.
`atomic_write_json` (`crates/outl-sync-iroh/src/peers_lock.rs:56`) mirrors `outl_core::snapshot::write_to_disk`, so a crash leaves either the old file or a stale `.tmp`, never a torn target.

**Membership merge is add-only.**
Gossip-learned entries may add a peer and may never clobber a known one, and the merge drops self and undialable entries.
That is what keeps a concurrent gossip tick from being an unlogged mutation of the trust set.

**Partial progress on #158, deliberately.**
`SyncProtocolHandler::serve` now checks the connection's authenticated `remote_id()` against `peers.json`, read fresh per connection, and closes `unknown-peer` when it is absent (`engine_sync.rs:729`).
That is the receiver-side half of revocation.
It does not finish the issue, because membership gossip re-adds the removed device within seconds.

## Why not the alternatives

**Grow the buffer incrementally and skip the cap.**
Half the fix, and it was tempting because it needs no constant to argue about.
It leaves the receiver reading an unbounded stream from a peer that never stops sending, so the OOM moves from one allocation to a slow one.
A ceiling makes the refusal immediate and legible in a log.

**Cap at the pairing handshake's 64 KiB.**
Consistent with the existing guard and wrong for this path: a first-sync op batch from a real workspace exceeds it easily, so the cap would refuse legitimate traffic and the guard would be turned off within a release.
256 MiB is chosen to be far above any plausible batch and far below "kills the process".

**Validate the workspace id before reading the frame, and rely on that.**
Appealing because it looks like authentication.
It does not help — the frame is read *in order* to learn the workspace id, and #155's premise is a peer that already passed pairing, so it would pass this check too.
Trust does not remove the need for a bound.

**Put `peers.json` behind an in-process mutex.**
Cheapest correct-looking fix, and it does not survive the actual topology.
The GUI, the MCP server and `outl sync` are separate processes against one workspace, so an in-process lock serializes three of the four writers and leaves the interesting race intact.
A flock is the only thing that spans processes.

**Model the peer set as an `Op` and let it converge through the log.**
The default position under invariant 7, and it was considered.
It costs correctness in the opposite direction: the op log is the thing the peer set gates access to, so making membership converge through it means a peer that should be refused can hand you the ops that say it is trusted.
Bootstrapping has to sit outside the thing it bootstraps.
The consequence is stated below rather than hidden.

**Ship #155 and #160 only after #158 and #159 are designed.**
The tidy option: one RFC, one coherent trust model.
It costs a live denial-of-service and a live data-loss bug in production for however long the design takes.
#155 was labelled P1 and marked active in production; #158 and #159 are P2 and `status:needs-design`.
Shipping the two mechanical fixes and writing down the two open holes is the honest split.

## The opposite direction

**Refusing a frame is now a way to break sync, not only to survive one.**
Before the cap, an oversized declaration crashed the receiver.
Now it errors, and the *mirrored* case matters: a legitimate frame above 256 MiB is refused with exactly the same error as an attack.
Nothing distinguishes them in the log, and nothing tells the user that sync is failing because a batch got too big rather than because a peer is hostile.
Today no code path produces such a batch — but nothing enforces that, and the first feature that does will surface as "sync stopped working".

**A lock is a new way to hang.**
`PeersWriteLock::acquire` blocks until the lock is free, on the caller's thread, in a synchronous `save`.
It is short by construction, and a stale lock from a killed process is now a way for a peer write to stall where the old plain overwrite would simply have proceeded — and corrupted.
The trade is deliberate; the failure mode is different, not absent.

**The receiver-side peer check makes removal look like it works.**
This is the sharpest inversion in this RFC.
`removed_peer_is_denied_sync` passes, so `peer remove` genuinely refuses the next inbound connection — and then membership gossip re-adds the peer within seconds and it works again.
The half-fix is *more* misleading than the original behaviour, because the user can now observe a denial and reasonably conclude they are protected.
Until [#158](https://github.com/avelino/outl/issues/158) lands with a tombstone, `peer remove` is not a security boundary, and this RFC exists partly to say so in a place a contributor will find.

**`peers.json` stays outside the op log, against invariant 7's default.**
Stated explicitly because the invariant says the opposite is the default position.
The peer set is a shared file with last-write-wins semantics between devices, which is exactly what the invariant warns about.
It is exempt because it gates access to the log, and the exemption has a cost: two devices can disagree about the peer set forever, and there is no convergence story for that disagreement.
Add-only merge keeps it from destroying entries, and add-only merge is also precisely why a removal cannot propagate — which is #158.

**Fail-open in the merge.**
Dropping undialable entries keeps the set clean and means a peer that is temporarily unreachable can be skipped rather than kept.
Combined with add-only, the peer set drifts toward "everything anyone has ever seen", which is the shape #158 has to fix.

## How it cannot regress

1. **Invariants.**
   `outl-sync-iroh/CLAUDE.md` → Sync request states the two-part receiver check (workspace id **and** `remote_id()` in `peers.json`, read fresh per connection) and names #158 as the reason the peer check exists at all.
   The same file's Membership merge row states the add-only rule (never clobber a known entry, drop self, drop undialable).
   Root `CLAUDE.md` invariant 7 is the rule this RFC documents an exception to, and the exception is written here rather than in the invariant so the invariant stays absolute for everything else.

2. **Tests.**
   - `frame_body_length_is_capped` (`crates/outl-sync-iroh/src/engine_sync.rs`) is the #155 guard.
     Its doc comment says the declared length is attacker-controlled and must be rejected *before* it can size an allocation, and it asserts the `0xFFFFFFFF` claim, one byte over the ceiling, the ceiling itself, an empty body and a small body.
     Testing the exact boundary is what stops a future "simplify" from turning the check into a loose sanity heuristic.
   - `concurrent_saves_never_lose_an_entry_or_tear_the_file` (`crates/outl-sync-iroh/src/peers_lock.rs`) is the #160 guard.
     It runs many threads through the full read-modify-write the production writers do, and asserts every add survives and every observer parses the file whole.
     Its doc comment records that the old plain `std::fs::write` failed it both ways, so a reader cannot mistake it for a redundant test.
   - `merge_unknown_never_clobbers_a_known_entry` (`crates/outl-sync-iroh/src/peers.rs`) pins the "never clobber" half of the add-only merge.
     `merge_skips_self`, `merge_adds_unknown_and_dedups_known` and `merge_skips_unreachable_peer` (`crates/outl-sync-iroh/src/engine_membership.rs`) pin the rest.
   - `removed_peer_is_denied_sync` (`crates/outl-sync-iroh/tests/regression.rs`) pins the receiver-side half of #158 — a peer absent from `peers.json` is refused `unknown-peer` on a fresh connection.

   **One gap, named.**
   Nothing tests the pairing window or the self-declared device id from #159 — **none found, gap**.
   There is no fix to guard yet.

## Scope

**Not covered — revocation ([#158](https://github.com/avelino/outl/issues/158), open).**
`peer remove` deletes a local entry and refuses the next inbound connection from that id, and then membership gossip resurrects the entry.
Real revocation needs a tombstone that propagates, and possibly a workspace-identity rotation so an old device cannot rejoin at all.
Until then the shared workspace id behaves like a permanent, un-revokable key: a lost or stolen paired laptop cannot be locked out.

**Not covered — pairing authentication ([#159](https://github.com/avelino/outl/issues/159), open).**
Two distinct holes, both live.
While the host is armed (a ~2-minute window) the **first** device to connect on the pairing channel is accepted and handed the workspace identity, with no PIN, challenge or confirmation that it is the device the invite was meant for.
And the joiner's device id is read from the handshake payload and stored as-is, never checked against the connection's authenticated identity.
A malicious joiner can therefore claim a trusted device's id and overwrite that device's stored address, making the legitimate device unreachable until it re-pairs.
The sketched fixes are a secret in the invite proven by HMAC, verifying the payload id against the authenticated identity on both sides, and a handshake timeout that re-arms on failure so a junk connection cannot consume the window.

**Not covered — op signing.**
Ops are unsigned, so a paired device can forge ops claiming another device's actor id.
[#38](https://github.com/avelino/outl/issues/38) open question 5, and out of scope for both open issues above.

**Not covered — transport, workspace identity and address resolution.**
Those are [RFC 0038](0038-sync-transport-and-workspace-identity.md).
