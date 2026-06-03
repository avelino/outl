# Changelog

All notable changes to outl are documented here. Format inspired by
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
uses [Semantic Versioning](https://semver.org/).

## [0.5.3] — 2026-06-02

**Unify backlinks, Insert-mode cross-block nav, anti-duplication policy.**

Two parallel backlinks pipelines (one on `outl-md::index`, one on
`outl-actions`) had drifted on policy — self-references were dropped
on the TUI panel but kept on mobile, and the user had to spot the
divergence by comparing surfaces. 0.5.3 collapses them into one path
through `outl_actions::backlinks_for_page`, deletes the cache on
`outl-md::index`, and renames the related helpers so the call sites
land on the shared API by default.

Insert mode also picks up the missing piece for vim/emacs muscle
memory: `Up`/`Down` cross blocks (commit, move selection, re-enter
Insert preserving the cursor column) the same way `Left`/`Right`
already did. Multi-line buffers (fenced code) absorb the move
internally first.

### Added

- **`outl_core::Tree::properties_of(node)`** — iterator over every
  property currently set on a node, in one pass. Used by the outline
  DTO so each `OutlineNode` carries its own properties without
  scanning the workspace-wide map per block.
- **`outl_md::view::line_col_to_char(s, line, col)`** — inverse of
  the existing `char_to_line_col`. Vim-style column clamping (past
  EOL → end of line) and line clamping (past last → end of string).
  Lets `outl_tui::EditBuffer::move_up` / `move_down` wrap the same
  primitives the renderer (`block_to_rows`) already uses.
- **`outl_tui::EditBuffer::move_up` / `move_down` / `visual_column`** —
  three thin wrappers over `outl_md::view::char_to_line_col` +
  `line_col_to_char`. Cross-block Up/Down in Insert calls these
  first; only spills into the next block when the cursor was already
  on the buffer's first/last line.
- **`outl_actions::project_outline_node(workspace, node)`** — build
  a single `OutlineNode` (subtree + properties) from the workspace.
  Used by the backlinks builder so each backlink carries its source
  block as a self-contained outline.
- **`outl_actions::flatten_subtree_paths(node)`** — DFS-ordered paths
  inside an `OutlineNode` subtree. Moved here from
  `outl_md::outline_ops` so any client that consumes
  `Backlink::source_block` can navigate it.
- **`outl_actions::OutlineNode.properties`** — `(key, value)` pairs
  in alphabetical order. Workspace and disk paths both normalise to
  the same order so backlink panels and outline pages never disagree
  visually.
- **`outl_actions::PageMeta.icon`** — page-level `icon::` property
  surfaced on the meta. Clients pick their own fallback (mobile uses
  `📄`/`📅` by `kind`; TUI uses `📄`).

### Changed

- **Backlinks now route through `outl_actions::backlinks_for_page`
  only.** `outl_md::index::Backlink`, `WorkspaceIndex.backlinks()`,
  `refresh_backlinks_from_source`, `patch_backlink_text`,
  `flatten_backlink_subtree` were deleted. The `outl-md` index still
  owns page metadata and the block-level index; only the parallel
  backlinks cache went away.
- **`outl_actions::Backlink` is the rich struct.** Now carries
  `source_block: OutlineNode` (subtree + properties),
  `source_block_path: Vec<usize>`, `source_path: Option<PathBuf>`
  alongside `block_id`, `block_text` (TODO/DONE prefix stripped),
  `todo`, `source_page`. Mobile renders just `block_text` + `todo`
  today and ignores the rest; the TUI uses the full subtree to
  render its mini-outline in the backlinks panel.
- **`outl_actions::backlinks_for_page(workspace, root, meta)` /
  `backlinks_for_target(workspace, root, target)`** now take the
  workspace root so each backlink can carry its source `.md` path.
  CLI passes `&ctx.root`, TUI passes `&self.workspace_root`, mobile
  passes `storage_root`.
- **TUI cross-block Up/Down in Insert.** Commits the current buffer,
  moves the outline selection, re-enters Insert with the cursor on
  the preserved column. Guard: when `move_selection` would land
  `Focus` on the backlinks panel, the TUI stops in Normal mode
  instead of opening a different page mid-Insert. Backlink edits
  keep the older Esc → j/k → i workflow until cross-page commits
  get their own pass.
- **`App::backlinks_for_current` is cached.** Per-frame and
  per-keystroke render calls hit a `RefCell<Option<(slug, Vec)>>`
  cache; invalidated on `save`, `save_page_with`,
  `reload_workspace_from_disk`, and any view switch. Cuts the
  workspace scan from `O(blocks)` per call to one per slug change.
- **Self-references are kept in backlinks.** The "skip
  self-references as noise" heuristic on `outl_md::index` was
  dropped — a block on today's journal that mentions
  `[[2026-06-02]]` is exactly the "linked from" pin the user expects
  to see when revisiting that day.

### Refactored

- **`crates/outl-core/src/tree.rs` (854 lines) →
  `crates/outl-core/src/tree/{mod, cycle, op, apply}`** —
  `Tree::creates_cycle` in `cycle.rs`, `Tree::do_op` +
  `Tree::undo_op` in `op.rs`, `Tree::apply_op` in `apply.rs`. Struct
  and accessors stay in `mod.rs`. The 11 inline CRDT tests moved to
  `crates/outl-core/tests/tree_unit.rs`. **Algorithm semantics
  unchanged** — verified line-by-line against Kleppmann et al. 2022
  and against the full invariant battery (convergence, cycle,
  cycle_chain, concurrent_edit_move, concurrent_delete_edit,
  late_op, idempotency, fractional_index, property_based,
  large_log: 32/32 green).
- **`crates/outl-tui/src/input.rs` (835 lines) →
  `crates/outl-tui/src/input/{mod, normal, insert, overlay, visual}`** —
  one handler per file, shared helpers (`cross_block_step`,
  `cursor_inside_open_fence`, `cross_block_nav_eligible`) stay in
  `mod.rs`.
- **`crates/outl-tui/src/actions/block.rs` (843 lines) →
  `crates/outl-tui/src/actions/block/{mod, insert, structural,
  backlink_edit, metadata}`** — Insert mode in `insert.rs`,
  create/indent/outdent/delete/move in `structural.rs`, cross-page
  backlink ops in `backlink_edit.rs`, properties + TODO toggle +
  pin in `metadata.rs`. TODO-prefix cycle helpers shared via
  `mod.rs`.
- **`crates/outl-tui/src/actions/lifecycle.rs` (669 lines) →
  `crates/outl-tui/src/actions/lifecycle/{mod, index_build,
  peer_sync, external, loading, persistence}`** — `App::new` and
  the shared `file_mtime` helper in `mod.rs`. Each submodule owns
  one concern.

No public API changed during the splits. Clients (mobile, CLI,
external consumers) need no update.

### Documentation

- **Anti-duplication policy** added to the root `CLAUDE.md` and
  echoed in every per-crate `CLAUDE.md`. Captures the lesson
  surfaced by the parallel `Backlink` structs and the near-miss with
  `line_start_and_column` (almost re-derived inside `EditBuffer`
  before the inverse `line_col_to_char` landed in `outl-md::view`).
  Rule: grep upstream first, prefer evolving the existing API over
  cloning the math.

### Internal

- `outl_md::Backlink`, `WorkspaceIndex.backlinks`,
  `refresh_backlinks_from_source`, `patch_backlink_text`,
  `flatten_backlink_subtree`, `outl_md::index::Backlink` removed.
- `outl_md::view` gained the `line_col_to_char` inverse.
- `outl_core::Tree.{nodes, properties, collapsed}` are now
  `pub(super)` so the split submodules can reach them. Public API
  unchanged.

## [0.5.1] — 2026-06-01

**Fix: multi-process writes against the same workspace.**

0.5.0 inherited an exclusive `flock` on `<root>/.outl/.lock` from
the SQLite era. The lock made sense when two writers on a single
`log.db` would race, but JSONL stores one file per actor — the
exclusive scope just blocked every legitimate co-tenant: TUI + MCP
server, MCP server + `sink-outl` plugin, two CLI calls in flight.
Symptom: `INVALID_ARG: workspace ... is locked by another outl
process` from the second opener, while the first ran fine and held
the lock for its whole session.

0.5.1 splits coordination into two locks. **Concurrent TUI + MCP
server + CLI subprocess is the supported case** from here on.

### Added

- **`outl_core::WorkspaceLock` is now shared** (`LOCK_SH`). Every
  well-behaved `outl` process piles on. The lock still surfaces a
  hard filesystem error when `flock` itself fails, but never
  rejects a legitimate second opener.
- **`outl_core::ActorWriteLock`** — exclusive `flock` on
  `<root>/ops/.lock-<actor>`. Held by exactly one process per
  actor id at a time. This is the new write-coordination boundary.
- **`outl_core::resolve_write_actor(ops_dir, config_actor)`** —
  helper used by every workspace opener. Tries `config_actor`
  first; on `AlreadyHeld`, generates `ActorId::new()` and locks the
  ephemeral one instead. Returns the lock + actor id pair.
- **`WsCtx.ephemeral_actor: bool`** flag on the CLI/MCP context so
  `outl doctor` / `outl workspace info` can show when a process is
  writing under an ephemeral actor.

### Changed

- **`outl-cli::ws::open`** acquires the shared workspace lock plus
  a per-actor write lock through `resolve_write_actor`. On `outl`
  invocations that land while a server/TUI already holds the
  config actor, this process spins a fresh `ops-<ephemeral>.jsonl`
  and writes there. Readers merge every `ops-*.jsonl` in `ops/`,
  so peers see the full op log.
- **`outl-tui::open_workspace`** follows the same flow. The TUI
  used to refuse to launch when an MCP server was running against
  the same workspace; it now coexists.

### Why the ephemeral-actor fallback is safe

Every actor is independent at the CRDT layer (it's literally the
mechanism multi-device sync relies on). Two processes on the same
device using two different actors merge the same way two devices
would: the readers replay every `ops-<actor>.jsonl` in HLC order,
the tree converges. The only cost is `ops/` accumulating one
jsonl per ephemeral lifetime — typically tiny files (a session's
writes), and a future `outl gc` can consolidate them per device.

### Migration

None. 0.5.0 workspaces work as-is. The next time you open a
workspace with a second `outl` process, it will silently mint an
ephemeral actor; the first process keeps writing under
`config.toml[workspace].actor_id` as before.

## [0.5.0] — 2026-06-01

**Breaking: SQLite is gone. JSONL is the only persistent storage.**

0.4.x kept two storage backends side by side — `SqliteStorage` for
local-only workspaces and `JsonlStorage` for shared/synced ones. The
result was a class of "writes go through but disappear when you open
the other client" bugs: any code path that opened a workspace via
`outl-cli` got SQLite, while `outl-tui` and mobile (Tauri) followed
`config.toml[workspace].storage` and got JSONL. Same workspace,
divergent op logs, silent loss.

0.5.0 collapses the surface: every client opens the workspace as
`JsonlStorage` rooted at `<root>/ops/`. There is no flag to choose,
no `[workspace].storage` knob with two valid values, no SQLite
fallback. The `Storage` trait stays in place for future backends
(ChronDB on the roadmap); the only impl that ships is JSONL plus
the in-memory test double.

### Migration from 0.4.x

If your workspace was created with 0.4.x and you have data in
`<root>/.outl/log.db`, the migration is a strict three-step
sequence. 0.5.x cannot read SQLite and 0.4.1 is the last release
that shipped `outl migrate-to-shared` (which this PR removed):

```bash
# 1. Pin 0.4.1 (last release with migrate-to-shared)
cargo install outl-cli --version 0.4.1 --locked

# 2. Run the one-shot migration (idempotent, leaves log.db intact)
outl migrate-to-shared <workspace>

# 3. Confirm ops/ops-<actor>.jsonl grew, then upgrade
cargo install outl-cli --version 0.5.1 --locked

# 4. Once you've verified peers see your data, delete log.db yourself
rm <workspace>/.outl/log.db <workspace>/.outl/log.db-shm <workspace>/.outl/log.db-wal
```

If you already had a mixed `log.db + ops/` workspace under 0.4.x,
step 2 is still required — `migrate-to-shared` is idempotent (HLC
dedup) and any ops that only ever made it into SQLite move over
on this run. After step 3, 0.5.x ignores `log.db` entirely.

### Removed

- **`SqliteStorage`** in `outl-core::storage`. Callers use
  `JsonlStorage` (persistent, per-actor JSONL) or `MemoryStorage`
  (the new in-memory test double, replaces `SqliteStorage::open_in_memory`).
- **`rusqlite` dependency.** Workspace `Cargo.toml` no longer pulls
  the SQLite C bundle. Faster builds, smaller binaries.
- **`outl migrate-to-shared`** subcommand. It only made sense while
  both backends coexisted; with only one backend the migration is
  a one-shot done on 0.4.1 before upgrading.
- **`config.toml[workspace].storage`** field is silently ignored
  now (kept readable so old configs don't error). Cleaning it up
  is fine but not required.

### Changed

- **`Paths` struct (`outl-cli/src/workspace_layout.rs`)** drops the
  `db: PathBuf` field, gains `ops: PathBuf` pointing at
  `<root>/ops/`. Every caller that touched `.outl/log.db` now
  targets the JSONL directory.
- **`outl init`** scaffolds `<root>/ops/` and opens `JsonlStorage`
  to materialize the per-actor `ops-<actor>.jsonl` file. The human
  output now reports `ops:` instead of `log:`.
- **`outl doctor`** drops the SQLite `PRAGMA integrity_check`
  finding and replaces it with a JSONL parse-and-load check
  (`JsonlStorage::open` surfaces every unreadable line via
  `tracing::warn!`, then the report carries the op count and the
  set of known node ids the sidecar cross-check needs).
- **`outl workspace info --json`** renames the `log_db` field to
  `ops_dir`. Stable-envelope shape otherwise unchanged.
- **`outl-tui::open_storage`** is now a one-liner. The config-driven
  match disappears; storage is always JSONL.
- **`Workspace::open_in_memory`** is unchanged in signature but uses
  the new `MemoryStorage` under the hood. No filesystem touch.

### Internal

- New `MemoryStorage` in `outl-core::storage::memory`. Pure
  `Vec<LogOp>` + snapshot slot, no I/O. Used by every test that
  previously called `SqliteStorage::open_in_memory()` and by
  `Workspace::open_in_memory`.

## [0.4.1] — 2026-06-01

Batch authoring for agents and scripts. The 0.4.0 CLI / MCP surface
covered every primitive write, but creating a structured page meant
chaining N tool calls — one per block — which costs round-trips,
turn budget on the agent, and time. 0.4.1 collapses that into the
three composite shapes an agent or import pipeline actually wants:
write a subtree, create a page with content, and stream a sequence
of writes in one workspace session.

No storage or op-log format changes — every new tool is shimmed over
the existing `outl-actions` primitives (`append_block`, `edit_text`,
`set_property`). Drop-in upgrade from 0.4.0.

### Added — composite write surface

- **`outl_block_append_tree` / `outl block append-tree`.** Append a
  root block plus its recursive children under a page or block in a
  single op-log session. Input shape:
  `{"text": "...", "children": [{"text": "...", "children": [...]}]}`.
  Response mirrors the input with `id` at every node so the caller
  can map specs back to freshly minted ids. CLI accepts the JSON
  inline (`--tree '{...}'`) or via stdin (`--tree -`).
- **`outl page create --content` / `outl_page_create` with
  `content`.** A new page lands with its full outline forest in one
  call instead of `page_create` + N × `block_append`. Accepts
  either a single root (`{text, children?}`) or a forest
  (`[{...}, {...}]`); the returned `content` array carries the
  block ids. Skipping the field keeps the historical empty-page
  behaviour.
- **`outl batch` / `outl_batch`.** Apply a list of writes
  sequentially in one workspace session. Supported `op` names:
  `page_create`, `page_update`, `page_delete`, `page_rename`,
  `block_append`, `block_append_tree`, `block_insert`,
  `block_update`, `block_move`, `block_delete`,
  `block_toggle_todo`, `daily_append`, `page_prop_set`. Each op's
  `args` mirror the matching standalone tool. **Stop-on-first-error
  semantics:** earlier ops stay in the op log (they're already CRDT
  ops; we don't roll them back), and the response carries
  `failed_at` / `failed_op` / `error` so the caller can recover or
  retry only the suffix that never ran. CLI exit code is `1` on
  partial failure.

### Added — `outl-actions::block`

- **`append_tree`, `append_forest`.** UI-agnostic primitives behind
  the new composite tools. `BlockTreeSpec` + `BlockTreeOutcome` are
  the shared DTOs (serde Serialize / Deserialize) so both client
  layers and future plugins can compose subtrees without re-deriving
  the recursion.

### Added — bench

- **`bench-cli-xlarge` workflow job.** Weekly + dispatch only.
  Generates a 10k-page batch payload via the new `xtask gen-10k`
  binary, applies it through `outl batch` end-to-end (subprocess +
  workspace lock + op log + sqlite + sidecar + md write), then runs
  `hyperfine` on `page list`, `search`, `query --tag`, `page get`,
  and `page render` against the populated workspace. Catches
  regressions in the surface that wraps the algorithm — the existing
  `bench-xlarge` job stays focused on the algorithm itself via
  criterion micro-benches.
- **`xtask` workspace member.** Internal task runner; today ships
  `gen-10k` (deterministic batch-payload generator) and is where any
  future codegen / fixture / bench helper lands.

### Docs

- `docs/cli.md` — new **Batch** section with the payload shape and
  failure semantics; `page create --content` and
  `block append-tree` documented inline next to the existing primitives.
- `docs/mcp.md` — multi-block authoring callout pointing at the
  three new composite tools.

## [0.4.0] — 2026-06-01

outl becomes scriptable. A full machine-shaped CLI (page, block,
daily, search, query, tag, prop, backlinks, export, workspace) lands
with a stable JSON envelope and exit codes, and the same handlers are
exposed over MCP via `outl mcp serve` (JSON-RPC over stdio) so Claude
Desktop, Cursor, and any other agentic client can drive a workspace
without parsing TUI output. Business logic stays in `outl-actions`;
the CLI and MCP are thin shims over the same code.

No storage or op-log format changes — drop-in upgrade from 0.3.x for
data on disk. **One breaking flag rename** for shell/cron users:
`--path` is now `--workspace` everywhere.

### CLI (`outl-cli`) — new machine surface

- **Subcommands cover the full workspace API.** `outl page
  {list,get,create,rename,delete,prop}`, `outl block
  {get,edit,create,delete,move,toggle}`, `outl daily
  {today,get,range}`, `outl search`, `outl query`, `outl tag
  {list,page}`, `outl prop {list,page}`, `outl backlinks
  {page,block,embed}`, `outl export hugo`, `outl workspace
  {info,doctor}`. Every command writes a stable JSON envelope
  (`{ok, data, error, meta}`) to stdout and a typed exit code, so
  scripts and CI never have to scrape human output. `--human` keeps
  the friendly table format for interactive use.
- **One Workspace per process, index cached.** Each invocation opens
  the workspace once, reuses the in-memory index, and drops the
  per-call SQLite replay that older `outl serve`-style flows paid.
- **`--workspace` replaces `--path`.** The TUI, server, doctor, and
  every new subcommand now take `--workspace <dir>`. Existing
  scripts that pass `--path` must rename the flag (env var stays
  `OUTL_WORKSPACE`). The TUI's positional path argument is
  unchanged for direct double-clicks.
- **CLI integration suite** (`cli_machine.rs`) exercises page,
  block, search, and workspace commands against a real workspace so
  envelope shape and exit codes can't drift.

### MCP server (new: `outl mcp serve`)

- **JSON-RPC over stdio.** `outl mcp serve --workspace <dir>` speaks
  the MCP protocol with `initialize`, `tools/list`, `tools/call`,
  `resources/list`, `resources/read`, `prompts/list`, and
  `prompts/get`. Drop the binary into Claude Desktop's
  `claude_desktop_config.json` or Cursor's `mcp.json` and the agent
  can read journals, search, follow backlinks, edit blocks, and
  toggle TODOs against the same workspace your TUI/mobile is using.
- **Tools** mirror the CLI 1:1 (`outl_page_*`, `outl_block_*`,
  `outl_daily_*`, `outl_search`, `outl_query`, `outl_tag_*`,
  `outl_prop_*`, `outl_backlinks_*`, `outl_workspace_*`) so the LLM
  sees the same surface a human would script.
- **Resources** expose read-only views over `outl://daily/today`,
  `outl://page/<slug>`, `outl://search?q=…`, etc., for clients that
  prefer URI-addressed reads to tool calls.
- **Prompts** ship `summarize_day` and friends so the agent can
  pull a daily-note summary in one round-trip.
- **Per-session workspace + cached index.** The MCP server holds
  one `WsCtx` for the life of the session and routes every read
  through `ServerCtx::with_workspace`, which reuses that handle and
  invalidates the index after lazy journal materialisation in
  `outl://daily/today` and `summarize_day`. Earlier prototypes
  opened a fresh `WsCtx` per call and self-deadlocked on the
  workspace lock the session already owned —
  `resources/read` and `prompts/get` are now part of the same
  cached path as `tools/call`.
- **MCP smoke suite** (`mcp_smoke.rs`) walks `initialize` →
  `tools/list` → `tools/call` → `resources/read` in one session so
  the lock-reuse contract can't regress.

### Security / hardening

- **Slug validation at the boundary.** `outl-actions::is_valid_slug`
  rejects empties, `.`/`..` traversal, path separators, and control
  chars before any filesystem write, surfaced as a typed
  `ActionError::InvalidSlug` (`INVALID_ARG` in the CLI/MCP
  envelope). Hugo export adds a second `target_within` check
  against canonicalised paths so a legacy bad slug imported from
  disk still cannot escape `--out`.
- **Doctor split.** `workspace doctor` runs `collect_json` (full
  lock probe, used by `outl doctor` from the shell) and
  `collect_in_session_json` (probe off, used by the MCP tool which
  already owns the lock). Before this split, `outl_workspace_doctor`
  always warned about the workspace lock on perfectly healthy
  workspaces.
- **Quieter failures stop being silent.** Page delete/rename
  replace `let _ = remove_file(...)` with a `remove_or_warn` helper
  so a broken filesystem surfaces in logs instead of disappearing.
  Regression tests cover malicious slugs, doctor-clean inside an
  MCP session, and delete being idempotent when the `.md` is
  already gone.

### Docs

- New `docs/cli.md` and `docs/mcp.md` cover the machine surface and
  the MCP wiring for Claude Desktop / Cursor end to end (envelope
  shape, every subcommand, every tool, every resource).
- Getting-started, tutorial, sync, theming, TUI, and clients docs
  refreshed for the `--workspace` rename and the new subcommand
  names.

## [0.3.1] — 2026-05-31

Mobile UX polish + autocomplete fixes. No protocol or storage
changes — drop-in upgrade from 0.3.0.

### Mobile (`outl-mobile`)

- **Autocomplete (`[[…]]`) now actually fires on iOS.** The native
  ref suggester chip strip was orphaned — `createEffect` was being
  registered after an `await` inside `onMount`, which lost Solid's
  reactive owner. State was published once at boot and never
  updated as the user typed.
- **TODO/DONE prefix is visible (and editable) in Insert mode.**
  Tapping a TODO block used to show only the checkbox + body
  (`ship it`) with the `TODO ` prefix hidden, so erasing the
  prefix from the editor was impossible. Now the prefix appears in
  the textarea (`TODO ship it`) and the checkbox flips to a bullet
  while editing — toggling state via the text Just Works.
- **Cursor lands inside `[[ ]]` / `(( ))` reliably.** `el.value =
  …` resets the textarea caret in iOS WKWebView; combined with
  Solid's `value={draft()}` rebinding the caret could end up
  outside the pair. Replaced with `setRangeText` + double
  `parkCaret` (sync + microtask) so every toolbar insert, paste
  completion, and suggester pick parks the caret where the user
  expects it.
- **Backspace inside empty `[[]]` / `(())` collapses the pair.**
  No more mashing backspace four times to undo an aborted ref.
  Same rule on TUI and mobile.
- **Smart Punctuation is OFF.** `--` no longer becomes `–`, `...`
  no longer becomes `…`, quotes stay straight. Code snippets and
  CLI commands in journals survive intact.
- **Toast pattern for errors** (auto-dismiss + Retry button) in
  place of the persistent red `<p>` that sat in the middle of the
  outline forever. Failed saves now offer a one-tap retry without
  losing the draft.
- **`commitInFlight` lock + 8 s timeout** serializes concurrent
  block edits (typing → TODO toggle → blur) so the older save
  never overwrites the newer, and a stuck Tauri command can't
  freeze Insert mode indefinitely.
- **Progressive loading message** ("Loading…" → "Connecting to
  iCloud…" → "Still waiting on iCloud…") + spinner + a Retry
  button on terminal failure. iCloud cold-start no longer reads as
  "the app froze".
- **Connectivity-aware SyncDot** uses `navigator.onLine` +
  `online`/`offline` listeners to actually show the offline pip
  (was dead code before). `aria-label` instead of `title` so iOS
  WKWebView users get the status verbally.
- **Tap targets meet Apple HIG** (~30 × 30 hit area on the
  bullet/checkbox; bullet is now actually tappable). `[[ref]]` and
  `#tag` taps navigate instead of opening the editor.
- **Long-press TODO** uses a distinct success haptic when creating
  a new TODO vs. cycling an existing one.
- **`SwipeRow` × `SwipeNavigator` conflict resolved** —
  swipe-right on the left edge no longer races the per-row
  swipe-delete (each one captures only its own direction).
- **`PageSwitcher`** opens the first match on `Enter`, dismisses
  on `Escape`, and supports swipe-down on the handle to dismiss
  (without stealing scroll from the list).
- **Backlinks empty state** so the bidirectional-linking concept
  is discoverable on freshly-created pages.
- **Performance** in long outlines: `draft()` is now a lazy getter
  prop only read by the block being edited (was triggering a
  reactive effect in every BlockRow per keystroke). Auto-resize
  coalesced into a single `requestAnimationFrame`.

### Shared (`outl-actions`)

- `edit_text` writes its argument **verbatim** instead of
  preserving a leading `TODO `/`DONE ` prefix automatically.
  Callers that surface state separately (mobile checkbox)
  reattach the prefix themselves — required so erasing the
  prefix in the editor actually sticks. TUI path is unaffected
  (it always passes the raw block text through reconcile).

### TUI (`outl-tui`)

- Backspace inside an empty `[[]]` / `(())` now collapses both
  brackets in one keystroke (matches the mobile behaviour).

## [0.3.0] — 2026-05-30

Cross-device sync goes live. A brand-new iOS app and the TUI share
the same workspace over iCloud Drive, both driven by a shared
`outl-actions` crate. Block refs / embeds land in the markdown
dialect.

### Mobile (`outl-mobile`) — new crate

- **Tauri 2 + SolidJS iOS client.** Bundle id `app.outl.mobile-app`,
  iCloud container `iCloud.app.outl.mobile-app`. Frontend is Solid +
  Tailwind; the Rust shell is intentionally thin (every workspace
  operation delegates to `outl-actions`).
- **iCloud Drive transport.** Workspace lives at
  `<ubiquity-container>/Documents/`. An ObjC bridge
  (`gen/apple/.../main.mm`) uses `NSMetadataQuery` to watch for peer
  changes and `NSFileCoordinator` + `startDownloadingUbiquitousItemAtURL`
  to force materialisation before reads — without those two steps
  the Rust side races the iCloud daemon and sees truncated op logs.
- **Per-device actor id** persisted under the app sandbox so each
  install writes to its own `ops-<actor>.jsonl`.
- iOS boot flash fixed; outl brand (palette + icon) applied across
  the app.

### Shared client (`outl-actions`) — new crate

- **Extracted every workspace mutation** (block edit, TODO toggle,
  indent / outdent, sibling create, delete, move, journal render,
  sync) out of `outl-tui` into a UI-agnostic crate. Functions take
  `&mut Workspace` + `&HlcGenerator` and route through
  `Workspace::apply` so the op log stays source of truth.
- TUI and mobile call the **same** functions for the same
  semantics — drift between clients is no longer possible by
  construction.
- `SyncEngine` interface owns the cross-device merge loop; iCloud is
  the v0 transport, iroh (phase 2) plugs in behind the same trait.

### Core (`outl-core`)

- **`JsonlStorage` op-log backend.** Single-file SQLite breaks under
  iCloud / Syncthing because the FS layer is last-write-wins per
  file. JSONL gives each actor its own `ops-<actor>.jsonl`, writes
  only to the local file, and merges every peer file on read.
- Folder layout is **`ops/`**, not `.ops/`. iCloud Documents skips
  dotted paths during cross-device sync — using a dot silently
  breaks multi-device workspaces. Same rule applied to the sidecar
  (`pages/<slug>.outl`, no leading dot).

### Markdown (`outl-md`)

- **`((blk-X))` inline refs and `!((blk-X))` embeds.** Stable
  `ref_handle` derived from the block's ULID tail (lazy 7+ char
  expansion on collision); sidecar bumped to v2. Embeds expand to
  the source root + children with a `↳` marker.
- Concurrent-safe writes over iCloud (atomic temp-file rename, no
  partial reads exposed to peers).
- `WorkspaceIndex` tracks block-ref backlinks alongside page-ref
  backlinks.

### TUI (`outl-tui`)

- Rebuilt as a **peer of shared workspaces** — same iCloud folder,
  same op log, same `outl-actions`. Edits on the laptop appear on
  the iPhone within seconds and vice versa.
- `((` autocomplete on block text, inline ref render, expanded
  embed view, Enter navigation to the source block, `/refer` and
  `/refer-embed` slash commands.
- `yr` chord copies the block's ref handle to the OS clipboard via
  arboard.
- outl brand (palette, icon, chrome) applied; mobile and TUI now
  look like the same product.

### CLI (`outl-cli`)

- **`outl migrate-to-shared` subcommand** rewrites a legacy SQLite
  workspace into the JSONL + sidecar layout consumed by both
  clients.
- `outl doctor` flags orphan `((blk-X))` and `!((blk-X))` handles.

### CI / release

- Release workflow rewritten as `prepare → tag → create_release
  (draft) → build matrix → publish_release`. Single `gh release
  create --draft` before the matrix and `gh release upload
  --clobber` per matrix leg, so paralleled jobs don't race each
  other on a repo with Immutable Releases turned on.
- macOS Intel binary now cross-compiles from `macos-latest` (ARM)
  instead of relying on the depleted `macos-13` runner pool.
- `outl-mobile` excluded from Linux CI jobs (Tauri iOS toolchain is
  macOS-only).

## [0.2.0] — 2026-05-26

Backlinks become a first-class part of the TUI: they live inline below
the outline (no more side panel), render the referencing block with
its children, and are fully editable in place.

### TUI (`outl-tui`)

- **Inline backlinks.** Replace the right-side panel with a section
  rendered below the outline, separated by a full-width `─` rule. Each
  source page shows up grouped under an icon + title header.
- **Full source block + children.** Backlinks render the referencing
  `OutlineNode` *with its subtree* (not a truncated snippet), so you
  see context without jumping to the source page.
- **Cursor navigation crosses the boundary.** `j`/`k` flow transparently
  between outline and backlinks. `app.focus: Focus::{Outline,
  Backlink{idx, sub_path}}` tracks where the cursor lives.
- **In-place edits land on the source `.md`.** `i`/`I`/`a`/`Esc`,
  `Ctrl+T` (TODO/DONE cycle), `o`/`O` (sibling create), `Tab`/`Shift+Tab`
  (indent/outdent), `dd` (delete), `K`/`J` (move up/down) — all work on
  a backlink the same way they work on the outline, persisting straight
  to the source page via `EditTarget::SourcePage`.
- **Optimistic index updates for snappy UX.** Edits patch the in-memory
  `WorkspaceIndex` immediately (next frame shows the new state), then
  save without scheduling a full workspace rebuild on the hot path.
- Cursor column preserved when entering Insert (`i` honors vim
  semantics; `I` still jumps home).
- Ghost cursor on the last outline block when focus had moved into the
  backlinks section is gone (`render_block` gates by `Focus::Outline`).
- `view.rs` split into `view/{inline, outline, overlays, backlinks}.rs`
  by responsibility — each file under 450 lines.

### Markdown (`outl-md`)

- `Backlink` carries the full `source_block: OutlineNode` and its
  `source_block_path` (DFS path in the source AST) instead of a flat
  index plus truncated snippet. Repeated refs to the same target inside
  one block collapse to a single backlink.
- `WorkspaceIndex::refresh_backlinks_from_source(path, &page)` —
  optimistic patch of every cached `source_block` for backlinks
  pointing at `path`. Used by the TUI's cross-page edit path.
- `WorkspaceIndex::patch_backlink_text(path, target_path, &new_text)`
  for text-only optimistic edits.

## [0.1.0] — 2026-05-25

First public release. Single-device editor; sync transport is on the
roadmap but the algorithm and op-log infrastructure are already in.

### Core (`outl-core`)

- Tree CRDT implementation following Kleppmann et al. 2022
  (`do_op` / `undo_op` / `apply_op` / `creates_cycle`).
- HLC timestamps with actor tiebreak.
- Append-only op log with sqlite backend (`SqliteStorage`).
- `Storage` trait so alternative backends (e.g. ChronDB) can slot in
  without touching the CRDT.
- Workspace file lock via `fs2::flock` — two `outl` processes on the
  same workspace get a clean error, not a race.
- Property-based test of strong eventual consistency over 100+
  randomised op permutations.

### Markdown / sidecar (`outl-md`)

- CommonMark parse + render with the outl dialect (`title::`,
  `icon::`, page/block properties, `[[refs]]`, `#tags`,
  `((block-id))`, fenced code blocks, multi-line block text).
- `.foo.outl` JSON sidecar holding the IDs the `.md` deliberately
  doesn't carry. **The `.md` stays clean** — no `id::`, no UUIDs.
- 3-level matching algorithm (`outl-md::matching`) reconstructs which
  block kept which ID after an external editor saves the file.
- Workspace index (`WorkspaceIndex`) — title, icon, slug, backlinks,
  tag namespace; powers the switcher, autocomplete and backlinks
  panel. Built once on boot, refreshed in a worker thread on save.
- Roundtrip property test: `parse(render(ast)) == ast` over randomly
  generated outlines including multi-line and fenced cases.

### Code-block execution (`outl-exec`)

- `Runtime` trait + `RuntimeRegistry`. Shipped runtimes (each behind
  a Cargo feature for opt-out distributions):
  - `lisp` — Steel (Scheme R5RS-ish in pure Rust).
  - `js` — Boa (ES2015+ in pure Rust).
  - `python` — RustPython (Python 3 subset).
  - `lua` — mlua 5.4 (vendored).
  - `rust` — `rustc → wasm32-wasip1 → wasmtime`. Compiled artefacts
    cached in `~/.cache/outl/runtimes/rust/<hash>.wasm`. ~20× faster
    on a re-run of the same snippet.
- WASM sandbox infrastructure (wasmtime engine + WASI ctx with no
  preopens / no env / no sockets, fuel-based instruction cap,
  epoch-interruption timeout, in-memory stdin/stdout/stderr).
- Idempotent result subblock — re-running the same code overwrites
  the existing `> **result:**` child instead of duplicating it.
- `source-hash::` stamped on each result child so the upcoming auto-run
  loop can short-circuit unchanged sources.

### TUI (`outl-tui`)

- Journal-first: opens on today's date.
- Vim-style modes (Normal / Insert / Visual) with chord support
  (`dd`, `gg`, `gx`, `yy`, `qq`-to-quit).
- Insert mode autocomplete for `[[refs]]`, `#tags`, and `/commands`
  (Notion-style slash menu).
- Slash command system + vim palette share one registry — every
  built-in command shows up in both. Built-ins: `prop-block`,
  `prop-page`, `search`, `run`, `theme`, `today`, `open`,
  `refresh`, `write`, `quit`, `help`. The registry is the
  plugin-extension point.
- `gx` runs the code block under the cursor through `outl-exec`.
- `auto-run::` property runs a block automatically on page open
  (cache-aware via SHA-256 of the source).
- `icon::` page property surfaces in every place the title shows
  (header, switcher, backlinks panel, search results, autocomplete,
  inline `[[refs]]`).
- Multi-line blocks via `Alt+Enter` / `Ctrl+J` / `Shift+Enter`
  (Shift+Enter only on terminals that speak the kitty keyboard
  protocol); plain `Enter` auto-detects an open code fence and
  inserts a soft newline inside it.
- Vertical scroll with `PgUp`/`PgDn`/`Ctrl+D`/`Ctrl+U`/`gg`/`G` and
  auto-scroll when the selection moves off-screen.
- Hot reload on external `.md` edits (polls mtime every 750ms; warns
  instead of clobbering when you're mid-Insert).
- Error modal overlay for multi-line failures (rustc compile errors,
  traps, missing toolchain), keeping the status line for short
  successes.
- Themes: 11 presets, switchable with `/theme <name>` at runtime.

### CLI (`outl-cli`)

- `outl` (no subcommand) opens the TUI in `$PWD`.
- `outl init <path>` scaffolds a workspace.
- `outl serve [--once]` reconciles `.md` files into the op log
  (one-shot or watch mode).
- `outl import logseq <src> <dst>` and `outl import roam <backup.json>
  <dst>` strip `id::` lines, slugify, seed sidecars.
- `outl doctor` and `outl reconcile` placeholders for the integrity
  and orphan-resolution flows.

### Tooling / DX

- Workspace MSRV: rustc 1.88.
- CI: `fmt` + `clippy -D warnings` + `cargo test --workspace --all-targets`
  on Linux and macOS.
- Bench CI: `small` / `medium` / `large` on every PR + push;
  `xlarge` (10k+ files) on weekly cron + manual dispatch.
- File-size guard hook (`.claude/hooks/file-size-guard.sh`) blocks
  Rust files past ~900 LOC unless the change is intentional —
  forces a refactor conversation before drift accumulates.
- Background workspace-index build: `App::new` paints the journal
  immediately and spawns a worker thread for the global index;
  backlinks/icons fill in within ~ms of boot.

### License

MIT.

[0.1.0]: https://github.com/avelino/outl/releases/tag/v0.1.0
