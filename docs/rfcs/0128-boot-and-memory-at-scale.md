# RFC 0128 — Boot and memory at scale: the snapshot, the index, and the two defects that cancelled each other

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#128](https://github.com/avelino/outl/issues/128), [#156](https://github.com/avelino/outl/issues/156), [#179](https://github.com/avelino/outl/issues/179), [#207](https://github.com/avelino/outl/issues/207) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [storage.md § Snapshot strategy](../storage.md#snapshot-strategy) |
| **Invariant** | root `CLAUDE.md` invariants 1 and 5; `outl-core/CLAUDE.md` → "Snapshot dir has exactly one owner", "Block text is two-tier" |
| **Guarded by** — snapshot is a projection | `snapshot_and_delta_match_full_replay`, `corrupt_snapshot_falls_back_to_full_replay` (`crates/outl-core/tests/snapshot_equivalence.rs`) |
| **Guarded by** — per-actor cutoff + one owner | `late_low_hlc_op_from_unseen_actor_survives_snapshot_boot` (`crates/outl-core/tests/snapshot_late_op.rs`), `snapshot_written_in_prod_layout_is_read_on_boot` (`tests/snapshot_prod_layout.rs`) |
| **Guarded by** — index-driven boot, lazy content | `reload_uses_fresh_idx_without_reparse` (`crates/outl-core/src/storage/jsonl/tests.rs`), `lazy_full_replay_block_text_matches_eager_snapshot_boot` (`tests/lazy_block_text.rs`) |
| **Guarded by** — schema 4 / postcard | `legacy_bincode_snapshot_falls_back_to_full_replay` (`tests/snapshot_equivalence.rs`), `legacy_peer_snapshot_is_skipped_not_fatal` (`tests/peer_snapshot_preserves_local.rs`) |

This RFC is deliberately thin.
The mechanics — boot sequence, cutoff arithmetic, sidecar freshness, failure-mode table, postcard rationale — are owned by [`docs/storage.md` → Snapshot strategy](../storage.md#snapshot-strategy) and by `outl-core/CLAUDE.md`.
What lives here is the set of decisions, what was rejected, and what got worse.
Per [One owner per fact](../contributing.md#one-owner-per-fact--link-dont-duplicate), anything already stated there is linked, not restated.

## Why

`Workspace::open_with_storage` was O(total history) on every boot, and the cost showed up in four different disguises.

**The CLI paid full replay per invocation (#109).**
`outl page get foo` rebuilt the entire world to print one page.
Replay is O(total history), not O(current state), and text editing through the CRDT generates a lot of ops — so the log grows with how much the thing is *used*, not with how big the vault is.
`outl page list` was ~200 ms on a fresh vault and seconds after two years of daily journaling, and a script looping over 50 pages paid 50 full replays back to back.

**Every block that had ever been edited kept a live Yrs `Doc` resident for the whole session (#108).**
One `HashMap<NodeId, Doc>`, never evicted.
At 80k blocks that is 0.5–1 GB, and on iOS jetsam killed the app mid-open.
The tree was cheap; the docs were what jetsam saw.

**On the real workspace the cost was not replay at all — it was reparse (#179).**
66k blocks, 211k ops, a 152 MB op log, instrumented boot:

```
JsonlStorage::reload   ~4.8s (debug) / ~1.4s (release)   ← dominates
snapshot hydrate       ~0.5s  (66,332 nodes)
snapshot HIT, delta = 232 ops                            ← the snapshot was fresh
```

Raw file I/O was 0.07 s.
The time was serde deserializing every line into a full `LogOp`, and `Op::Edit` carries `text_op: Vec<u8>` (a Yrs update) serialized as a JSON number array.
The sync path was far worse: pairing a fresh mobile device, the desktop pushed 211,406 ops and spent **~35 s** computing the delta, because `actor_census`, `local_vector_clock` and `ops_missing_for` each did a full reload plus `all_ops()`.
The mobile app then froze ingesting and reopening.
The offset index the boot needed already existed and was already **persisted** to `ops-<actor>.idx` on every reload — and never loaded back.

**And the fast boot that was supposed to fix all of this had never run in production, which hid something much worse (#156).**
Two coupled defects.
Half 1: the snapshot was written to `<root>/.outl/snapshots` while boot looked in `<root>/snapshots`.
`JsonlStorage` derived the directory from `ops_dir.parent()`, and production passes `ops_dir = <root>/ops` while every test used `<root>/.outl/ops`.
So every test passed and every production boot silently fell back to full replay.
Half 2: the replay cutoff was a single global HLC, so an op made offline on another device — carrying a legitimately *lower* timestamp — sat below the mark and was skipped forever, present on disk and absent from the tree, permanently.
Half 1 kept Half 2 from ever firing.

**Separately, the boot cache's encoder became an adoption blocker (#207).**
`outl-core` is published so other projects can [embed outl as a storage layer](../embedding.md).
[RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141) flags bincode as unmaintained, and it was a direct dependency used in exactly one place.
This was found the only way it could be: a maintainer auditing a PR that embedded outl rejected it because the graph failed their policy gate.

One measurement worth keeping next to all of the above, from [RFC 0137](0137-storage-scale.md): at 1M ops, boot is ~550 ms, which is acceptable.
**Boot was never the wall — RSS was**, at roughly 590 bytes of resident memory per op, forever.

## What we chose

### 1. The snapshot is a projection, not a source of truth (#128)

The op log stays append-only, in full, per actor.
The snapshot is a boot shortcut: hydrate the materialized state, then replay only the ops after it.
A missing, stale or corrupt snapshot is silently ignored and boot falls back to full replay.

This is the decision the other three lean on.
Because no snapshot can corrupt state, a format break costs one slower boot (decision 4), a wrong cutoff is a correctness bug rather than an unrecoverable one, and the whole cache can be deleted at any time.
There is deliberately no `Op::Snapshot`: a snapshot is not a mutation, and putting it in the log would make the cache sync, which makes it a second source of truth.

### 2. The cutoff is a per-actor vector clock, and the snapshot directory has exactly one owner (#156)

`SnapshotBody.cutoff` is a `BTreeMap<ActorId, Hlc>`, never a single `Hlc`.
Boot replays, per actor, every op above that actor's mark **plus every op of an actor the snapshot never saw**, via `Storage::ops_since_per_actor`.
Because an actor's own HLCs are monotonic the boundary is exact, and idempotency covers the equal-HLC edge.

The directory fix went further than aligning two paths: **snapshot I/O left the `Storage` trait entirely.**
`Storage` owns the op log; `Workspace` owns the snapshot cache and derives its directory from `root` alone.
Two owners was the actual bug, and `outl-core/CLAUDE.md` says outright never to re-add `save_snapshot` / `load_snapshot` to `Storage`.

### 3. Boot reads an index, not the whole log; block text and `Doc`s are both lazy (#179, #108)

The persisted `.idx` / `.nodes.idx` sidecars are now **loaded back** when fresh, with the tail reindexed when the `.jsonl` grew and a full rebuild only when they are missing or corrupt.
They are dot-prefixed on purpose, and the reason is a correctness one rather than tidiness — see `outl-core/CLAUDE.md`, and the mirrored risk in [The opposite direction](#the-opposite-direction).

Content became two tiers, both reconstructed from the op log: a `HashMap<NodeId, String>` of materialized text behind `Workspace::block_text`, and a bounded LRU of live `Doc`s (`DOC_CACHE_CAP = 512`) only for blocks being edited or merged right now.
At 80k blocks that is ~5–10 MB instead of 0.5–1 GB.

Full-replay boot then stopped materializing text eagerly: pass 1 applies every op to the tree, and pass 2 records which nodes carry `Edit` history in a `pending` set so `block_text` rebuilds each string on first read.
The snapshot boot path is unchanged, because it hydrates already-materialized strings rather than replaying.

`Workspace::resident_text_count()` is `pub` on purpose — it is the observability window into the lazy path, so a downstream crate can assert that a read path does not force the whole workspace to materialize.
`from_disk_build_does_not_materialize_workspace` (`crates/outl-actions/tests/backlinks_index_disk.rs`) is the first consumer of that guarantee.

### 4. A deliberate format break, because the boot cache has no migration cost (#207)

`SCHEMA_VERSION` went 3 → 4 and the encoder went bincode → postcard, together.
Old snapshots fail `decode` and land on the path a corrupt snapshot always took.
Postcard is maintained, `MIT OR Apache-2.0`, and varint — smaller on the wire, which also matters because snapshots ship between peers over iroh.

Two hardenings ride along, and both are about a *successful* decode being wrong rather than a failing one:

- **`decode` compares `!=`, not `<=`.**
  A well-formed, correctly-hashed body claiming an *older* schema was being accepted.
  Dead today, because a real schema-3 file is bincode and dies in the parser.
  The moment a future schema keeps postcard and merely appends a field, that is a partial tree read as if it were whole.
- **`compute_hash` and `from_parts` return `Result`.**
  Degrading an encode failure to a default would hash the empty vector on both the write and the verify side, and `sha256([])` compares equal to `sha256([])`.
  The integrity check would keep passing while checking nothing.

## Why not the alternatives

**Do not build a snapshot at all — trust the replay (#33, and this is the version that shipped first).**
[#33](https://github.com/avelino/outl/issues/33) proposed exactly this feature and was **closed on purpose**.
Realistic workspaces replayed in milliseconds, no benchmark showed replay was the bottleneck, and the snapshot brings a pile of new problems: graceful shutdown versus crash, corruption recovery, format forward-compat, multi-process election.
The closing line was *"Trust the replay — reopen when boot time becomes a measured problem on a real workspace."*
That was right, and #179's 1.4 s over 211k ops is the measurement that reopened it.
This is recorded because "we said no first, on principle, and then a number changed our mind" is the part an RFC usually loses.

**Cache parsed page ASTs in an LRU instead (#34).**
Right instinct, wrong target.
Lazy parse of the open page already happened in `load_current` (`crates/outl-tui/src/actions/lifecycle/loading.rs`), and mobile and desktop already did the same through per-page commands.
An AST cache had no caller that could validate it, because every client opens one page at a time.
The actual boot cost was `WorkspaceIndex::build` walking the filesystem instead of deriving from the log.

**Cap the LRU to fix the reparse (#179).**
Tried, and measured: `open_with_cap` drops RSS from 302 MB to 208 MB and **does not change the time at all**.
The cost was deserializing 211k ops, not filling the cache.
This is the clearest case in this RFC of a plausible fix being measured and dropped rather than argued about.

**Invalidate or rebuild the snapshot whenever a peer's op-log file changes (#156, the issue's own second option).**
Cheaper to implement than a vector clock, and it makes the cache useless exactly when it is worth most.
A device syncing with an active peer would rebuild on every delivery, so the workspace that most needs a fast boot never gets one.
A per-actor cutoff costs one `BTreeMap` per snapshot and never invalidates.

**Fix the folder mismatch now and design the cutoff afterwards (#156).**
This is the trap, and it is why the issue was labelled `needs-design` instead of picked up.
See [The opposite direction](#the-opposite-direction) — it is the main thing this RFC exists to record.

**bincode 2 with `config::legacy()` (#207, Option A in the issue).**
It reproduces the 1.x wire format byte for byte, so existing snapshots keep loading, `content_hash` values stay stable, and no schema bump is needed.
It also fails the actual goal: the advisory carries `patched = []` with no version range, so it flags **every** bincode release including 2.x, and a downstream `cargo audit` keeps failing exactly as before.
`bincode 3.0.0` is a tombstone whose entire `lib.rs` is `compile_error!`.

**bincode 2 with `config::standard()` (#207, Option B, the issue's own recommendation).**
Same advisory problem, and it already accepted the schema bump.
Once the bump is being paid for anyway, postcard buys the varint win without keeping a flagged crate in the graph.

**Sync the snapshot through the file transport so a fresh peer boots fast (#128 non-goal).**
Sidecar churn in iCloud is a known UX killer ([#127](https://github.com/avelino/outl/issues/127)).
Peer snapshot transfer belongs on iroh — [RFC 0137](0137-storage-scale.md) Phase B, PR 9.

## The opposite direction

### The coupled-defects trap (#156) — the reason this RFC exists

**Two defects that cancel each other out are more dangerous than either one alone, and the obvious fix for the cheap half is the trigger for the expensive half.**

Half 1 was a performance bug with a two-line fix that any reader arriving from "why are our boots slow?" would make immediately.
Half 2 was permanent, silent, cross-device divergence with no symptom, no error and no log line.
Fixing Half 1 in isolation does not improve anything by itself — it **arms** Half 2.
There is no intermediate state where the workspace is merely faster: the moment snapshot boot starts running, a lagging peer's offline edit can fall below a global cutoff and vanish from the tree while sitting on disk.

Two rules came out of it, and they generalize past this issue:

1. **When an optimization is found to be inert, the first question is not "how do we turn it on" but "what has never run".**
   An inert code path has never been exercised against production data, so its correctness is *unmeasured*, not proven.
   Half 2 had been in the tree the whole time and no test could have caught it, because turning it on was a separate change.
2. **A test suite that uses a layout production does not use proves nothing about the path production takes.**
   Every #156 test passed against `<root>/.outl/ops`.
   Production passes `<root>/ops`.
   That is the entire distance between "covered" and "never ran".
   `snapshot_written_in_prod_layout_is_read_on_boot` exists to keep using the production layout, and it is worthless the moment someone "simplifies" it to the convenient one.

The same issue notes that fixing Half 1 alone *also* re-activates a separate latent text-corruption path — partial edit history read from a bounded cache — which is [RFC 0129](0129-op-log-durability.md) decision 3.
Two independent loaded guns behind one two-line change.

### What decision 2 makes worse

A per-actor cutoff means the delta grows with the number of actors the snapshot has never seen, so a workspace paired with many devices replays more at boot than a single global mark would.
That is the correct trade — replaying an op twice is idempotent, skipping one is permanent — but the boot win degrades as the device count grows, and nothing today measures that curve.

### What decision 3 makes worse, and the mirrored read

Lazy block text moves work from boot to first read: opening a page for the first time after boot pays a per-block `Doc` rebuild the old eager pass had already paid.
The user feels it as a slower first page rather than a slower launch, which is the right place to put it, and it is still worse than free.

The sharper cost is that the lazy path's correctness depends on the node's `Edit` history being **complete** when the string is rebuilt.
The mirror of "boot does not materialize everything" is "a read materializes from a possibly-short history", and that read does not fail — it hands the user block text they never wrote.
Both directions are pinned: `lazy_full_replay_block_text_matches_eager_snapshot_boot` here, and `ops_for_node_surfaces_missing_ops_instead_of_dropping_them` in [RFC 0129](0129-op-log-durability.md).

Trusting the `.idx` sidecar has the same shape, one level down.
Its freshness check validates the tail byte-exactly and then trusts the prefix `[0, max_offset)`.
A *synced* index could therefore arrive torn in the middle with an intact tail, pass the check, and feed a wrong offset into `read_op_at` — a silently dropped op on the index-driven reads.
That vector is closed by keeping the sidecar dot-prefixed and device-local, **not** by the freshness check.
Which makes it the weakest kind of invariant: nothing in the freshness check would notice if someone moved the sidecar onto the sync surface, and the failure would look like ordinary bit-rot.
`outl-core/CLAUDE.md` states the rule for exactly that reason.

### What decision 4 makes worse

Nothing for a local device beyond one slower boot, once.
Cross-version pairing degrades: a peer still on the old build ships a snapshot this build cannot read, so the pair falls back to op replay.
`legacy_peer_snapshot_is_skipped_not_fatal` pins that "skips" never becomes "errors" — an undecodable peer snapshot must be dropped from the candidate scan, not propagated as a failure.

### Is the user told, in any of these cases?

No, and in every case that is deliberate: each degradation lands on the path a corrupt snapshot always took, which is full replay of the source of truth.
The user gets a slower boot, never a wrong tree, and there is nothing for them to do.
The price of that silence is that a snapshot must never be able to **succeed wrongly**, which is what the `content_hash`, the `!=` schema compare and the `Result`-returning `compute_hash` are all defending.
A cache allowed to fail quietly must not also be allowed to lie quietly.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 1 puts the snapshot, the index and the `.md` on the same footing: projections of the log, never a place to fix state.
   Root invariant 5 is why snapshot I/O is deliberately **not** on the `Storage` trait.
   `outl-core/CLAUDE.md` is the enforcing surface and carries four separate rules from this RFC:
   "Snapshot dir has exactly one owner — the `Workspace`, keyed off `root`", including the explicit *never re-add `save_snapshot` / `load_snapshot` to `Storage`*;
   the per-actor-cutoff rule, with #156 Half 2 named as the reason;
   "This crate's dependency graph is public surface", with `SCHEMA_VERSION` and the encoder moving together;
   and "Block text is two-tier, not one live `Doc` per block".
   `docs/storage.md` → [Snapshot strategy](../storage.md#snapshot-strategy) and [Wire format](../storage.md#wire-format-postcard-schema-4) are the single owner of the *how*.
   `docs/embedding.md` → [Dependency policy](../embedding.md#dependency-policy) owns what an embedder's gate finds today.

2. **Tests, per decision.**

   *Snapshot is a projection:* `snapshot_and_delta_match_full_replay`, `corrupt_snapshot_falls_back_to_full_replay`,
   `leftover_tmp_from_crashed_save_is_ignored`, `apply_trigger_writes_snapshot_at_threshold` (`crates/outl-core/tests/snapshot_equivalence.rs`);
   `adopting_peer_snapshot_preserves_local_ops`, `peer_snapshot_cycle_reorder_matches_full_replay` (`tests/peer_snapshot_preserves_local.rs`).

   *Per-actor cutoff and one owner:* `late_low_hlc_op_from_unseen_actor_survives_snapshot_boot` (`tests/snapshot_late_op.rs`);
   `snapshot_written_in_prod_layout_is_read_on_boot` (`tests/snapshot_prod_layout.rs`).

   *Index-driven boot and lazy content:* `reload_uses_fresh_idx_without_reparse`, `reload_reindexes_appended_tail`, `reload_rebuilds_on_missing_or_corrupt_idx`,
   `reload_rebuilds_on_shrunk_jsonl`, `reload_rebuilds_on_node_offset_asymmetry` (`crates/outl-core/src/storage/jsonl/tests.rs`);
   `lazy_full_replay_block_text_matches_eager_snapshot_boot` (`tests/lazy_block_text.rs`);
   `full_replay_boot_defers_block_text_materialization`, `reopen_rebuilds_text_without_resident_docs`, `doc_cache_is_bounded`,
   `evicted_block_rebuilds_from_log` (`crates/outl-core/src/workspace.rs`);
   `from_disk_build_does_not_materialize_workspace` (`crates/outl-actions/tests/backlinks_index_disk.rs`).

   *Schema 4 / postcard:* `legacy_bincode_snapshot_falls_back_to_full_replay` (`tests/snapshot_equivalence.rs`);
   `legacy_peer_snapshot_is_skipped_not_fatal` (`tests/peer_snapshot_preserves_local.rs`);
   the schema-compare units in `crates/outl-core/src/snapshot.rs` covering a version above **and** below `SCHEMA_VERSION`.

   Three of these look deletable and are not:
   `snapshot_prod_layout.rs` is worthless if someone switches it to the convenient `<root>/.outl/ops` layout, because that is precisely the layout under which #156 was invisible;
   `legacy_bincode_snapshot_falls_back_to_full_replay` and `legacy_peer_snapshot_is_skipped_not_fatal` depend on `crates/outl-core/fixtures/legacy-snapshot-schema3.bin` being a **real captured pre-#207 file**,
   so regenerating it with today's encoder would leave both tests green while covering nothing;
   `full_replay_boot_defers_block_text_materialization` reads like an implementation-detail assertion and is the only thing stopping a return to the O(all blocks) pass that froze mobile.

3. **Benchmarks, explicitly not gates.**
   `crates/outl-core/tests/boot_scale_bench.rs` (50k–500k ops, RSS and boot, `--ignored`) and `tests/snapshot_bench.rs`.
   They are development tools: op generation alone costs ~51 s for 12k ops because `append_op` fsyncs, so they can never run in CI.
   The signal is the relative split, not the absolute numbers.

## Scope

**Not covered — constant RSS and per-page op-log shards.**
[RFC 0137](0137-storage-scale.md) owns both: Phase A (bounded LRU, offset index, `apply_lru_cap`) shipped, Phase B (`PageScope`, per-page shards, migration CLI, iroh-blobs snapshot transfer) is pending.
This RFC's decision 3 builds directly on Phase A's index, and RFC 0137 also records why mmap was declined for the cold-read path.

**Not covered — the sync half of #179, and it has not shipped.**
`actor_census` in `crates/outl-sync-iroh/src/engine_sync.rs` still does `JsonlStorage::open` plus `all_ops()`, and `local_vector_clock` derives from it.
So the ~35 s delta computation on a 211k-op log is improved only by Front A's faster `reload`, not removed.
Moving the census and `ops_missing_for` onto the offset index is still open, and lives in `outl-sync-iroh` rather than here.

**Not covered — op-log compaction.**
[#110](https://github.com/avelino/outl/issues/110), Phase 3 of #128, needs an undo horizon and its own UX.
Nothing in this RFC removes a byte from the log; the snapshot only lets boot skip reading it.

**Not covered — the read-side durability rules the index and the lazy text depend on.**
[RFC 0129](0129-op-log-durability.md): skip-and-continue on a damaged record, `MissingOp` on an indexed-but-unreadable op, and the "every `Op::Edit` must be replayable into a fresh empty `Doc`" invariant that makes lazy rebuild safe.

**Not covered — `smallstr 0.3.1`.**
[RUSTSEC-2026-0215](https://rustsec.org/advisories/RUSTSEC-2026-0215), transitive through `yrs`, no patched release, needs an upstream fix.
It is the one flagged crate an embedder's gate still finds, and [`docs/embedding.md`](../embedding.md#dependency-policy) is the single owner of that state.
