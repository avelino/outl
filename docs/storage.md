# Storage

`outl-core` does not know what disk looks like.
It speaks to storage through a single trait.

## The trait

```rust
pub trait Storage: Send + Sync {
    /// Append an op. Must be durable before returning Ok.
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError>;

    /// Return all ops with HLC > ts, in HLC order.
    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops touching the given node.
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops created by the given actor.
    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError>;

    /// Return the most recent HLC per actor (vector clock for sync).
    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError>;

    /// Return all ops in HLC order. Used for full replay on open.
    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError>;

    /// Per-actor delta for snapshot boot: every op whose HLC is above the
    /// cutoff of its OWN actor (or whose actor is absent from the map).
    /// Default impl filters `all_ops`; backends may override for speed.
    fn ops_since_per_actor(
        &self,
        cutoff: &BTreeMap<ActorId, Hlc>,
    ) -> Result<Vec<LogOp>, StorageError>;
}
```

Snapshots are **not** a `Storage` responsibility — see [Snapshot strategy](#snapshot-strategy).
The op log is all `Storage` owns.

`StorageError` is the storage trait's typed error (`thiserror`).

---

## The only persistent backend: JsonlStorage

`JsonlStorage` is the storage.
It's what every client (`outl-cli`, `outl-tui`, `outl-mobile`) opens.
There is no flag, no config knob, no fallback to anything else.

### Layout

```text
<workspace>/
└── ops/
    ├── ops-<this-actor>.jsonl    ← we only ever write here
    ├── ops-<peer-actor>.jsonl    ← read-only mirror of another device
    └── ...
```

Each device writes to **exactly one** file, named by its actor id.
Reads merge every `ops-*.jsonl` in the directory back into a single HLC-ordered op log.
That's it.

### Why "one file per actor"

This is the whole reason JSONL exists in the first place. iCloud Drive, Syncthing, Dropbox, any folder-level sync transport: they all reconcile **per file**.
Last-write-wins per path.
If two devices share one log file they race on every byte; the loser's ops vanish silently.

Per-actor files turn that race into a no-op.
Each device's file is append-only and owned by exactly one writer.
Sync transport ships the bytes; the merge happens inside `outl-core`'s CRDT, not at the filesystem layer.
Zero coordination, zero conflicts, zero data loss.

### Why JSONL specifically

- **Append-only writes** map to the filesystem cleanly.
  No WAL, no schema, no transactions to coordinate.
- **Line-delimited** means partial-write recovery is trivial: the loader skips any malformed tail line and keeps going.
- **Human-readable in a pinch.** `tail -f ops-*.jsonl` to watch what's happening; `jq` to inspect a single op.
- **`serde_json` already in the dependency graph** for the JSON envelope.
  Zero new C dependencies.

### Boot reads an index, not the whole log (RFC #137 Front A)

`JsonlStorage` keeps a bounded LRU of hot ops plus a per-actor **offset index** (`ops-<actor>.idx`, HLC → byte offset) and a per-node **secondary index** (`ops-<actor>.nodes.idx`).

On `reload` (boot) the loader streams each `.jsonl` line with a **parse-lite** pass.
That pass pulls only the two fields index-building needs — the op's HLC and the node it touches — and deliberately skips deserializing the heavy payload (`Op::Edit`'s `text_op` byte array above all).
It builds the offset + node indexes and leaves the LRU **empty**.
It does not reparse or re-allocate every op into RAM, which is what made open time (and iOS memory) scale with total history rather than with what boot actually needs — the offset index plus a small snapshot delta.

The full ops are read back **lazily on demand** through the offset index: a single `seek` + one-line parse per op (`read_op_at`), preferring a warm LRU hit when there is one.
The `Storage` read methods are driven off the index — `all_ops`, `ops_since`, `ops_since_per_actor`, `ops_for_actor`, `ops_for_node`, `last_ts_per_actor`.
So they return the **complete** op set — the same set + HLC order as before — regardless of what the LRU currently holds.
The LRU is purely a RAM bound now; the index is the complete logical view.
`last_ts_per_actor` and `ops_since_per_actor` (the snapshot-boot delta) answer straight from the index keys, so the common boot touches only the index and the recent tail, never the full log.

### Why the directory is named `ops/`, not `.ops/`

iCloud Documents and a few other sync transports skip dot-prefixed paths during cross-device sync.
A dotted directory silently breaks multi-device workspaces, with no visible failure mode until the user opens the second device and sees nothing.
The non-dotted name pays a "visible directory" cost for guaranteed sync coverage.

### What lives outside `ops/`

- `.outl/config.toml` — actor id and creation timestamp.
  Local to the device; not synced (each peer mints its own).
- `.outl/.lock` — workspace lock file.
  Local, never synced.
- `.outl/orphans.log` — diagnostic from the reconcile pipeline.
  Local.
- `.outl/peers.toml` — peer registry for P2P sync.
  Local.

Anything that doesn't make sense to share between devices stays under `.outl/`.
The synced surface is `ops/` plus the `.md` / `.outl` (sidecar) projection.

---

## The test double: MemoryStorage

`MemoryStorage` is a pure `Vec<LogOp>`, no disk (and no snapshot — an in-memory workspace has no `root` to cache under).
Used by:

- `Workspace::open_in_memory` — when a caller wants a workspace that never touches the filesystem.
- The test suites of `outl-core`, `outl-actions`, `outl-cli` — every place that previously called `SqliteStorage::open_in_memory()`.

Not a sync backend.
No per-actor file, no merging.
Lives only to keep tests fast.

---

## Roadmap backend: ChronDbStorage (issue #1)

[ChronDB](https://chrondb.com/) is a git-backed database with native time-travel queries.
The win for outl:

- **History as a feature**, not an afterthought.
  Every op is a git commit.
- **Time-travel queries**: "show me the workspace as of 2026-04-01".
- **Branching**: workspace branches that can be merged.

### What ChronDB needs to gain first

- **Embedded mode** — no external server, ships as a library.
- **Secondary indices** — fast lookup by `node_id` and `actor`.
- **Stable Rust client** — without that, integration is painful.

Until those land, ChronDB is the future, not the present.

### How the switch will happen

When ChronDB is ready, the PR adds `outl-core/src/storage/chrondb.rs` implementing `Storage`, plus an `outl init --backend chrondb` flag in `outl-cli`.
The `Storage` trait absorbs the new impl — no change in `outl-core/src/tree.rs`, no change in `outl-md`, no change in the TUI.
That's the whole point of the trait.

Tracked: <https://github.com/avelino/outl/issues/1>.

---

## What `outl-core` does NOT know

- File paths — storage opens itself.
- Locking — `outl-core::WorkspaceLock` is a separate concern, handled at the workspace boundary, not inside storage.
- Workspace layout — storage knows nothing about `pages/` or `journals/`.
  Those live one layer up.
- Whether it's running on disk or in memory.

---

## Concurrency

- `Storage` is `Send + Sync`.
  `JsonlStorage` uses `RwLock` around its in-memory cache; reads are concurrent, writes serialize.
- `append_op` writes one line, then flushes.
  Crash-safe at line granularity: a partial write produces an unparseable tail line, which the loader skips on next open.
- **Glued-op recovery on read.**
  `JsonlStorage::reload` parses each line with a streaming `serde_json::Deserializer`.
  A line carrying two (or more) concatenated JSON objects with no separating newline (`…}}}{"ts":…`) is recovered into all its ops instead of being dropped.
  That signature is what an interleaved, non-atomic concurrent append produces; the recovery means an external writer that glued two ops together never silently loses the user's content.
  A recovered line is logged at `warn` (it still signals a writer that should have serialized).
  The op log dedups by op id, so re-reading a recovered op that another file also carries is harmless.
  Writers inside this repo must still serialize their appends — recovery is the read-side safety net, not a license to write unsynchronized (see `outl-sync-iroh` → append-serialization invariant).

---

## Snapshot strategy

A snapshot is a **local boot cache** — a projection of the materialized tree + block text that short-circuits full op-log replay on open (#109/#128).
It is owned by `Workspace`, **not** by `Storage`: `Storage` owns the op log, and the snapshot is written straight to `<root>/.outl/snapshots/snap-<actor>.bin`, never through the backend.

Why `<root>/.outl/snapshots` and not next to `ops/`?
The op log at `<root>/ops` must sync (iCloud / Syncthing), so it is deliberately not a dotfile.
The snapshot must **not** sync — it is a per-device cache — so it lives under the dotted `.outl/`.
Deriving the snapshot dir from the storage's `ops_dir` was the #156 bug.
Production passes `ops_dir = <root>/ops`, so the reader looked in `<root>/snapshots` while the writer used `<root>/.outl/snapshots`, and boot was silently inert.
The workspace `root` is now the single source of the snapshot dir.

Boot + delta:

1. `snapshot::read_from_disk` loads the body; a missing / stale / corrupt snapshot is silently ignored and boot falls back to a full replay (the op log is the source of truth — the snapshot can never corrupt state).
2. Hydrate the tree + block text from the body.
3. Replay the **per-actor delta**: for each actor `A`, every op with `hlc > cutoff[A]`, plus every op of an actor absent from the cutoff (unseen when the snapshot was taken).

The cutoff is a per-actor vector clock (`BTreeMap<ActorId, Hlc>`), not a single global HLC.
A single cutoff tracks only the snapshotting actor's high-water mark, so a legitimately-low-HLC op from a lagging peer delivered after the snapshot would fall below it and vanish from the tree though it's durably in storage (#156).
Per-actor, each op is compared against its own actor's mark, and because an actor's HLCs are monotonic the boundary is exact — no drop, no double-apply (idempotency covers the equal-HLC boundary).

Writing is driven by `Workspace::set_snapshot_policy(enabled, op_threshold)` (in-band background writer, off the calling thread) and `Workspace::save_snapshot` (synchronous, on graceful shutdown).
Snapshots are optional: a workspace with none replays the full log.

---

## Failure modes

| Failure | Detection | Recovery |
|---------|-----------|----------|
| `append_op` fails to flush | `Result` propagated to caller | Caller decides; the in-memory tree should be considered stale; `outl doctor` can reload from disk |
| Partial-write tail in a `.jsonl` | `JsonlStorage::reload` logs the unparseable line via `tracing::warn!` and skips it | Truncate that line; the next valid op is fine |
| Glued ops on one line (`…}}}{"ts":…`) from an interleaved concurrent append | `JsonlStorage::reload` streams every concatenated JSON object off the line and warns | No action — both ops are recovered on next open; dedup makes a double-read harmless |
| Sidecar lost | `outl doctor` detects missing `.outl` | Regenerate from op log by re-rendering the page |
| HLC clock skew | `uhlc` clamps to avoid runaway logical counter | Tracked in HLC config; rare in practice |

---

## What is **not** here anymore

Pre-0.5.0, outl shipped a second persistent backend: `SqliteStorage` (`.outl/log.db`, WAL mode).
It was the default for local-only workspaces and the source of an entire class of "writes go through but vanish on the other client" bugs.
`outl-cli` opened it via SQLite, `outl-tui` and mobile followed `config.toml` and opened JSONL on the same workspace, and the two backends diverged silently.

0.5.0 dropped SQLite entirely.
There is one persistent backend.
Cross-device sync is no longer a config decision; it's the only mode.
See `CHANGELOG.md` for the migration path from a 0.4.x SQLite workspace.
