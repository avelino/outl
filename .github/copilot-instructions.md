# Copilot review instructions — outl

You are reviewing a pull request in **outl**, a local-first outliner with a CRDT-based tree sync engine, written in Rust.
Read this whole file before commenting.
Your job is **not** a style pass — fmt, clippy, and CI already enforce style.
Your job is the review a Staff/Principal engineer would give: catch correctness, architecture, and scalability problems that humans miss, and only speak when it matters.

If you cannot map a finding to a concrete, real-world consequence, **stay silent**.
Noise costs reviewer attention; a single sharp comment earns trust.

---

## 0. Read these first

- Root `CLAUDE.md` — project-wide invariants and conventions.
- The `CLAUDE.md` inside the crate(s) the PR touches (e.g.
  `crates/outl-core/CLAUDE.md`).
- `CONTRIBUTING.md` — the merge bar and "decisions you don't get to revisit".
- `docs/contributing.md` — the review policy this file mirrors.
- `docs/development.md` — the engineer onramp (build / run / test / debug / CI / release). Load it when the PR touches CI workflows, slash commands, hooks, agents, or anything else a contributor's first 30 minutes depend on.
- `docs/architecture.md`, `docs/crdt.md`, `docs/markdown-format.md` — load the relevant one when the PR touches that area.
- The PR description and any linked issue.

Treat the per-crate `CLAUDE.md` as authoritative over generic Rust opinions.
If your suggestion contradicts it, drop the suggestion.

---

## 1. Gate the PR before reviewing code

**Before reading the diff**, evaluate the PR description:

- Is there a linked issue (`Closes #N`, `Fixes #N`, `Related to #N`)?
- Is the problem the PR solves stated in one paragraph, in plain language?
- For a refactor: is *why now* explicit?
  ("Code is cleaner" is not enough.
  Either it unblocks something concrete, or it pays down debt the description names.)
- For a fix: is the bug behaviour described, with repro or a failing test?
- For a feature: does it match an item on `docs/roadmap.md` or an approved issue?

**If the description fails this gate**, your first and only top-level comment should be:

> Before I can review this PR meaningfully, the description needs a linked issue or a concrete problem statement.
> What real user-facing problem does this solve, and why now?
> If this is exploratory, please mark it as a draft and add an `RFC` label.

Do not proceed to line-level comments until that is fixed.
Reviewing a diff without knowing what problem it solves produces opinions, not review.

**Exception:** typo fixes, doc-only changes under `docs/` or `README.md`, and dependency bumps with a clear changelog link can skip this gate.

---

## 2. Non-negotiable invariants

These are project-level invariants.
A PR that violates any of them is a **blocker**, regardless of how clean the code looks.
Quote the invariant by name in your comment.

1. **Op log is source of truth.** Mutations flow through `Op` → `apply_op` → log.
   The materialized tree and the `.md` files are projections.
   Reject any code that writes to `.md` to "fix" state without going through an `Op`.

2. **Markdown stays 100% clean.** No `id::` lines, no inline UUIDs, no HTML comments carrying state.
   IDs live only in the `.outl` sidecar (a sibling JSON file, not a dotfile — iCloud strips dotted paths).

3. **CRDT follows Kleppmann et al. 2022 literally.** `do_op`, `undo_op`, `apply_op`, and `creates_cycle` must match the paper.
   These four functions have a **100% line and branch coverage requirement**.
   Any new branch without a test is a blocker.

4. **A move that creates a cycle is a deterministic no-op on the materialized tree, but the op still goes into the log.** Removing the op breaks reordering correctness on replay.

5. **Storage is a `trait`, not a struct.** `outl-core` must not import `rusqlite`, `serde_json` writers for file IO, or any concrete backend.
   Everything goes through `dyn Storage`.
   A second persistent backend does not land without an RFC issue first.

6. **Delete is `Move(node, TRASH_ROOT)`**, never physical removal.

7. **Convergent state goes through the op log, never a shared file.** If two actors can disagree about a value and you want them to reconcile, model it as an `Op`.
   The sidecar is for structural matching metadata only (id, position, content hash, ref handle).

8. **Layering.** `outl-core` never depends on UI or CLI crates.
   `outl-actions` is the shared workspace-mutation surface every client (`outl-tui`, `outl-mobile`, `outl-cli`) must call.
   A PR that reimplements an `outl-actions` helper inside a client is a blocker — point at the existing function.

9. **No reintroduction of SQLite, rusqlite, or any binary log format.** Cross-device sync depends on per-actor append-only JSONL.

10. **Settled decisions are off-limits in a PR.** ULID for IDs, `uhlc` for time, MIT license, JSONL-per-actor, Tauri for mobile, iCloud as v0 transport — do not suggest changing these in a code-review comment.
    If a contributor disagrees, the path is an issue, not a PR.

---

## 3. Rust quality bar

Comment when the diff introduces any of the following.
Skip when the existing surrounding code already does it (that's a separate cleanup).

- **`.unwrap()` outside `#[cfg(test)]`** — require `.expect("explicit reason")` or `?` propagation.
  The `expect` message must name the invariant being asserted, not just "should not fail".
- **`.unwrap_or_default()` masking an error path** — if the default is a silent data-loss bug, flag it.
- **`unsafe` in `outl-core`** without a `// SAFETY:` comment naming the invariants the caller relies on.
- **`anyhow` in a library crate** (`outl-core`, `outl-md`, `outl-actions`).
  Libraries use `thiserror` so callers can match on variants.
  `anyhow` is only OK at binary boundaries (`outl-cli`, `outl-tui`).
- **`Box<dyn Error>` as a public return type** — same reason.
- **`String` where `&str` works**, **`Vec<T>` where `&[T]` works**, **owned arg where borrowed works** — but only in public APIs and hot paths; do not bikeshed this on internal helpers.
- **`async fn` with a blocking call inside** (`std::fs`, `std::thread::sleep`, large CPU loop without `spawn_blocking`).
- **Holding a `Mutex`/`RwLock` across an `.await`** — deadlock waiting to happen.
- **Public API change on `outl-core`, `outl-md`, or `outl-actions` without doc-comment update** — the per-crate `CLAUDE.md` should also reflect it.

Skip these (CI / fmt / clippy handle them):

- Import ordering, line width, brace placement.
- Naming conventions clippy already lints.
- `mod` declaration order.

---

### 3.1 Markdown / documentation style

Flag when a `*.md` change introduces hard-wrapped prose (lines broken at ~70/80/100 chars mid-sentence).
Every prose `*.md` in this repo uses [semantic line breaks](https://sembr.org/): one sentence per line, breaking after `.`/`!`/`?` (and sometimes `:`), never at an arbitrary column.

What to flag:

- Prose paragraphs hard-wrapped at a column width.
- A sentence split across two or more lines for no reason.
- Tables rewritten to span multiple lines per row (must stay one row per line).

What to leave alone:

- Code fences, YAML frontmatter, ASCII tree diagrams — preserve exactly.
- Outline content (anything under `note-example/`, real workspace pages, fixtures) — this is data in the outl dialect, not prose docs.
- Single-line list items, headings, link references.

Scope: root `CLAUDE.md`, per-crate `CLAUDE.md`, `docs/*.md`, `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.github/*.md`, `.claude/agents/*.md`, `.claude/commands/*.md`.
Root `CLAUDE.md` has the canonical rule under "Markdown / documentation style".

### 3.2 One owner per fact — link, don't duplicate

Every user-facing fact lives in exactly one `docs/*.md`. `CLAUDE.md` files **link** to it instead of copying the table or chord list.

When reviewing a PR, flag duplication of these surfaces between `docs/*.md` and any `CLAUDE.md`:

| Fact | Canonical home |
|---|---|
| Every keyboard shortcut (TUI + desktop + mobile) | `docs/shortcuts.md` |
| `outl` CLI subcommands | `docs/cli.md` |
| TUI manual (modes, overlays) | `docs/tui.md` |
| Outl markdown dialect + sidecar | `docs/markdown-format.md` |
| CRDT algorithm + invariants | `docs/crdt.md` |
| Storage trait + JSONL backend | `docs/storage.md` |
| Sync model | `docs/sync.md` |
| MCP wiring + recipes | `docs/mcp.md` + `docs/mcp-recipes.md` |
| Config file | `docs/config.md` |
| Theming palette | `docs/theming.md` |
| Dev loop | `docs/development.md` |
| Contributing policy | `docs/contributing.md` |

What a `CLAUDE.md` *should* carry: invariants, architectural decisions you don't get to revisit, crate-specific contracts, the reasoning behind a choice — things a contributor needs *before* touching code, not user reference.

When you spot a PR copying a `docs/` table (or row of it) into a `CLAUDE.md`, **request a change**: replace with a link. Reverse direction is fine — `docs/*.md` linking *into* a `CLAUDE.md` for architectural depth is welcome.

Canonical rule lives at root `CLAUDE.md` → "One owner per fact — link, don't duplicate".

---

## 4. Performance — hot paths only

Comment on performance only when the code is on a path that runs frequently or scales with workspace size.
**Do not flag allocations in setup, error paths, or one-shot CLI commands.**

Paths that are hot in outl:

- `outl_core::tree` — every op apply, every materialized-tree walk.
- `outl_core::log` — every append, every replay (workspace boot, sync pull).
- `outl_md::parse` / `outl_md::render` — every `.md` read/write, every TUI refresh of a buffer.
- `outl_md::index` — backlink index rebuild scales with workspace size.
- `outl_tui` render loop — runs on every keystroke.
- `outl_actions::SyncEngine` work loop — runs on every file event.

In those paths, flag:

- `.clone()` on `String`, `Vec`, or large structs where a borrow would work, and the clone is per-call (not one-time setup).
- `.to_string()` / `format!()` when the caller only needs `&str` or `Display` deferral.
- `Vec::new()` followed by repeated `push` inside a loop where capacity is knowable (`Vec::with_capacity`).
- `HashMap` for small fixed key sets where a `match` or array would do.
- Re-parsing the same markdown / re-walking the same subtree on every keystroke — propose caching with a clear invalidation story.
- Big-O regressions on tree ops or backlink computation.
  Walk the algorithm in the comment.

If unsure whether it's a hot path, ask in the comment — do not assert.

---

## 5. Architecture, scalability, extensibility

This is where a Staff/Principal review earns its keep.
Flag:

### 5.1 Shared primitives catalog — check this before approving any helper

**Before approving a helper that touches anything below, scan the relevant sub-table.** If the diff adds a primitive that overlaps with an entry here, it's a duplicate — block the PR and point at the existing function with `file:line`.

> This catalog is mirrored at root `CLAUDE.md` § "Shared primitives catalog".
> When you edit either copy, sync both — a `PostToolUse` hook (`catalog-sync-guard.sh`) flags drift between the two.

The catalog is grouped by area.
Skim headings, then drill in.

#### 1. Workspace lifecycle, op log, and HLC (outl-core)

| Intent | Use this | File |
|---|---|---|
| Open a workspace (in-memory tests, on-disk JSONL prod) | `outl_core::Workspace::open_in_memory` / `open_with_storage` | `crates/outl-core/src/workspace.rs` |
| Route an op through log → tree (the **only** mutation path) | `outl_core::Workspace::apply(LogOp)` | `crates/outl-core/src/workspace.rs` |
| Read materialized tree / op log / block text from a workspace | `outl_core::Workspace::tree` / `log` / `block_text` | `crates/outl-core/src/workspace.rs` |
| Build a Yrs text-replace update payload for an op | `outl_core::Workspace::build_text_replace_update` | `crates/outl-core/src/workspace.rs` |
| Generate HLC timestamps with actor tiebreak (every op needs one) | `outl_core::HlcGenerator::new` / `next` / `observe` | `crates/outl-core/src/hlc.rs` |
| Wrap an `Op` into a `LogOp` (timestamp + actor) for `apply` | `outl_core::Op` + `outl_core::LogOp` | `crates/outl-core/src/op.rs` |
| Sentinel node ids (`root`, `trash`) | `outl_core::NodeId::root()` / `trash()` | `crates/outl-core/src/id.rs` |
| Per-device identity for ops | `outl_core::ActorId` | `crates/outl-core/src/id.rs` |
| Fractional index for sibling ordering | `outl_core::Fractional` | `crates/outl-core/src/fractional.rs` |

#### 2. Tree reads (outl-core + outl-actions::tree)

| Intent | Use this | File |
|---|---|---|
| Does a node still exist? | `Tree::contains` | `crates/outl-core/src/tree/mod.rs` |
| Parent / position / property of a node | `Tree::parent` / `position` / `property` | `crates/outl-core/src/tree/mod.rs` |
| Iterate every property currently set on a node | `Tree::properties_of` | `crates/outl-core/src/tree/mod.rs` |
| Collapsed flag for a node | `Tree::is_collapsed` / `collapsed_ids` | `crates/outl-core/src/tree/mod.rs` |
| Walk every node in the tree | `Tree::iter_nodes` / `node_count` | `crates/outl-core/src/tree/mod.rs` |
| Children of a parent (in fractional order) | `outl_actions::tree::children_of` | `crates/outl-actions/src/tree.rs` |
| Walk a subtree applying a closure | `outl_actions::tree::walk_subtree` | `crates/outl-actions/src/tree.rs` |
| Sibling after a node + position helpers for inserts | `outl_actions::tree::next_sibling` / `position_after` / `position_for_new_last_child` | `crates/outl-actions/src/tree.rs` |
| Which page does this node sit under? | `outl_actions::tree::enclosing_page_id` | `crates/outl-actions/src/tree.rs` |

#### 3. Block mutations (outl-actions::block + collapsed + todo + quote)

Every entry here routes through `Workspace::apply`.
Reject PRs that build a `LogOp` from a client and call `apply` directly.

| Intent | Use this | File |
|---|---|---|
| Append a single block under a parent | `outl_actions::block::append_block` | `crates/outl-actions/src/block.rs` |
| Append a tree / forest under a parent (uses `BlockTreeSpec` → `BlockTreeOutcome`) | `outl_actions::block::append_tree` / `append_forest` | `crates/outl-actions/src/block.rs` |
| Create sibling after / child under a block | `outl_actions::block::create_after` / `create_under` | `crates/outl-actions/src/block.rs` |
| Edit a block's text | `outl_actions::block::edit_text` | `crates/outl-actions/src/block.rs` |
| Indent / outdent / move up / move down a block | `outl_actions::block::indent` / `outdent` / `move_up` / `move_down` | `crates/outl-actions/src/block.rs` |
| Delete a block (`Move(node, TRASH_ROOT)`, **never** physical) | `outl_actions::block::delete` | `crates/outl-actions/src/block.rs` |
| Toggle block collapsed (converges via `Op::SetCollapsed`) | `outl_actions::collapsed::toggle_block_collapsed` / `set_block_collapsed` | `crates/outl-actions/src/collapsed.rs` |
| Cycle / split / read TODO/DONE state | `outl_actions::todo::cycle_todo` / `split_todo` / `TodoState` / `TODO_PREFIX` / `DONE_PREFIX` | `crates/outl-actions/src/todo.rs` |
| Toggle TODO/DONE on a block in one call | `outl_actions::block::toggle_todo` | `crates/outl-actions/src/block.rs` |
| Read / toggle blockquote state (`"> "` text prefix, CommonMark-compatible) | `outl_actions::quote::is_quote` / `split_quote` / `toggle_quote` / `QUOTE_PREFIX` | `crates/outl-actions/src/quote.rs` |
| Toggle blockquote on a block in one call | `outl_actions::block::toggle_quote` | `crates/outl-actions/src/block.rs` |

#### 4. Pages and journals (outl-actions::page + journal)

| Intent | Use this | File |
|---|---|---|
| Page-property keys (constants — don't hardcode the strings) | `outl_actions::page::SLUG_KEY` / `KIND_KEY` / `TYPE_KEY` | `crates/outl-actions/src/page.rs` |
| Canonical `type::` value marking a person (filter for `@` mention popup) | `outl_actions::page::PERSON_TYPE` | `crates/outl-actions/src/page.rs` |
| Page metadata (slug, kind, title, **`page_type`**) for a node id | `outl_actions::page::page_meta` / `PageMeta` / `PageKind` | `crates/outl-actions/src/page.rs` |
| Validate a slug for filesystem safety | `outl_actions::page::is_valid_slug` | `crates/outl-actions/src/page.rs` |
| Derive a **deterministic page id** from slug | `outl_actions::page::page_id_from_slug` | `crates/outl-actions/src/page.rs` |
| Find / list / create-if-missing pages | `outl_actions::page::find_by_slug` / `list_all` / `open_or_create` | `crates/outl-actions/src/page.rs` |
| Open-or-create a page from a **human-typed name** (slugifies + keeps original as title) | `outl_actions::page::open_or_create_by_name` | `crates/outl-actions/src/page.rs` |
| Open-or-create whatever a **user-typed ref target** points at (date → journal, `@x` mention → person page, else slug/title match → page, else create) | `outl_actions::page::open_or_create_by_ref` | `crates/outl-actions/src/page.rs` |
| Search pages typed `type:: person`, fuzzy-ranked (powers `@` mention autocomplete) | `outl_actions::page::search_persons` | `crates/outl-actions/src/page.rs` |
| Read / write a property on a page (or any node) | `outl_actions::page::read_text_prop` / `set_property` | `crates/outl-actions/src/page.rs` |
| Migrate pre-page-model blocks under today's journal | `outl_actions::page::migrate_legacy_into_today` | `crates/outl-actions/src/page.rs` |
| Open / create journal for a date or today | `outl_actions::page::open_journal` / `open_today` | `crates/outl-actions/src/page.rs` |
| Journal date utilities | `outl_actions::page::today` / `journal_slug` / `journal_title` / `date_from_slug` / `previous_journal_date` / `next_journal_date` | `crates/outl-actions/src/page.rs` |
| Filesystem paths | `outl_actions::journal::journals_dir` / `pages_dir` / `page_md_path` | `crates/outl-actions/src/journal.rs` |
| Render a page out to `.md` | `outl_actions::journal::render_page_md` | `crates/outl-actions/src/journal.rs` |
| Apply edited `.md` back into the workspace | `outl_actions::journal::apply_page_md` / `apply_page_md_with_sidecar` | `crates/outl-actions/src/journal.rs` |
| Apply every page's `.md` to disk in one pass | `outl_actions::journal::apply_all_pages_md` | `crates/outl-actions/src/journal.rs` |
| Read → modify → write a page's `.md` atomically | `outl_actions::journal::mutate_page_md` | `crates/outl-actions/src/journal.rs` |
| Atomic `.md` write (crash-safe) | `outl_actions::journal::write_md_atomic` | `crates/outl-actions/src/journal.rs` |

#### 5. Parse / render (outl-md::parse + render)

| Intent | Use this | File |
|---|---|---|
| Parse `.md` → outline AST (no IDs) | `outl_md::parse::parse` → `ParsedPage` (includes `warnings: Vec<ParseWarning>`) | `crates/outl-md/src/parse.rs` |
| Render outline AST → `.md` (clean, no IDs) | `outl_md::render::render` | `crates/outl-md/src/render.rs` |
| Non-fatal parser recovery records (heading instead of bullet, etc.) | `outl_md::ParseWarning` + `outl_md::ParseWarningKind` | `crates/outl-md/src/parse.rs` |
| The outline AST node DTO | `outl_md::OutlineNode` / `outl_actions::outline::OutlineNode` | `crates/outl-md/src/parse.rs` + `crates/outl-actions/src/outline.rs` |
| Project the workspace tree into the UI DTO | `outl_actions::outline::project_outline` / `project_outline_node` | `crates/outl-actions/src/outline.rs` |
| Flatten an `OutlineNode` subtree to DFS paths | `outl_actions::outline::flatten_subtree_paths` | `crates/outl-actions/src/outline.rs` |
| Read page from disk + project to outline view | `outl_actions::outline::read_page_view` / `read_page_view_with_workspace` | `crates/outl-actions/src/outline.rs` |
| Read page + parser warnings (for banner / doctor / status line) | `outl_actions::outline::read_page_outline` / `read_page_outline_with_workspace` → `PageOutline { nodes, warnings }` | `crates/outl-actions/src/outline.rs` |

#### 6. External markdown coercion & ingest (outl-actions::paste + ingest)

| Intent | Use this | File |
|---|---|---|
| Coerce **external markdown** (line endings, indent unit 4→2, Roam/GitHub/Logseq tokens, long-form dates → ISO, strip `id::` with Crockford validation, strip unknown `{{…}}` / `^^…^^`) | `outl_actions::paste::normalize_external_syntax` | `crates/outl-actions/src/paste/normalize.rs` |
| "Does this clipboard look like an outline?" classifier | `outl_actions::paste::looks_like_outline` | `crates/outl-actions/src/paste/mod.rs` |
| Convert clipboard markdown → outl ops grafted at a position | `outl_actions::paste::paste_markdown` → `PasteOutcome` / `PasteAnchor` | `crates/outl-actions/src/paste/mod.rs` |
| Ingest a `.md` as a real page (creates page node + reconciles blocks) | `outl_actions::ingest::ingest_md_file` / `ingest_dir` | `crates/outl-actions/src/ingest.rs` |
| Create stub pages for every `[[ref]]` with no file of its own | `outl_actions::ingest::create_missing_ref_pages` | `crates/outl-actions/src/ingest.rs` |

#### 7. Reconcile & matching (outl-md::reconcile + matching + diff)

| Intent | Use this | File |
|---|---|---|
| Reconcile existing `.md` against sidecar | `outl_md::reconcile::reconcile_md` / `reconcile_md_with_page_id` | `crates/outl-md/src/reconcile.rs` |
| Reconcile every `.md` in a directory | `outl_md::reconcile::reconcile_dir` | `crates/outl-md/src/reconcile.rs` |
| Reconcile error / report types | `outl_md::ReconcileError` / `ReconcileReport` | `crates/outl-md/src/reconcile.rs` |
| 3-level matching algorithm | `outl_md::matching::match_blocks` → `Match` / `MatchLevel` | `crates/outl-md/src/matching.rs` |
| Diff AST + AST + sidecar → minimum `Op`s | `outl_md::diff::diff_to_ops` → `DiffPlan` | `crates/outl-md/src/diff.rs` |
| Same diff but also propagates page-level props (`title::`, `type::`, `pinned::`, …) into op log as `Op::SetProp` on the page root — fixes the desktop/mobile vs TUI divergence on fixture/external-editor pages | `outl_md::diff::diff_to_ops_with_page_props` | `crates/outl-md/src/diff.rs` |
| Reconcile-pipeline version stamped on every sidecar — bumping it forces every legacy sidecar through `reconcile_md` once on the next boot (idempotent CRDT) | `outl_md::sidecar::CURRENT_PIPELINE_VERSION` | `crates/outl-md/src/sidecar.rs` |

#### 8. Sidecar (outl-md::sidecar + atomic)

| Intent | Use this | File |
|---|---|---|
| Full sidecar struct + per-block entries | `outl_md::Sidecar` / `SidecarBlock` | `crates/outl-md/src/sidecar.rs` |
| Construct a fresh sidecar for a new page | `outl_md::sidecar::Sidecar::new_for_page(page_id, &file_hash)` | `crates/outl-md/src/sidecar.rs` |
| Read / write sidecar (JSON, version 2, backward-reads v1) | `outl_md::sidecar::read` / `write` | `crates/outl-md/src/sidecar.rs` |
| Sidecar path resolution for a `.md` | `outl_md::sidecar::sidecar_path_for` / `resolve_sidecar_path` | `crates/outl-md/src/sidecar.rs` |
| Derive `((blk-XXXXXX))` ref handle from `NodeId` | `outl_md::sidecar::derive_ref_handle` | `crates/outl-md/src/sidecar.rs` |
| Hash block / file content | `outl_md::sidecar::content_hash` / `file_hash` | `crates/outl-md/src/sidecar.rs` |
| Low-level crash-safe write | `outl_md::atomic::write_atomic` | `crates/outl-md/src/atomic.rs` |

#### 9. In-flight outline AST helpers (outl-md::outline_ops)

Operate on `Vec<OutlineNode>` **before** the tree is rebuilt from the op log.
UI-agnostic; TUI and mobile both consume them.

| Intent | Use this | File |
|---|---|---|
| Flat count / TODO+DONE counts | `outline_ops::flat_count` / `count_todos` | `crates/outl-md/src/outline_ops.rs` |
| Flat index ↔ path / node lookup at path | `outline_ops::path_for_index` / `index_for_path` / `node_at_path` / `node_at_path_mut` | `crates/outl-md/src/outline_ops.rs` |
| Count descendants / grab siblings slice | `outline_ops::descendants_count_at_path` / `siblings_mut` | `crates/outl-md/src/outline_ops.rs` |
| Insert sibling before / after a path | `outline_ops::insert_sibling_before` / `insert_sibling_after` | `crates/outl-md/src/outline_ops.rs` |
| Indent / outdent / delete / move up / move down at path | `outline_ops::indent_at_path` / `outdent_at_path` / `delete_at_path` / `move_up_at_path` / `move_down_at_path` | `crates/outl-md/src/outline_ops.rs` |

#### 10. Indices and search (outl-md::index + block_index)

| Intent | Use this | File |
|---|---|---|
| Build / query workspace-wide index | `outl_md::WorkspaceIndex::build` / `by_slug` / `by_title` / `pages` / `pages_by_title_prefix` | `crates/outl-md/src/index.rs` |
| Patch / remove a page in an existing index | `WorkspaceIndex::patch_page` / `remove_page` | `crates/outl-md/src/index.rs` |
| Resolve `((blk-XXXXXX))` / lookup block by id or location | `WorkspaceIndex::resolve_block_ref` / `block_by_id` / `block_at_location` | `crates/outl-md/src/index.rs` |
| Reverse refs / iterate / search | `WorkspaceIndex::block_refs_to` / `iter_blocks` / `search_block_text` / `block_count` | `crates/outl-md/src/index.rs` |
| Stand-alone block-level index | `outl_md::BlockIndex` + `BlockEntry` + `BlockReference` | `crates/outl-md/src/block_index.rs` |
| `PageEntry` DTO | `outl_md::PageEntry` | `crates/outl-md/src/index.rs` |

#### 11. View helpers for editors (outl-md::view + inline)

| Intent | Use this | File |
|---|---|---|
| Char ↔ (line, col) on a buffer | `outl_md::view::char_to_line_col` / `line_col_to_char` | `crates/outl-md/src/view.rs` |
| Project a block to renderable rows | `outl_md::view::block_to_rows` → `BlockRow` / `BlockRowKind` | `crates/outl-md/src/view.rs` |
| Tokenize inline markdown | `outl_md::inline::tokenize` → `InlineTok` | `crates/outl-md/src/inline.rs` |
| Tokenize inline markdown into an owned, Serde-friendly form for wire/DTO payloads | `outl_md::inline::tokenize_owned` → `InlineToken` | `crates/outl-md/src/inline.rs` |
| Reconstruct the source markdown from a `Vec<InlineTok>` (Bold/Italic/Strike carry recursively-tokenized inners) | `outl_md::inline::inline_to_source` | `crates/outl-md/src/inline.rs` |
| Resolve the ref under a caret position | `outl_md::inline::ref_at_cursor` → `RefTarget` | `crates/outl-md/src/inline.rs` |
| Validate a `((blk-XXXXXX))` handle | `outl_md::inline::is_valid_block_handle` | `crates/outl-md/src/inline.rs` |
| Canonicalize a fence info-string (`rs` → `rust`, etc.) — used by `outl-exec` runtime dispatch + frontend syntax highlighter | `outl_md::lang::canonical`, `outl_md::lang::KNOWN_ALIASES` | `crates/outl-md/src/lang.rs` |
| Resolve a `:shortcode:` to its unicode glyph (one-way; multiple shortcodes can alias the same codepoint) | `outl_md::emoji::shortcode_to_unicode` | `crates/outl-md/src/emoji.rs` |
| Validate the `[a-z0-9_+-]+` shape of an emoji shortcode (catalog check is separate) | `outl_md::emoji::is_valid_shortcode` | `crates/outl-md/src/emoji.rs` |
| Validate one char of a shortcode (`[a-z0-9_+-]`) — char-by-char walks (TUI's `detect_trigger`, `try_emoji`) skip allocating a 1-char `String` per keystroke | `outl_md::emoji::is_valid_shortcode_char` | `crates/outl-md/src/emoji.rs` |
| Search the GitHub gemoji catalog for shortcodes matching a query (powers `:emoji:` autocomplete across TUI / mobile / desktop) | `outl_md::emoji::search` → `EmojiHit` | `crates/outl-md/src/emoji.rs` |
| Byte offset for a char index (UTF-8 safe) | `outl_md::inline::byte_index_for_char` | `crates/outl-md/src/inline.rs` |

#### 12. Backlinks (outl-actions::backlinks)

| Intent | Use this | File |
|---|---|---|
| Extract `[[ref]]` tokens out of a block's text | `outl_actions::backlinks::extract_refs` | `crates/outl-actions/src/backlinks.rs` |
| Backlink DTO | `outl_actions::backlinks::Backlink` | `crates/outl-actions/src/backlinks.rs` |
| Walk backlinks for a target / page (matches `[[ref]]` literally and `#tag` via slugify) | `outl_actions::backlinks::backlinks_for_target` / `backlinks_for_page` | `crates/outl-actions/src/backlinks.rs` |

#### 13. Code-block execution (outl-actions::exec)

Cross-client glue every UI uses to wire a "run this fence" gesture (TUI `g x`, desktop `Cmd+X`, mobile long-press) into `outl-exec` and back. `outl_actions::exec::run_code_block` is the **only** entry point a Tauri command / TUI action should call — never re-implement flat-DFS / `.md` path / DTO per client.

| Intent | Use this | File |
|---|---|---|
| Flat DFS index of a `NodeId` inside an outline forest | `outl_actions::flat_index_for_block` | `crates/outl-actions/src/outline.rs` |
| Orchestrate exec (walk → path → `outl_exec::run_block_at_index` → DTO) | `outl_actions::exec::run_code_block` | `crates/outl-actions/src/exec.rs` |
| Serializable mirror of `outl_exec::ExecOutput` | `outl_actions::ExecOutputDto` | `crates/outl-actions/src/exec.rs` |
| Outcome shipped to client (`language` + `result_ok` xor `error`) | `outl_actions::RunCodeBlockOutcome` | `crates/outl-actions/src/exec.rs` |

Runtime selection (which languages ship) is per-binary via `outl-exec` features in the consumer's `Cargo.toml`. `outl-actions` pulls `outl-exec` with `default-features = false` so it never drags `wasmtime` into the mobile IPA.

#### 14. Sync engine, locks, storage trait

| Intent | Use this | File |
|---|---|---|
| Shared sync entry point (TUI poller + mobile iCloud watcher) | `outl_actions::SyncEngine::new` | `crates/outl-actions/src/sync.rs` |
| Reload workspace from disk after peer change | `SyncEngine::reload_workspace` | `crates/outl-actions/src/sync.rs` |
| Re-project a page's `.md` + sidecar / reload + reproject in one call | `SyncEngine::reproject_page` / `refresh_page` | `crates/outl-actions/src/sync.rs` |
| Snapshot every / peer-only `ops-*.jsonl` | `SyncEngine::snapshot` / `snapshot_peers` (`OpsFileSnapshot`) | `crates/outl-actions/src/sync.rs` |
| Scan for orphan `.md` (no sidecar / stale hash) | `SyncEngine::scan_for_orphans` | `crates/outl-actions/src/sync.rs` |
| Cross-process workspace lock | `outl_core::WorkspaceLock::acquire` | `crates/outl-core/src/lock.rs` |
| Per-actor write lock | `outl_core::ActorWriteLock::try_acquire` | `crates/outl-core/src/lock.rs` |
| Resolve which actor this process writes as | `outl_core::resolve_write_actor` | `crates/outl-core/src/lock.rs` |
| The `Storage` trait every backend implements (invariant #5) | `outl_core::Storage` / `StorageError` | `crates/outl-core/src/storage/mod.rs` |

**Review checklist on every PR that adds a helper:**

- Does the new function name / signature describe something already in the catalog above?
  If yes → blocker, point at the existing one.
- Does the PR add a `normalize`, `coerce`, `strip`, `slugify`, `hash`, `derive`, or `extract` helper without grepping the catalog first?
  Ask: "did you check `<catalog entry>` before writing this?"
- Does the new code create a page / write `.md` / mint a `NodeId` / build a `LogOp` outside the catalog primitives?
  Block — that's how invariants drift.
- Does the PR add a new `pub fn|struct|enum|const` in `crates/outl-{core,md,actions}/src/`?
  The new symbol **must** appear in the catalog (a local `catalog-drift-guard.sh` hook enforces this pre-merge; the same rule applies in review).

### 5.2 Reuse-first violations — no parallel implementations

Duplication here is a real hazard: two implementations of the same logic drift apart over time, and the user is the one who hits the divergence.

**Past incidents to anchor severity:**

- `outl_md::index::Backlink` and `outl_actions::Backlink` were two parallel "backlinks" pipelines that started identical and ended up disagreeing on self-references — caught by the user, not the reviewer.
  Collapsed into `outl_actions::backlinks_for_page` in 0.5.3.
- PR #47 (Logseq import) opened with `crates/outl-cli/src/cmd/import/normalize.rs` reimplementing `\r\n` handling, `id::` stripping, and long-form date rewriting — every one of which `outl_actions::paste::normalize_external_syntax` already owned.
  Caught in review *after* a Claude-assisted PR shipped without the catalog being visible.
  That's why §5.1 exists.

The rule the PR author was expected to follow:

1. **Grep before writing.** `rg "fn foo"` / `rg "struct Foo"` across `crates/`.
   Look in **upstream crates first**, in this order: `outl-core` → `outl-md` → `outl-actions`.
   These are where shared primitives live.
   The catalog above is your starting point.
2. **Prefer evolving the existing API** over duplicating, even if that means a small refactor (rename, generalize a parameter, move into a sibling module).
   One owner per concept; many callers.
3. **Refactor *into* the shared crate, not *around* it.** If a TUI helper feels like it could live in `outl-actions`, the PR should move it there *now* — the mobile client will need it soon.
   The `flatten_subtree_paths` migration is the canonical pattern.
4. **Duplication is OK only when the platforms are genuinely different.** `outl-tui::EditBuffer` and the mobile `<textarea>` are both "cursor + text", but one is a terminal widget Rust has to render itself and the other is a browser primitive.
   Same role, different runtime — not duplication.
   **Recalculating** `(line, col)` from `cursor` in both places, though, would be — extract to `outl_md::view::char_to_line_col` and wrap.

When you spot a duplicate, point at the existing function with `file:line` and ask: "can you call this instead, or extend it if it doesn't quite fit?
The fix is to wrap or evolve the upstream API, **never** to write a parallel one.
If the author argues for duplication, they have to fit it into case 4 above — same role, genuinely different runtime.
Anything else is a blocker.
- **Layering violations.** UI imports in `outl-core`.
  Client crates building op trees instead of calling `outl-actions`.
  Workspace mutations done outside `Workspace::apply`.
- **New `Op` variant without the full checklist.** Adding a variant touches `apply_op`, `undo_op` (the inverse must be exact), the sidecar serializer, the markdown projection, the replay tests, and the per-crate docs.
  Check the diff against `/new-op` expectations and call out anything missing.
- **Trait surface that locks out a future backend.** `Storage` must stay implementable by ChronDB later.
  If a new method assumes file semantics (paths, flock), question it.
- **Sidecar / op-log format changes without a migration story.** Existing workspaces on disk must still load.
  Either the change is backward-compatible (new optional field) or there is a versioned migration path described in the PR.
- **File size growth past 600 lines.** Note it, suggest a split by responsibility, point at `refactor-architect` agent.
  Past 900 lines, request a refactor before merge.
- **Premature abstraction.** A new trait or generic with one impl and no second use case in sight.
  The Rule of Three applies — concrete first, abstract on the third caller.

### 5.3 Documentation drift — block PRs that change behavior without updating the dev/contrib docs

`docs/development.md` (engineer onramp) and `docs/contributing.md` (review policy) are the two pages a new contributor reads before opening their first PR.
A stale onramp is **worse than no onramp** because it sends contributors confidently into a wall — they follow steps that no longer work and silently distrust the project the rest of the way.

**Use this table to decide when the PR must update docs.** If you see a diff in the left column and no matching update in the right column, request the doc change before approving.

| If the PR touches... | Require an update to |
|---|---|
| `.github/workflows/ci.yml` (jobs, matrix, excluded crates, `RUSTDOCFLAGS`, paths-ignore) | `docs/development.md` § 9 (CI walkthrough) |
| `.github/workflows/release.yml`, `mobile.yml`, `desktop.yml`, `testflight.yml`, `bench.yml`, `cleanup-tags.yml` | `docs/development.md` § 9 (CI table) and § 10 (Release process) |
| `.claude/settings.json` hooks, `.claude/agents/*.md`, `.claude/commands/*.md` (any slash command or hook behavior) | `docs/development.md` § 4 (Dev loop) |
| `rust-toolchain.toml` version bump | `docs/development.md` § 1 and root `CONTRIBUTING.md` |
| System deps for a crate (Tauri, GTK, Bun, Xcode, hyperfine, etc.) | `docs/development.md` § 1 ("Optional toolchains by area") |
| New crate added to `crates/` | `docs/development.md` § 2, root `CLAUDE.md` repo layout, per-crate `CLAUDE.md` |
| New native iOS surface (file added to `crates/outl-mobile/swift/OutlKit/Sources/`, `crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`, or `main.mm`) | `docs/development.md` § 3 (the "Why the mobile crate has native Swift / ObjC code" table — does the new surface fit an existing row or is it a new reason?) + § 6 cookbook + `crates/outl-mobile/CLAUDE.md` |
| New `Op` variant, sidecar field, or op-log format change | `docs/development.md` § 6 cookbook + `docs/crdt.md` + `outl-md/CLAUDE.md` |
| `/check` / `/check-invariants` / `/roundtrip` / `/coverage` / `/new-op` / `/init-playground` semantics | `docs/development.md` § 4 (slash command table) |
| Benchmark layout (new bench file, new size tier, hyperfine recipe) | `docs/development.md` § 8 (Performance) |
| Version source-of-truth or release tooling (e.g. someone re-adds `version` to `tauri.conf.json`) | `docs/development.md` § 10 + `crates/outl-mobile/CLAUDE.md` (and reject re-adding the `version` field — it's an invariant) |
| Conventional Commits enforcement or release-notes pipeline | `docs/development.md` § 10 + root `CLAUDE.md` "Coding conventions" |
| Storage trait surface, `JsonlStorage` / `MemoryStorage` test contract | `docs/development.md` § 5 + `docs/storage.md` + `outl-core/CLAUDE.md` |
| New `Action` variant in `outl-shortcuts` / new keybinding / chord rebound | `docs/shortcuts.md` (the row that ships to users) + `outl-shortcuts/src/{action.rs,defaults.rs}` + every client's dispatcher (`outl-tui/src/input/*.rs`, `outl-desktop/src/lib/{shortcuts.ts,action-handlers.ts}`) + `outl-desktop/src/lib/api.ts` (TS mirror of the `Action` union — no codegen, drift here is silent until runtime) |
| Public API of a shared primitive listed in `CLAUDE.md` § 5.1 catalog | The same catalog row, **plus** mirror the change in this file's § 5.1 |

Phrase the comment so the author knows exactly which file and section to move.
"Doc looks stale" is noise; "section 9 of `docs/development.md` still says `ci.yml` runs on the workspace including `outl-mobile` — this PR removes that exclusion" is review.

---

## 6. Simplicity — fewer moving parts wins

Push back on:

- A new dependency for a feature that is two functions of standard library code away.
  Compare crate size, maintenance status, transitive deps, and licence before accepting.
- A configuration knob with no concrete user asking for it.
  Defaults that are right for the 90% case beat knobs that nobody tunes.
- Cleverness over readability.
  If a reviewer must run the code in their head to understand it, the next maintainer will lose more time than the original author saved.
- A trait, builder, or macro added for "future flexibility" with no named future caller.

---

## 7. Testing bar

- **Bug fix without a regression test → blocker.** The test must fail on `main` and pass with the patch.
  Ask for it explicitly.
- **Critical path touched without coverage proof.** `outl_core::tree::{do_op, undo_op, apply_op, creates_cycle}` and `outl_md::reconcile_md` carry 100% line and branch coverage rules.
  New branches need new tests.
  Ask the author to run `/coverage outl-core` (or the relevant crate) and paste the result.
- **Test asserts implementation, not behaviour.** A test that breaks on any refactor is a maintenance tax.
  Suggest asserting against the public surface (op log contents, materialized tree shape, rendered markdown), not internal helpers.
- **Mocked storage in an integration test that should hit `JsonlStorage`.** Real-file integration is cheap; mocks hide the bugs that matter.
- **`#[ignore]` or `#[should_panic]` added without a comment** explaining the invariant being protected.

---

## 8. What NOT to comment on

These produce noise.
Stay silent:

- Anything `cargo fmt`, `cargo clippy -D warnings`, or rustdoc warnings already enforce.
- Style preferences (`if let Some(x) = y` vs `match y { Some(x) => ... }`) with no behavioural difference.
- "I would have named this differently" without a concrete clarity win.
- Speculation about a future architecture that nobody asked for.
- Re-litigating the "decisions you don't get to revisit" table in `CONTRIBUTING.md` and root `CLAUDE.md`.
- Adding TODOs the author already acknowledged in the PR description's "Out of scope" section.
- Disagreements with documented patterns in `CLAUDE.md` — defer to the file, do not argue with it inline.

---

## 9. How to format your review

Group findings by severity.
Lead each finding with the file and line.

```
🔴 Blocker   — violates an invariant or guarantees a regression.
🟡 Should-fix — concrete problem with a clear fix, but not a blocker.
🔵 Consider  — design or perf observation worth a reply, not a change request.
```

Each finding follows this shape:

```
**`crates/outl-core/src/tree.rs:184`** — 🔴 Blocker
Calling `apply_op` directly here bypasses the log append, so the
mutation will not replay on a second device. Route through
`Workspace::apply` instead; see the existing call at
`crates/outl-actions/src/block.rs:73`.
```

End the review with one of these two closing lines:

- **If the PR description gate passed and the diff is mergeable as-is or with should-fixes only:**
  > LGTM once the should-fix items are addressed.
  > No blockers.

- **If there is a blocker:**
  > Blocked: <one-line summary>.
  > Resolve the 🔴 items above before the next round.

- **If the gate failed (no issue / weak description):**
  > Not reviewed in detail — the PR needs an issue link or a problem statement first (see top comment).

Keep the whole review under ~400 words unless the diff is genuinely large.
A long review is a sign you are commenting on too much.

---

## 10. Out of scope right now

The project is in Phase 0–1.
Do **not** suggest work on:

- P2P sync transport (`iroh`) — iCloud is the v0 transport.
- Query DSL (`{{query: ...}}`).
- Tauri desktop shells beyond the existing mobile crate.
- Plugin system (`rhai`).
- `ChronDbStorage` backend (tracked as issue #1).
- Android mobile build (iOS only today).
- Per-page op log shards (only when the workspace hits 10k pages).

If the PR touches one of these, it should already be linked to its tracking issue.
