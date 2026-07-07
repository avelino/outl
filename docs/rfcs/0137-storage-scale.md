# RFC 0137 — Storage scale: constant RSS, then constant boot/sync

**Status**: Phase A shipped; Phase B pending
**Issue**: [#137](https://github.com/avelino/outl/issues/137)
**Branch**: `feat/storage-scale-rfc-137`
**Tracking**: 11 PRs across two phases (A done, B pending)

## Why

`JsonlStorage` keeps every op ever written in resident memory. The cache
field was `RwLock<Vec<LogOp>>` at [`storage/jsonl.rs:42`][src-cache];
`reload()` reads every `ops-<actor>.jsonl` line-by-line into that `Vec`
([`storage/jsonl.rs:92-130`][src-reload]); `boot_from_full_replay`
([`workspace.rs:230-269`][src-full]) loads all of them through `OpLog`
on top. Phase 1 snapshot (#128) speeds up boot but doesn't reduce RSS —
the storage cache still mirrors everything.

Measured on M-series Mac, release build, via `boot_scale_bench.rs`:

| ops    | boot_snap | boot_full | rss_snap | rss_full | source      |
|--------|-----------|-----------|----------|----------|-------------|
| 50k    | 27 ms     | 31 ms     | 37 MB    | 54 MB    | measured    |
| 100k   | 55 ms     | 64 ms     | 66 MB    | 98 MB    | measured    |
| 250k   | ~140 ms   | ~160 ms   | ~160 MB  | ~230 MB  | extrapolated |
| 500k   | ~280 ms   | ~320 ms   | ~300 MB  | ~470 MB  | extrapolated |
| 1M     | ~550 ms   | ~640 ms   | ~600 MB  | ~900 MB  | extrapolated |

Per-op cost: ~0.55 µs boot, ~590 bytes RSS. Both linear.

**Boot is not the wall.** Even at 1M ops, 550 ms is acceptable.
**RSS is the wall.** Each `Op::Edit` costs ~590 bytes of resident memory
forever — even after a snapshot, even after the block was deleted.
Mobile hits the wall first (iOS jetsam kills above ~500 MB).

## Decisions (locked)

1. **LRU + mmap first (Phase A), per-page shards second (Phase B).**
   Phase A bounds RSS at the storage layer without a format break.
   Phase B bounds boot and sync at the workspace layer. Both are needed
   because a single page (10-year journal) can still grow without bound.

2. **`PageScope` default = `PerPage` in new workspaces** (Phase B).
   Existing workspaces stay on `Global` until migrated. Migration CLI
   is opt-in.

3. **Include iroh-blobs snapshot transfer** as PR #9 in Phase B
   (closes Phase 2 of #128). Reduces wire bytes for fresh peer pairing.

4. **Single long-running branch** `feat/storage-scale-rfc-137`, merged
   to `main` as one PR (squash). Each sub-PR is a logical commit.

## Phase A — constant RSS (SHIPPED)

Replace `Vec<LogOp>` with a bounded LRU (cap configurable in
`outl.toml` via `[storage] lru_cap`, default 20k ops desktop, 5k on
mobile). Add a per-actor offset index `(actor, hlc) → file offset` so
cold ops stay addressable without buffering the whole log.
`Workspace::apply_lru_cap(cap)` is called by every long-lived client
AFTER boot finishes re-materialising Yrs `Doc`s via `ops_for_node` —
boot needs every op in RAM, the long-running client sheds cold history
afterwards.

Result: RSS ≈ constant (LRU cap + index + mmap window), regardless of
history. No `.jsonl` format change. No migration. No client breaks.

### Phase A PRs (DONE)

| # | Title | Status |
|---|-------|--------|
| 1 | `perf(core): per-actor offset index sidecar` | shipped — `crates/outl-core/src/storage/index.rs` (new), `JsonlStorage::reload` populates + persists `ops-<actor>.idx` |
| 2 | `perf(core): replace Vec<LogOp> cache with bounded LRU` | shipped — `cache: RwLock<LruCache<Hlc, LogOp>>`, `JsonlStorage::open_with_cap`, `[storage] lru_cap` in `outl-config` |
| 3 | `perf(core): Workspace::apply_lru_cap shrinks cache post-boot` | shipped — `Storage::resize_cache` (default no-op), `Workspace::apply_lru_cap`, wired in TUI + desktop + mobile |

**Milestone verification**:
- `cargo test -p outl-core` passes (CRDT invariants intact, 100%
  coverage on the four critical functions preserved).
- New unit tests cover the LRU cap (`bounded_lru_evicts_old_ops`,
  `reload_with_bounded_lru_keeps_cap`) and the offset index
  (roundtrip, missing/corrupt file, rebuild from jsonl).
- `boot_scale_bench.rs` is the long-running scale measurement
  (`cargo test -p outl-core --release --test boot_scale_bench
  boot_scale_100k -- --ignored --nocapture`).

## Phase B — constant boot/sync (#37 / Part 2 of `sync.md`) — PENDING

`PageScope`, per-page op log shards, `migrate-to-per-page-ops`, four
clients update. Builds on Phase A — each page-scope reuses the LRU +
mmap storage layer.

This is a 1-2 month refactor with a format break and migration CLI.
It cannot be done blind in a single session — the op log format is
the sync correctness invariant, and a bad migration corrupts every
paired device. Tracked separately.

### Phase B PRs (PENDING)

| # | Title | Touches |
|---|-------|---------|
| 5 | `refactor(core): PageScope on Storage trait (default Global)` | `storage/mod.rs`, `storage/jsonl.rs`, `workspace.rs` (back-compat: `Global` matches current behaviour byte-for-byte) |
| 6 | `feat(core): per-page op log shards for new workspaces` | `storage/jsonl.rs` (write path), `outl-cli init` (default scope = `PerPage`) |
| 7 | `feat(cli): migrate-to-per-page-ops command` | `outl-cli/src/cmd/migrate_to_per_page_ops.rs`, idempotent + `.v0.bak` backup |
| 8 | `refactor: clients use page scopes` | TUI, mobile, desktop, CLI — `open_or_create` dispatches by scope, boot loads page lazily on navigation |
| 9 | `feat(sync): iroh-blobs snapshot transfer (closes #128 phase 2)` | `outl-sync-iroh` — ship snapshot binary then delta ops on fresh pair |

**Phase B prerequisites before landing**:
1. Workspace backup to test the migration against (a real 3-year vault).
2. Wire-format review — per-page shards change what iroh ships.
3. `crdt-invariant-checker` agent pass on any change to `apply_op`.
4. `markdown-roundtrip-tester` agent pass on the migration output.

## Open questions (Phase B)

- **LRU cap default for mobile** — resolved: 5k (mobile pins
  `min(cfg.lru_cap, 5_000)` in `outl-mobile/src-tauri/src/workspace_open.rs`);
  desktop default 20k.
- **Index persistence** — resolved: sidecar `ops-<actor>.idx` (Phase A).
- **mmap strategy** — deferred to follow-up; Phase A reads cold ops via
  `File + seek + parse`, which is correct but slower than mmap. Land
  mmap when a profile shows the cold-read path is hot.
- **Snapshot interaction** — resolved: `apply_lru_cap` runs AFTER
  `ops_for_node` finishes re-materialising Yrs `Doc`s at boot (see
  `Workspace::apply_lru_cap`).
- **Throughput cost of cold reads** — pending; needs a micro-bench
  before mmap lands. Today `ops_for_node` returns only the resident
  tail, which is what boot-from-snapshot needs (delta-edited nodes
  are still warm).

## Non-goals (explicit)

- **Op log compaction** (#110). Orthogonal. The LRU doesn't delete ops
  from disk; compaction is a separate horizon (`Undo horizon`).
- **Block text CRDT replacement**. Yrs stays. The two-tier `Doc` cache
  (#108's fix) stays.
- **ChronDB backend** (#1). Out of scope. The `Storage` trait changes
  planned for Phase B (PageScope) are forward-compatible with ChronDB.

## References

- Issue: <https://github.com/avelino/outl/issues/137>
- Companion: `docs/sync.md` Part 2 (the design Phase B implements)
- Related issues: #37, #128, #110, #109, #108
- Bench: `crates/outl-core/tests/boot_scale_bench.rs`
- Phase A code:
  - `crates/outl-core/src/storage/index.rs` (offset index sidecar)
  - `crates/outl-core/src/storage/jsonl.rs` (LRU + cap + index wiring)
  - `crates/outl-core/src/storage/mod.rs` (`Storage::resize_cache`)
  - `crates/outl-core/src/workspace.rs` (`Workspace::apply_lru_cap`)
  - `crates/outl-config/src/schema.rs` (`[storage] lru_cap`)

[src-cache]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/storage/jsonl.rs#L42
[src-reload]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/storage/jsonl.rs#L92
[src-full]: https://github.com/avelino/outl/blob/main/crates/outl-core/src/workspace.rs#L230

