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

    /// Persist a snapshot of materialized state for faster startup.
    fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError>;

    /// Load the most recent snapshot, if any.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError>;
}
```

`Snapshot` is opaque bytes (format owned by the impl).
It's an optimization: at startup we load the snapshot then replay ops appended after it.

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
- `.outl/peers.toml` — phase-2 peer registry.
  Local.

Anything that doesn't make sense to share between devices stays under `.outl/`.
The synced surface is `ops/` plus the `.md` / `.outl` (sidecar) projection.

---

## The test double: MemoryStorage

`MemoryStorage` is a pure `Vec<LogOp>` + snapshot slot, no disk.
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

---

## Snapshot strategy

After every N ops (default 1000), take a snapshot:

1. Serialize the materialized tree to bytes.
2. `save_snapshot` persists it.
3. Future startup: `load_snapshot` returns the latest; replay only ops past it.

Snapshots are optional.
A workspace with no snapshot replays the full log.
Implement when the log gets noticeably slow — not before.

---

## Failure modes

| Failure | Detection | Recovery |
|---------|-----------|----------|
| `append_op` fails to flush | `Result` propagated to caller | Caller decides; the in-memory tree should be considered stale; `outl doctor` can reload from disk |
| Partial-write tail in a `.jsonl` | `JsonlStorage::reload` logs the unparseable line via `tracing::warn!` and skips it | Truncate that line; the next valid op is fine |
| Sidecar lost | `outl doctor` detects missing `.outl` | Regenerate from op log by re-rendering the page |
| HLC clock skew | `uhlc` clamps to avoid runaway logical counter | Tracked in HLC config; rare in practice |

---

## What is **not** here anymore

Pre-0.5.0, outl shipped a second persistent backend: `SqliteStorage` (`.outl/log.db`, WAL mode).
It was the default for local-only workspaces and the source of an entire class of "writes go through but vanish on the other client" bugs — `outl-cli` opened it via SQLite, `outl-tui` and mobile followed `config.toml` and opened JSONL on the same workspace, the two backends diverged silently.

0.5.0 dropped SQLite entirely.
There is one persistent backend.
Cross-device sync is no longer a config decision; it's the only mode.
See `CHANGELOG.md` for the migration path from a 0.4.x SQLite workspace.
