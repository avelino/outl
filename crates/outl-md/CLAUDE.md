# CLAUDE.md — outl-md

The boundary between **what the user sees** (clean markdown) and **what the
core processes** (op log with stable IDs).

If this crate misroutes a block during matching, the user perceives "outl
deleted my work" — even if the op log still has it. Treat matching with the
same paranoia as the CRDT.

## What this crate owns

- Parse `.md` (clean, no IDs) → outline AST
- Render outline AST → `.md` (clean, no IDs)
- Read/write `.outl` sidecar (JSON, dotfile) — current version `2`,
  reads v1 transparently (handles backfilled on load). The sidecar
  is **structural metadata only** (id, line, indent, content hash,
  ref handle). State that must converge between devices (fold flags,
  pinned, etc.) goes through the op log in `outl-core`, never here.
- The 3-level matching algorithm (external edit → reconstruct IDs)
- Diff (old AST + new AST + old sidecar blocks) → minimal sequence of
  `Op`s, preserving `ref_handle` verbatim on level-1/2 matches. The
  old-handle lookup is O(N) overall (HashMap by id), not O(N²) — never
  reintroduce a linear scan per new block. **`diff_to_ops` only emits
  structural ops (Create / Move / SetProp); a second pass inside
  `reconcile_md::sync_block_text` walks the AST + new sidecar in
  lockstep and emits one `Op::Edit` per block whose text differs from
  the workspace. Skipping that pass silently zeroes text across
  devices — local stays fine, peer replays an empty tree, iCloud
  ships the empty `.md` back.**
- **`outline_ops`** — pure `Vec<OutlineNode>` AST helpers
  (`flat_count`, `path_for_index`, `insert_sibling_after/before`,
  `indent_at_path`, `outdent_at_path`, `delete_at_path`,
  `move_up_at_path`, `move_down_at_path`, …). They operate on an
  in-flight AST that hasn't been parsed back into a workspace yet, so
  they sit in `outl-md` (UI-agnostic, no `Workspace`) rather than in
  `outl-actions`. The TUI re-exports them through a one-line shim at
  `outl-tui/src/outline_ops.rs`; the mobile client consumes them
  directly.
- **Inline tokenization** (`inline.rs`) — `**bold**`, `[[refs]]`,
  `#tags`, `((blk-XXXXXX))`, `!((blk-XXXXXX))` — and `ref_at_cursor`
  (resolves to `RefTarget::Page`, `Journal`, `Tag`, or `Block`).
  **UI-agnostic.** TUI, future Tauri GUI, and mobile clients all
  consume the same `InlineTok` / `RefTarget` types and map them to
  their own primitives (`Span`, HTML, `AttributedString`,
  `AnnotatedString`).
- **Block index** (`block_index.rs`) — `NodeId → BlockEntry`,
  `ref_handle → NodeId`, `NodeId → [BlockReference]` (reverse refs),
  `(slug, dfs_path) → NodeId` for location lookup. Population is
  two-phase (`collect_page_blocks` then `collect_page_refs`) so reverse
  edges survive arbitrary page-load order during the initial build.
  Lookups are O(1).
- **Workspace index** (`index.rs`) — page-level (`slug → PageEntry`,
  backlinks) plus block-level (re-exports the `BlockIndex` API).
  Public surface includes `resolve_block_ref(handle)`, `block_by_id`,
  `block_at_location(slug, &[usize])`, `block_refs_to(id)`,
  `iter_blocks`, `block_count`, `search_block_text(query, limit)`.
  `block_at_location` is the O(1) replacement for scanning
  `iter_blocks()` to find the entry for a known `(page, dfs_path)`,
  e.g. when the TUI translates a keyboard chord onto a specific block.
- **Slugify** (`slug.rs`) — `[[Avelino]]` → `pages/avelino.md`. The
  user-facing name is preserved verbatim in the page's `title::`
  property.
- **`derive_ref_handle(NodeId) -> String`** (`sidecar.rs`) —
  deterministic: `blk-` + last 6 chars of the ULID's Crockford base32,
  lowercased. Same input always yields the same handle so two devices
  agree on what `((blk-XXXXXX))` means. On a collision inside a single
  workspace, the **second** block to land gets its handle lazily
  expanded one character at a time (drawing from the same ULID tail)
  until unique — both the winner and the loser stay independently
  resolvable. The sidecar still records the deterministic 6-char form;
  the expanded handle lives in `BlockEntry.ref_handle` in memory and
  in the workspace handle map.
- **`BlockEntry.text_fold: String`** — lowercased cache of the block's
  `text`, populated at index build. Powers `search_block_text` without
  allocating per keystroke. Public field, but consumers must not build
  `BlockEntry` by hand — go through the index population path so
  `text_fold` stays consistent with `text`.
- **`reconcile_md_with_page_id(ws, hlc, md_path, page_id, orphan_log)`**
  (`reconcile.rs`) — like `reconcile_md`, but pins the page node id
  used when no sidecar exists yet. `outl_actions::ingest_md_file` passes
  `page_id_from_slug(slug)` so blocks attach to the same node
  `open_or_create` just created under root. Without this, the two paths
  mint different ids and blocks hang off a phantom node that never
  appears in `page list`. Call sites that ingest a file as a first-class
  page (import, serve, orphan scanners) must use this variant; the bare
  `reconcile_md` is for in-place external-edit reconcile where the
  sidecar already carries the correct id.

## What this crate does NOT own

- The op log → `outl-core`
- File watching / debounce → `outl-cli`
- Reconcile TUI → `outl-tui`
- Network sync → `outl-sync` (phase 2)

## The 3-level matching algorithm

When an external save lands on `pages/foo.md`:

1. **Parse** new `.md` → AST without IDs.
2. **Load** `foo.outl` (sibling of `foo.md`) → AST with old IDs and content hashes.
3. **Match** new ↔ old blocks at 3 confidence levels:

| Level | Confidence | Criteria | Action |
|-------|-----------|----------|--------|
| 1 | High | `content_hash` exact match, same parent (by hash) or identical structure | Preserve ID, emit `Move` if position changed |
| 2 | Medium | Normalized Levenshtein similarity > 80%, same parent OR position within ±2 lines | Preserve ID, emit `Edit` (+ `Move` if needed), log warning |
| 3 | Low / no match | Falls through | New ULID for new block; old block becomes `Delete` (`Move` to `TRASH_ROOT`); record in `.outl/orphans.log` |

**Hard rule:** a block that drops to level 3 must appear in `orphans.log`
before being deleted. **Silent deletion is a P0 bug.**

## Sidecar format

Current version: `2`. Full spec in
[`docs/markdown-format.md`](../../docs/markdown-format.md#the-outl-sidecar).

```json
{
  "version": 2,
  "page_id": "01HXY8KJZQ9T8M7VN3P2R6S4A0",
  "last_synced_hash": "sha256:...",
  "last_synced_at": "2026-05-24T11:22:00-03:00",
  "blocks": [
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A1",
      "line": 1,
      "indent": 0,
      "content_hash": "sha256:...",
      "ref_handle": "blk-r6s4a1"
    }
  ]
}
```

- `content_hash` = SHA-256 of the **block's textual content** (not children).
- `ref_handle` = short user-typeable handle for `((blk-XXXXXX))`. v1
  sidecars (no field) load fine — the handle is backfilled in memory
  via `derive_ref_handle`. The next write persists v2. On collision,
  expansion may produce a 7+ char form (see `derive_ref_handle` above).

**Sidecar is not a sync surface.** UI state that must converge between
devices — fold flags, pinned, selection, anything user-meaningful —
goes through the op log (`outl-core`), not here. iCloud / Syncthing
sync the sidecar file as one blob with last-write-wins semantics, so
two devices flipping different fields in the same window lose data.
The op log gives each actor its own jsonl, lets the FS sync per-file
without conflict, and reconverges through the CRDT. See the root
`CLAUDE.md` invariant 7.
- Sidecar lives next to the `.md` as `pages/<slug>.outl` (no leading
  dot). Replicated between devices alongside the `.md`. Don't
  gitignore by default. The dotfile form (`.foo.outl`) was abandoned
  because iCloud Documents skips dotted paths during cross-device
  sync.
- **Stale entries are skipped during index build.** When a sidecar
  block's `content_hash` no longer matches the corresponding block in
  the `.md`, that entry is left out of the workspace index instead of
  polluting it with a wrong subtree. The block reappears in the index
  after the next reconcile updates the sidecar.

## Outl markdown dialect

```markdown
title:: example
status:: active
tags:: #project

- top level block
  priority:: high
  - child block with [[page reference]]
  - child block with ((blk-r6s4a1))
  - expanded inline: !((blk-r6s4a1))
- another top level
```

- `key:: value` lines at top of file = page properties (frontmatter outliner-style).
- `key:: value` lines nested as children of a block = block properties.
- `[[name]]` = page reference (bidirectional link).
- `[[2026-05-24]]` = journal reference (renders as date).
- `#tag` = tag (page reference with classification semantics).
- `((blk-XXXXXX))` = inline block reference (renders as the source block's text).
- `!((blk-XXXXXX))` = block embed (renders source block expanded with subtree).
- `{{query: ...}}` = saved query (phase 3, parse as opaque for now).

**No `id::`, no UUID, no HTML comments** — IDs go in the sidecar only.

## Files

```
src/
├── lib.rs
├── parse.rs        # md → AST (no IDs)
├── render.rs       # AST → md (clean)
├── sidecar.rs      # read/write .outl JSON, derive_ref_handle, content_hash
├── matching.rs     # 3-level matching algorithm
├── diff.rs         # AST diff → Op sequence (takes old_blocks to preserve ref_handle)
├── inline.rs       # InlineTok (Plain/Bold/.../BlockRef/Embed), RefTarget, ref_at_cursor
├── index.rs        # WorkspaceIndex — page-level + block-level facade
├── block_index.rs  # BlockEntry, BlockReference, BlockIndex (id ↔ handle ↔ reverse refs)
├── reconcile.rs    # reconcile_md + reconcile_md_with_page_id (parse → match → diff → apply)
├── slug.rs         # slugify page names
├── view.rs         # render helpers consumed by UIs
└── atomic.rs       # crash-safe write_atomic

tests/
├── roundtrip.rs              # render(parse(md)) == md (property test)
├── external_edit.rs          # light external edit preserves IDs
├── duplicate_block.rs        # Ctrl+D in vscode → first keeps ID, second gets new
├── identical_blocks_swap.rs  # two identical blocks change parents
└── heavy_edit.rs             # >20% content change → level 2 warning

benches/
└── block_index.rs            # resolve / search_block_text on 100k blocks
```

## Bench harness

`cargo bench -p outl-md --bench block_index` measures the cost the
`((blk-XXXXXX))` path adds to the index. Today's numbers (M-series
laptop):

- `resolve(handle)` — ~17 ns at 100k indexed blocks. O(1) HashMap hit.
- `search_block_text(query, limit)` — ~12 ms at 100k blocks (linear
  scan with case-fold + position scoring). Suitable for the
  autocomplete popup the TUI uses today; future fzf-style scoring can
  drop in behind the same signature.

## Invariants

1. **Roundtrip stability.** `render(parse(md))` produces a semantically
   identical `.md` (same tree, properties, content; whitespace may normalize).
2. **No silent block loss.** A block falling to level 3 is always in `orphans.log`.
3. **Sidecar is JSON-valid.** Always. If you can't write valid JSON, you fail.
4. **Sidecar `version` field always present.** Future migrations.
5. **`content_hash`** is `sha256(block.content_text())` consistently. Same hash function across read and write.
6. **`ref_handle` is preserved across level-1 and level-2 matches.**
   `diff_to_ops` reads it from the previous sidecar's block list and
   reuses it verbatim, so a `((blk-XXXXXX))` already written in
   another `.md` keeps resolving even if the handle was once expanded
   past the default 6-char tail.
7. **`derive_ref_handle` is deterministic** — same `NodeId` in, same
   handle out. Two devices building the sidecar independently must
   agree on what `((blk-XXXXXX))` means.

## Things to never do here

- ❌ Write IDs into the `.md` file (use sidecar)
- ❌ Delete a block in matching without recording in `orphans.log` first
- ❌ Match on similarity > 80% across **different parents** without warning
- ❌ Skip the property test in `roundtrip.rs`
- ❌ Use a different hash function in sidecar read vs write
- ❌ Drop sidecar version 1 support when adding version 2 (always backward read)
- ❌ Block on a corrupt sidecar — fall back to "regenerate from op log" via `outl doctor`

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-md -- -D warnings`
3. `cargo test -p outl-md`
4. `/roundtrip` to invoke the markdown-roundtrip-tester agent
5. Manual smoke: create a fixture md, render it back, diff
