# CLAUDE.md — outl-core

The kernel.
Tree CRDT, op log, storage trait.
Nothing else.

If you break this crate, you corrupt the user's tree on sync.
**There is no second chance** to win back trust if that happens.
Treat every change as production-bound.

## What this crate owns

- `Op` enum and `LogOp` envelope
- HLC timestamps (wrapper over `uhlc`)
- `NodeId`, `ActorId` (ULID-based).
  `NodeId::from_slug(slug)` is the **single owner** of the deterministic page/journal-root id derivation (`sha256("outl-page:" + slug)[..16]`).
  Every path that materialises a page root routes here — in-app `open_or_create`, `outl-md`'s external-`.md` reconcile, `outl-actions::desync` recovery.
  Two paths (or two devices) then converge on the **same** root id for a slug instead of splitting the page across two competing roots.
  `outl_actions::page::page_id_from_slug` is a thin wrapper kept for its call sites.
- `WorkspaceId` — the stable, **shared** workspace identity (one per workspace, the same bytes on every paired device), persisted at `<root>/.outl/workspace-id`.
  This is NOT the local path: the P2P transport keys its gossip topic on this id so two devices at different paths sync as one workspace, and pairing makes the joiner adopt the host's id.
  Read-or-generated on first open (migration-safe); never written into the clean markdown.
  See `outl-sync-iroh/CLAUDE.md` → "Workspace identity is a stable shared id, NOT the path".
- Fractional indexing
- The CRDT itself: `do_op`, `undo_op`, `apply_op`, `creates_cycle`
- Append-only `OpLog`
- `Storage` trait + `JsonlStorage` (one file per actor, syncable via iCloud / Syncthing / shared FS) + `MemoryStorage` (test double)
  - **Read-side glued-op recovery.**
    `JsonlStorage::reload` parses each line with a streaming `serde_json::Deserializer`.
    A line carrying concatenated JSON objects with no separating newline (`…}}}{"ts":…` — the signature of an interleaved, non-atomic concurrent append) is recovered into all its ops instead of dropped.
    This is a read-side safety net only; writers must still serialize their appends (the corruption was produced by an unsynchronized `outl-sync-iroh` write).
    Dedup-by-op-id makes re-reading a recovered op harmless.
    See `docs/storage.md` → Concurrency / Failure modes.
- Domain models: `Workspace`, `Page`, `Journal`, `Block`, `Property`, `Tag`
- Materialized-state **snapshot** boot cache (`snapshot.rs`): a projection of the tree + block text that short-circuits full op-log replay on boot (#109/#128).
  It is **not** a `Storage` responsibility — the snapshot is a *local* cache and is written straight to `<root>/.outl/snapshots/snap-<actor>.bin` (never on the file-sync surface, never through the op log).
  `Workspace` is the single owner: it reads via `snapshot::read_from_disk` on boot and writes via `snapshot::write_to_disk` (both the synchronous `save_snapshot` and the background threshold writer).
  The op log stays the source of truth — a missing / stale / corrupt snapshot is silently ignored and boot falls back to a full replay, so the snapshot can never corrupt state.
  `<root>/ops/` (the op log) is deliberately **not** a dotfile because it must sync; `<root>/.outl/snapshots/` deliberately **is**, because it must not.
  The replay cutoff is a **per-actor vector clock** (`SnapshotBody.cutoff: BTreeMap<ActorId, Hlc>`), never a single global HLC — boot replays, per actor, every op above that actor's mark plus every op of an actor the snapshot never saw.
  A single global cutoff silently drops a low-HLC op from a lagging peer delivered after the snapshot (#156 Half 2); the delta comes from `Storage::ops_since_per_actor`.

### Snapshot dir has exactly one owner — the `Workspace`, keyed off `root`

The snapshot directory is derived **only** from the workspace `root` (`<root>/.outl/snapshots`), never from the storage's `ops_dir`.
This was a real bug (#156): `JsonlStorage` used to derive its own `ops_dir.parent()/snapshots`.
But production passes `ops_dir = <root>/ops` (not `<root>/.outl/ops`), so the storage read `<root>/snapshots` while the background writer wrote `<root>/.outl/snapshots`.
They never met: snapshot boot was inert in production, while every test (which used `<root>/.outl/ops`) passed.
The fix removed snapshot I/O from the `Storage` trait entirely: storage owns the op log, the workspace owns the snapshot cache, and there is now a single path derivation.
Never re-add `save_snapshot` / `load_snapshot` to `Storage` — that reintroduces the two-owners divergence.

### Block text is two-tier, not one live `Doc` per block

`Workspace`'s `ContentStore` does **not** keep a live Yrs `Doc` resident for every block.
That was the cause of issue #108: a vault in the hundreds-of-thousands-of-blocks range held 0.5-1GB of resident docs and iOS jetsam killed the app on open.

Instead it keeps two tiers, both reconstructed on open from the op log:

- `text: HashMap<NodeId, String>` — the materialized string of every block.
  The hot read path behind `Workspace::block_text`.
  Cheap, roughly the text size.
- `cache: DocCache` — a bounded LRU (`DOC_CACHE_CAP = 512`) of live `Doc`s, only for blocks being edited or merged right now.
  A cold block is rebuilt on demand via `ContentStore::ensure_doc` (private, in `src/content.rs`), which replays that block's `Edit` ops from the log into a fresh `Doc`.
  Yrs is a CRDT, so update order does not change the result — convergence is preserved.

`open_with_storage` replays in **two passes**.
Pass 1 applies every op to the tree/log (`Edit` is a no-op on the tree).
Pass 2 groups `Edit` ops by node and materializes one `Doc` at a time, so the open-time memory peak is a single live doc rather than one per block.

This is a materialization change only: the op log stays the source of truth, the `Doc`/string are projections, and the public surface (`block_text`, `build_text_replace_update`, `apply`) is unchanged.
The resident `OpLog` still holds every `Op::Edit`'s `text_op` bytes (the cheaper second copy of history); shrinking that is the separate per-page op-log shards work, not this change.

## What this crate does NOT own

- Markdown parsing/rendering → `outl-md`
- Sidecar `.outl` JSON → `outl-md`
- CLI / TUI → `outl-cli`, `outl-tui`
- Network sync → `outl-sync-iroh` (P2P via iroh, default transport; file/iCloud opt-in)

If you find yourself reaching for `comrak`, `ratatui`, `iroh`, or anything file-format related: **stop**.
You're in the wrong crate.

## The five invariants

This crate exists to maintain these.
They are properties of the algorithm proven in Kleppmann et al. 2022.

1. **Convergence (SEC).**
   All replicas applying the same set of ops in any order produce the same materialized tree.
2. **Commutativity after reordering.** `apply(a, b, c)` == any permutation.
3. **Idempotency.** `apply(op); apply(op)` == `apply(op)`.
4. **Tree invariant.**
   Materialized state is always a valid tree.
5. **No silent loss.**
   Every op stays in the log, even ones turned into no-ops by cycle detection.

## Op log is the only sync surface

Any per-block (or per-page) state that must converge between devices — fold flags, pinned status, whatever ships next — lands as an `Op` variant on this enum.
Never as a field of `SidecarBlock`, a key in a shared JSON file, or anything else that depends on iCloud / Syncthing to merge file contents.
Those transports are last-write-wins per file and lose concurrent writes silently.

`Op::SetCollapsed` is the canonical example.
Anatomy of a new "per-block UI state that needs to sync" Op:

- A variant with `node`, the desired value, and an `old_*` field.
- `do_op` captures the old value and applies the new one to a side table (`HashMap` / `HashSet`) inside `Tree`.
- `undo_op` restores the captured `old_*`.
- A read accessor on `Tree` (e.g.
  `is_collapsed(node) -> bool`).
- Storage `op_touches_node` covers the new variant.

Anything cheaper than this in the design discussion is wrong — correctness across devices is not optional.

The test battery in `tests/` is the operational expression of these.
If you change `tree.rs`, every one of those tests must still pass.

## Algorithm reference

The paper: **Kleppmann, Mulligan, Gomes, Beresford. "A highly-available move operation for replicated trees.
IEEE TPDS 2022.** <https://martin.kleppmann.com/papers/move-op.pdf>

OCaml reference implementation by the authors: <https://github.com/martinkl/crdt-tree-move>

Core algorithm sketch:

```
apply_op(new_op):
    if new_op.ts > log.last().ts:
        do_op(new_op)
        log.append(new_op)
    else:
        undone = []
        while not log.empty() and log.last().ts > new_op.ts:
            op = log.pop()
            undo_op(op)
            undone.push(op)

        do_op(new_op)
        log.append(new_op)

        for op in undone.reverse():
            do_op(op)
            log.append(op)
```

`do_op` for `Op::Move`:

```
do_op(op):
    if op is Move:
        old_parent = tree.parent(op.node)  // preserved on the LogOp for undo
        old_position = tree.position(op.node)
        if creates_cycle(op.node, op.new_parent):
            // NO-OP on the materialized tree
            // but the LogOp goes into the log unchanged
            return
        tree.set_parent(op.node, op.new_parent, op.position)
```

`creates_cycle(node, new_parent)`:

```
n == new_parent OR new_parent is descendant of n (recursive)
```

Always walk to root or until cycle confirmed.
**A non-transitive cycle check is wrong** and will fail `cycle_chain.rs`.

## Files

```
src/
├── lib.rs              # public API surface
├── id.rs               # NodeId, ActorId (ULID wrappers)
├── workspace_id.rs     # WorkspaceId — stable shared workspace identity (.outl/workspace-id)
├── hlc.rs              # HLC timestamps (uhlc wrapper)
├── op.rs               # Op enum, LogOp envelope, serde
├── fractional.rs       # Fractional indexing (position between siblings)
├── tree.rs             # THE algorithm — do_op, undo_op, apply_op, creates_cycle
├── log.rs              # OpLog (append-only, ordered by HLC)
├── storage/
│   ├── mod.rs          # trait Storage
│   ├── jsonl.rs        # JsonlStorage (only persistent backend)
│   └── memory.rs       # MemoryStorage (test double, no disk)
├── workspace.rs        # Workspace entry point
├── page.rs             # Page model (projection over op log)
├── journal.rs          # Journal (page with date-key)
├── block.rs            # Block (tree node, with Yrs TextRef for content)
├── property.rs         # Property (key-value on block or page)
└── tag.rs              # Tag (page reference with classification semantics)

tests/
├── convergence.rs           # 3 replicas, random ops in different orders
├── cycle.rs                 # classic A↔B move cycle
├── cycle_chain.rs           # A→B→C with concurrent C→A
├── concurrent_edit_move.rs  # block edited and moved simultaneously
├── concurrent_delete_edit.rs# delete wins, edit registered
├── late_op.rs               # old-ts op forces reorder
├── idempotency.rs           # apply N times == apply 1 time
├── fractional_index.rs      # concurrent inserts in same gap
├── large_log.rs             # 10k ops stress test
├── property_based.rs        # proptest: SEC for Create+Move, fwd-vs-reversed
└── convergence_property.rs  # proptest: full-op-mix convergence suite (below)
```

## Convergence property suite (`tests/convergence_property.rs`)

The definitive guard for the SEC claim.
It generates bounded random op programs across up to 4 actors with globally-unique, monotonic-per-actor HLCs.
The op mix is `Create` / `Move` / delete=`Move`→trash / `SetProp` / `SetCollapsed`.
It delivers them to multiple replicas under random permutations and random duplication.
Every op carries a unique HLC so the idempotency dedup never silently drops two distinct ops.
The comparison is a `BTree`-keyed snapshot of the **full** materialized state: node parent+position, every property binding, and the collapsed set.
That is stronger than `common::assert_trees_equal`, which compares nodes only.
It is deterministic (no wall clock; permutations driven by seeded xorshift) and shrinks to a minimal counterexample on failure.

Properties and the invariants (above) they guard:

1. `convergence_under_reordering` — SEC + commutativity under any permutation, not just reverse.
2. `idempotent_under_duplication` — idempotency: 1–3× redelivery == once.
3. `concurrent_moves_never_cycle` — tree invariant + no silent loss.
   Concurrent cycle-forming moves never materialize a cycle, the no-op move still lives in every replica's log, and all replicas converge.
4. `hlc_actor_tiebreak_is_deterministic` — equal physical+logical, different actor resolves to the same winner on every replica.
5. `late_op_undo_redo_round_trips` — the `undo_op`→`do_op` reorder path is a faithful round-trip (a late op forces a full undo/redo of the log).

### Regression: `Op::Create` honors the cycle guard

`Op::Create` runs `creates_cycle` before inserting, exactly like `Op::Move`.
This was a real bug the convergence suite surfaced.
The `Op::Create` branch used to do a bare `entry().or_insert((parent, pos))` with no cycle check.
So a `Create(node, parent)` whose `parent` was already a descendant of `node` inserted `node → parent` and closed a loop (a prior `Move` re-parents something under `node` under reordering).
That violates invariant #4 and then panics `creates_cycle` on the malformed tree.
A cycle-forming `Create` is now a no-op on the materialized tree (the op still goes into the log).
Undo is safe because a node only ever comes into existence through its own `Create` (`Move` never inserts a new entry), so a cycle-skipped `Create` leaves `node` absent and `undo_op`'s `remove(node)` is a no-op.
The deterministic regression is `create_respects_cycle_guard` (asserts no cycle, C stays unmaterialized, all ops logged, across every delivery order); the full-surface `convergence_under_reordering` property exercises it under random programs.

## Coverage targets

- **Crate overall:** > 90%
- **`tree::do_op`, `tree::undo_op`, `tree::apply_op`, `tree::creates_cycle`: 100%** (no exceptions)

Use `/coverage outl-core` to check.

## Things to never do here

- ❌ Take a dependency on `outl-md`, `outl-cli`, `outl-tui`, or `iroh`
- ❌ Bring back SQLite, rusqlite, or any binary store.
  `JsonlStorage` is the only persistent backend; cross-device sync depends on per-actor files that iCloud / Syncthing can merge.
- ❌ Add an `Op` variant without `old_*` fields (undo will be impossible)
- ❌ Skip the cycle check in `do_op` for `Move`
- ❌ Remove an op from the log because it was a no-op (silent loss)
- ❌ Compare HLCs without including actor as tiebreak
- ❌ Use `unwrap()` outside of tests
- ❌ Use `unsafe` without a multi-line comment documenting invariants

## Reuse-first

This crate is the **foundation**: every other crate consumes its types.
Before adding a new primitive (a `Tree` accessor, an `Op` variant, an `id` helper), grep for an existing one — even partial matches are worth wrapping rather than duplicating.
`Tree` accessors in particular cluster around the same `HashMap` — prefer one more `properties_of`-style method over two callers each filtering the map by hand.

Root [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations) has the workspace-level policy.

## When you're adding a new Op variant

Use the `/new-op <Name>` slash command.
It walks through all 7 places that need to change.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-core -- -D warnings`
3. `cargo test -p outl-core`
4. `/coverage outl-core` — must show 100% on the four critical functions
5. Invoke `crdt-invariant-checker` agent
6. If you touched `do_op`/`undo_op`/`apply_op`/`creates_cycle`: invoke `paper-verifier`

Only then is the change ready.
