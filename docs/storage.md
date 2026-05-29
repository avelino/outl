# Storage

`outl-core` does not know what disk looks like. It speaks to storage
through a single trait.

## The trait

```rust
pub trait Storage: Send + Sync {
    /// Append an op. Must be durable before returning Ok.
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError>;

    /// Return all ops with HLC > ts, in HLC order.
    fn ops_since(&self, ts: HLC) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops touching the given node.
    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError>;

    /// Return all ops created by the given actor.
    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError>;

    /// Return the most recent HLC per actor (vector clock for sync).
    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, HLC>, StorageError>;

    /// Iterate all ops in HLC order. Used for full replay.
    fn all_ops(&self) -> Result<Box<dyn Iterator<Item = Result<LogOp, StorageError>> + '_>, StorageError>;

    /// Snapshot of materialized state for fast reload.
    fn snapshot(&self) -> Result<Snapshot, StorageError>;

    /// Restore from a snapshot (does not clear op log).
    fn restore(&mut self, snapshot: Snapshot) -> Result<(), StorageError>;
}
```

`Snapshot` is the serialized materialized tree. It's an optimization: at
startup we deserialize the snapshot, then replay any ops appended after it.

`StorageError` is the storage trait's typed error (`thiserror`).

---

## Default backend: JsonlStorage (multi-device) / SqliteStorage (legacy)

Two backends ship today:

- **`JsonlStorage`** writes to `ops/ops-<actor>.jsonl` — one
  append-only JSONL file per device. iCloud Drive (and any other
  file-level sync transport) syncs each actor's jsonl independently,
  so two devices never collide at the filesystem layer. This is the
  default used by `outl-mobile` and the multi-device workflow.
- **`SqliteStorage`** writes to `.outl/log.db` (WAL mode, ACID). The
  original single-device backend; still useful when no sync transport
  is in play.

The `Storage` trait abstracts both. Workspaces using `JsonlStorage`
are the multi-device path (mobile + TUI sharing an iCloud workspace);
SQLite remains available for users who only ever edit on one machine.

The SQLite schema below documents the legacy backend.

### Schema

```sql
CREATE TABLE IF NOT EXISTS ops (
    ts_physical INTEGER NOT NULL,
    ts_logical  INTEGER NOT NULL,
    actor       BLOB    NOT NULL,
    node_id     BLOB    NOT NULL,
    op_kind     TEXT    NOT NULL,
    op_data     BLOB    NOT NULL,
    PRIMARY KEY (ts_physical, ts_logical, actor)
);

CREATE INDEX IF NOT EXISTS ops_node     ON ops (node_id);
CREATE INDEX IF NOT EXISTS ops_actor    ON ops (actor);
CREATE INDEX IF NOT EXISTS ops_ts       ON ops (ts_physical, ts_logical);

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ts_physical INTEGER NOT NULL,
    at_ts_logical  INTEGER NOT NULL,
    at_actor       BLOB    NOT NULL,
    state_blob     BLOB    NOT NULL,
    created_at     TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
```

`op_data` is `bincode`-serialized `Op` (compact, fast).

WAL mode is enabled (`PRAGMA journal_mode=WAL`) so the TUI can read while
the watcher writes.

### Why SQLite

- Single-file database, zero ops.
- ACID guarantees: durability of `append_op` is real.
- Embedded everywhere Rust runs (rusqlite + bundled feature).
- Concurrent read with WAL.
- The op log is naturally key-value; SQLite is overkill but not bloated.

### Why not RocksDB / sled / fjall

- RocksDB: heavier dep, C++ linkage pain.
- sled: ABI churn, unstable.
- fjall: newer, less battle-tested.

SQLite wins on boring proven.

---

## Roadmap backend: ChronDbStorage (issue #1)

[ChronDB](https://chrondb.com/) is a git-backed database with native
time-travel queries. The win for outl:

- **History as a feature**, not an afterthought. Every op is a git commit.
- **Time-travel queries**: "show me the workspace as of 2026-04-01".
- **Branching**: workspace branches that can be merged.

### What ChronDB needs to gain first

- **Embedded mode** — no external server, ships as a library like SQLite.
- **Secondary indices** — fast lookup by `node_id` and `actor`.
- **Stable Rust client** — without that, integration is painful.

Until those land, ChronDB is the future, not the present.

### How the switch will happen

When ChronDB is ready, the PR is roughly:

1. Add `crates/outl-core/src/storage/chrondb.rs` implementing `Storage`.
2. Add `ChronDbStorage` to the workspace types.
3. Add `outl init --backend chrondb` flag to `outl-cli`.
4. Document migration: `outl migrate --to chrondb` reads sqlite ops, writes
   them to chrondb in order.

No change in `outl-core/src/tree.rs`. No change in `outl-md`. No change in
the TUI. That's the whole point of the trait.

Tracked: <https://github.com/avelino/outl/issues/1>.

---

## What `outl-core` does NOT know

- File paths — storage opens itself.
- Locking — storage handles its own concurrency.
- Workspace layout — storage knows nothing about `pages/` or `journals/`.
- Whether it's running on disk or in memory.

The in-memory implementation (`MemoryStorage`) is a useful test double
and lives in `crates/outl-core/tests/common/` (not shipped).

---

## Concurrency

- `Storage` is `Send + Sync`. Implementations use interior mutability
  (`Mutex`, RwLock) as needed.
- `append_op` is serialized — one writer at a time.
- Reads (`ops_since`, etc.) are concurrent with writes when the backend
  supports it (SQLite WAL does; ChronDB will).

For phase 1, `Workspace` holds `Arc<Mutex<dyn Storage>>` and serializes
all access. Phase 2 may relax this depending on the sync transport's
needs.

---

## Snapshot strategy

After every N ops (default 1000), take a snapshot:

1. Serialize the materialized tree to bytes.
2. Insert into `snapshots` table with the latest HLC.
3. Future startup: load latest snapshot, replay ops after it.

Snapshots are optional. A workspace with no snapshot replays the full log.
For phase 1, snapshots are a "nice to have" — implement only after the
log gets noticeably slow.

---

## Failure modes

| Failure | Detection | Recovery |
|---------|-----------|----------|
| `append_op` fails to commit | `Result` propagated to caller | Caller decides; the in-memory tree should be considered stale; `outl doctor` can reload from disk |
| SQLite file corrupted | `outl doctor` checks integrity via `PRAGMA integrity_check` | Restore from snapshot if available; otherwise full op replay from `.md` files (best-effort) |
| Sidecar lost | `outl doctor` detects missing `.outl` | Regenerate from op log by re-rendering the page |
| HLC clock skew | `uhlc` clamps to avoid runaway logical counter | Tracked in HLC config; rare in practice |
