# CLAUDE.md — outl-core

The kernel. Tree CRDT, op log, storage trait. Nothing else.

If you break this crate, you corrupt the user's tree on sync.
**There is no second chance** to win back trust if that happens. Treat every
change as production-bound.

## What this crate owns

- `Op` enum and `LogOp` envelope
- HLC timestamps (wrapper over `uhlc`)
- `NodeId`, `ActorId` (ULID-based)
- Fractional indexing
- The CRDT itself: `do_op`, `undo_op`, `apply_op`, `creates_cycle`
- Append-only `OpLog`
- `Storage` trait + `JsonlStorage` (one file per actor, syncable via
  iCloud / Syncthing / shared FS) + `MemoryStorage` (test double)
- Domain models: `Workspace`, `Page`, `Journal`, `Block`, `Property`, `Tag`

## What this crate does NOT own

- Markdown parsing/rendering → `outl-md`
- Sidecar `.outl` JSON → `outl-md`
- CLI / TUI → `outl-cli`, `outl-tui`
- Network sync → `outl-sync` (phase 2)

If you find yourself reaching for `comrak`, `ratatui`, `iroh`, or anything
file-format related: **stop**. You're in the wrong crate.

## The five invariants

This crate exists to maintain these. They are properties of the algorithm
proven in Kleppmann et al. 2022.

1. **Convergence (SEC).** All replicas applying the same set of ops in any
   order produce the same materialized tree.
2. **Commutativity after reordering.** `apply(a, b, c)` == any permutation.
3. **Idempotency.** `apply(op); apply(op)` == `apply(op)`.
4. **Tree invariant.** Materialized state is always a valid tree.
5. **No silent loss.** Every op stays in the log, even ones turned into no-ops
   by cycle detection.

## Op log is the only sync surface

Any per-block (or per-page) state that must converge between devices —
fold flags, pinned status, whatever ships next — lands as an `Op`
variant on this enum. Never as a field of `SidecarBlock`, a key in a
shared JSON file, or anything else that depends on iCloud / Syncthing
to merge file contents. Those transports are last-write-wins per file
and lose concurrent writes silently.

`Op::SetCollapsed` is the canonical example. Anatomy of a new
"per-block UI state that needs to sync" Op:

- A variant with `node`, the desired value, and an `old_*` field.
- `do_op` captures the old value and applies the new one to a side
  table (`HashMap` / `HashSet`) inside `Tree`.
- `undo_op` restores the captured `old_*`.
- A read accessor on `Tree` (e.g. `is_collapsed(node) -> bool`).
- Storage `op_touches_node` covers the new variant.

Anything cheaper than this in the design discussion is wrong —
correctness across devices is not optional.

The test battery in `tests/` is the operational expression of these. If you
change `tree.rs`, every one of those tests must still pass.

## Algorithm reference

The paper:
**Kleppmann, Mulligan, Gomes, Beresford. "A highly-available move operation
for replicated trees." IEEE TPDS 2022.**
<https://martin.kleppmann.com/papers/move-op.pdf>

OCaml reference implementation by the authors:
<https://github.com/martinkl/crdt-tree-move>

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

Always walk to root or until cycle confirmed. **A non-transitive cycle check
is wrong** and will fail `cycle_chain.rs`.

## Files

```
src/
├── lib.rs              # public API surface
├── id.rs               # NodeId, ActorId (ULID wrappers)
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
└── property_based.rs        # proptest, asserts SEC across permutations
```

## Coverage targets

- **Crate overall:** > 90%
- **`tree::do_op`, `tree::undo_op`, `tree::apply_op`, `tree::creates_cycle`:
  100%** (no exceptions)

Use `/coverage outl-core` to check.

## Things to never do here

- ❌ Take a dependency on `outl-md`, `outl-cli`, `outl-tui`, or `iroh`
- ❌ Bring back SQLite, rusqlite, or any binary store. `JsonlStorage`
  is the only persistent backend; cross-device sync depends on
  per-actor files that iCloud / Syncthing can merge.
- ❌ Add an `Op` variant without `old_*` fields (undo will be impossible)
- ❌ Skip the cycle check in `do_op` for `Move`
- ❌ Remove an op from the log because it was a no-op (silent loss)
- ❌ Compare HLCs without including actor as tiebreak
- ❌ Use `unwrap()` outside of tests
- ❌ Use `unsafe` without a multi-line comment documenting invariants

## When you're adding a new Op variant

Use the `/new-op <Name>` slash command. It walks through all 7 places that
need to change.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-core -- -D warnings`
3. `cargo test -p outl-core`
4. `/coverage outl-core` — must show 100% on the four critical functions
5. Invoke `crdt-invariant-checker` agent
6. If you touched `do_op`/`undo_op`/`apply_op`/`creates_cycle`: invoke `paper-verifier`

Only then is the change ready.
