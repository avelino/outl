# Shared primitives catalog

**Before writing any helper, scan these tables first.**
Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here —
the cost of finding the existing one is a `grep`;
the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).


> This catalog is mirrored at [`.github/copilot-instructions.md`](https://github.com/avelino/outl/blob/main/.github/copilot-instructions.md) §5.1.
> When you edit either copy, sync both — a `PostToolUse` hook flags drift, but the discipline starts before the hook fires.

The catalog is grouped by area.
Skim the headings, then drill in.

For the reuse-first rule (why this matters, past drift incidents, what to do when a primitive doesn't exist yet), see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).

## 1. Workspace lifecycle, op log, and HLC (outl-core)

| Intent | Use this | File |
|---|---|---|
| Open a workspace (in-memory for tests, on-disk JSONL for prod) | `outl_core::Workspace::open_in_memory` / `open_with_storage` | `crates/outl-core/src/workspace.rs` |
| Route an op through the log → tree (the **only** mutation path) | `outl_core::Workspace::apply(LogOp)` | `crates/outl-core/src/workspace.rs` |
| Read the materialized tree / op log from a workspace | `outl_core::Workspace::tree` / `log` / `block_text` | `crates/outl-core/src/workspace.rs` |
| Build a Yrs text-replace update payload for an op | `outl_core::Workspace::build_text_replace_update` | `crates/outl-core/src/workspace.rs` |
| Generate HLC timestamps with actor tiebreak (required for every op) | `outl_core::HlcGenerator::new` / `next` / `observe` | `crates/outl-core/src/hlc.rs` |
| Wrap an `Op` into a `LogOp` (timestamp + actor) for `apply` | `outl_core::Op` + `outl_core::LogOp` | `crates/outl-core/src/op.rs` |
| Extract the `NodeId` an op targets | `outl_core::op::op_node(&Op) -> Option<NodeId>` | `crates/outl-core/src/op.rs` |
| Sentinel node ids (`root`, `trash`) | `outl_core::NodeId::root()` / `trash()` | `crates/outl-core/src/id.rs` |
| Per-device identity for ops | `outl_core::ActorId` | `crates/outl-core/src/id.rs` |
| Stable, shared workspace identity (read/generate, persist, pairing-adoption) — the gossip-topic key, NOT the path | `outl_core::WorkspaceId::read_or_create` / `write` / `from_raw` (errors: `outl_core::WorkspaceIdError`) | `crates/outl-core/src/workspace_id.rs` |
| Fractional index for sibling ordering | `outl_core::Fractional` | `crates/outl-core/src/fractional.rs` |

## 2. Tree reads (outl-core + outl-actions::tree)

| Intent | Use this | File |
|---|---|---|
| Does a node still exist in the tree? | `Tree::contains` | `crates/outl-core/src/tree/mod.rs` |
| Parent of a node | `Tree::parent` | `crates/outl-core/src/tree/mod.rs` |
| Fractional position of a node | `Tree::position` | `crates/outl-core/src/tree/mod.rs` |
| Single property lookup on a node | `Tree::property` | `crates/outl-core/src/tree/mod.rs` |
| Iterate every property currently set on a node | `Tree::properties_of` | `crates/outl-core/src/tree/mod.rs` |
| Collapsed flag for a node | `Tree::is_collapsed` / `collapsed_ids` | `crates/outl-core/src/tree/mod.rs` |
| Walk every node in the tree | `Tree::iter_nodes` / `node_count` | `crates/outl-core/src/tree/mod.rs` |
| Children of a parent (in fractional order) | `outl_actions::tree::children_of` | `crates/outl-actions/src/tree.rs` |
| Walk a subtree applying a closure | `outl_actions::tree::walk_subtree` | `crates/outl-actions/src/tree.rs` |
| Sibling after a node + position helpers (for inserts) | `outl_actions::tree::next_sibling` / `position_after` / `position_for_new_last_child` | `crates/outl-actions/src/tree.rs` |
| Which page (slug-bearing root child) does this node sit under? | `outl_actions::tree::enclosing_page_id` | `crates/outl-actions/src/tree.rs` |

## 3. Block mutations (outl-actions::block + collapsed + todo + quote)

Every entry here routes through `Workspace::apply` — never build a `LogOp` from a client and apply it directly.

| Intent | Use this | File |
|---|---|---|
| Append a single block under a parent | `outl_actions::block::append_block` | `crates/outl-actions/src/block/create.rs` |
| Append a tree / forest (with children) under a parent | `outl_actions::block::append_tree` / `append_forest` (uses `BlockTreeSpec` → returns `BlockTreeOutcome`) | `crates/outl-actions/src/block/create.rs` |
| Create sibling before a block (vim `O`; floor-slot swap when the anchor is first child) | `outl_actions::block::create_before` | `crates/outl-actions/src/block/create.rs` |
| Create sibling after / child under a block | `outl_actions::block::create_after` / `create_under` | `crates/outl-actions/src/block/create.rs` |
| Create sibling after a block, appending at page end when the anchor is stale | `outl_actions::block::create_after_or_append` (the desktop/mobile `create_block` stale-anchor fallback — one owner, no per-client duplication) | `crates/outl-actions/src/block/create.rs` |
| Edit a block's text | `outl_actions::block::edit_text` | `crates/outl-actions/src/block/edit.rs` |
| Move a block to sit **after an arbitrary target** (cut-and-paste-block; crosses pages; one `Op::Move`, preserving id + refs; rejects self-subtree cycles) | `outl_actions::block::move_after` | `crates/outl-actions/src/block/moves.rs` |
| Indent / outdent / move up / move down a block | `outl_actions::block::indent` / `outdent` / `move_up` / `move_down` | `crates/outl-actions/src/block/moves.rs` |
| Re-parent a block under an arbitrary page/block (cross-page move) | `outl_actions::block::move_under` | `crates/outl-actions/src/block/moves.rs` |
| Delete a block (`Move(node, TRASH_ROOT)`, **never** physical) | `outl_actions::block::delete` | `crates/outl-actions/src/block/moves.rs` |
| Toggle a block's collapsed flag (converges via `Op::SetCollapsed`) | `outl_actions::collapsed::toggle_block_collapsed` / `set_block_collapsed` | `crates/outl-actions/src/collapsed.rs` |
| Cycle / split / read TODO/DONE state (encoded as text prefix) | `outl_actions::todo::cycle_todo` / `split_todo` / `TodoState` / `TODO_PREFIX` / `DONE_PREFIX` | `crates/outl-actions/src/todo.rs` |
| Toggle TODO/DONE on a block in one call | `outl_actions::block::toggle_todo` | `crates/outl-actions/src/block/edit.rs` |
| Read / toggle blockquote state (encoded as `"> "` text prefix, CommonMark-compatible) | `outl_actions::quote::is_quote` / `split_quote` / `toggle_quote` / `QUOTE_PREFIX` | `crates/outl-actions/src/quote.rs` |
| Toggle blockquote on a block in one call | `outl_actions::block::toggle_quote` | `crates/outl-actions/src/block/edit.rs` |

## 4. Pages and journals (outl-actions::page + journal)

| Intent | Use this | File |
|---|---|---|
| Page-property keys (constants — don't hardcode the strings) | `outl_actions::page::SLUG_KEY` / `KIND_KEY` / `TYPE_KEY` / `TITLE_KEY` | `crates/outl-actions/src/page.rs` |
| Canonical `type::` value marking a page as a person (`@` mention autocomplete filter) | `outl_actions::page::PERSON_TYPE` | `crates/outl-actions/src/page.rs` |
| Page metadata (slug, kind, title, **`page_type`**) for a node id | `outl_actions::page::page_meta` / `PageMeta` / `PageKind` | `crates/outl-actions/src/page.rs` |
| Validate a slug for filesystem safety (`..`, `/`, `\`, control chars) | `outl_actions::page::is_valid_slug` | `crates/outl-actions/src/page.rs` |
| Derive a **deterministic page/journal-root id** from slug (so every creation path — in-app, `outl-md` reconcile, desync recovery — converges on ONE root; the single owner) | `outl_core::NodeId::from_slug` (thin wrapper `outl_actions::page::page_id_from_slug`) | `crates/outl-core/src/id.rs` |
| Find / list / create-if-missing pages (`find_by_slug` resolves a deterministic winner when a slug has >1 root, so a split-brain workspace stops flickering pre-merge) | `outl_actions::page::find_by_slug` / `list_all` / `open_or_create` | `crates/outl-actions/src/page.rs` |
| Repair a split-brain workspace where a slug has >1 page/journal root (re-parents every child under the canonical root, trashes the emptied duplicates, all via `Op`s so it converges on every device; idempotent) | `outl_actions::merge_duplicate_slug_roots` (impl `outl_actions::page_merge`) | `crates/outl-actions/src/page_merge.rs` |
| Repair journal titles doubled by concurrent offline creation (two devices minted the same deterministic root and each wrote the slug into the root's Yrs text, so the concurrent inserts concatenated into `"2026-06-252026-06-25"`; clears the text via `Op::Edit` so the title falls back to the slug; idempotent, journal-only) | `outl_actions::repair_doubled_journal_titles` (impl `outl_actions::page_repair_titles`) | `crates/outl-actions/src/page_repair_titles.rs` |
| Delete a page (move root to `NodeId::trash()` via one `Op::Move`; whole subtree travels with it; returns `PageMeta` so callers can drop projections + navigate away; `ActionError::PageNotFound` when the slug doesn't resolve) | `outl_actions::page::delete` (re-exported as `outl_actions::delete_page`) | `crates/outl-actions/src/page.rs` |
| Remove a page's `.md` + `.outl` from disk (the inverse of `apply_page_md_with_sidecar`; idempotent on missing files; pairs with `page::delete`) | `outl_actions::journal::remove_page_projection` (re-exported at crate root) | `crates/outl-actions/src/journal.rs` |
| Open-or-create a page from a **human-typed name** (slugifies + keeps original as title, used when a `[[ref]]` / `#tag` / picker query may not be a valid slug) | `outl_actions::resolve::open_or_create_by_name` | `crates/outl-actions/src/resolve.rs` |
| Open-or-create whatever a **user-typed ref target** points at (date → journal, else literal/slugified/title match → existing page, else create) — handles `@`-prefixed mentions by stripping the `@` and marking new pages as `type:: person`; the one decision tree so frontend regex and backend parser cannot drift on `[[2026-13-01]]` or `[[@avelino]]` | `outl_actions::resolve::open_or_create_by_ref` | `crates/outl-actions/src/resolve.rs` |
| Search pages typed `type:: person`, fuzzy-ranked by query (powers the `@` mention autocomplete in every client) | `outl_actions::person::search_persons` | `crates/outl-actions/src/person.rs` |
| Read / write a property on a page (or any node) | `outl_actions::page::read_text_prop` / `set_property` | `crates/outl-actions/src/page.rs` |
| Migrate pre-page-model blocks under today's journal (run on boot) | `outl_actions::page::migrate_legacy_into_today` | `crates/outl-actions/src/page.rs` |
| Open / create the journal for a specific date or today | `outl_actions::page::open_journal` / `open_today` | `crates/outl-actions/src/page.rs` |
| Journal date labels & day arithmetic (slug ↔ date, title, `[[YYYY-MM-DD]]` ref, prev/next day) | `outl_actions::dates::journal_slug` / `journal_title` / `journal_ref` / `date_from_slug` / `previous_journal_date` / `next_journal_date` | `crates/outl-actions/src/dates.rs` |
| Today's journal date in the configured timezone (delegates to `clock`) | `outl_actions::page::today` | `crates/outl-actions/src/page.rs` |
| Week arithmetic — ISO-week tag (`#2026-W22`, `%G`-correct at year boundaries) and "days until next `<weekday>`" (same weekday → 7, never 0) | `outl_actions::dates::week_tag` / `days_until_next_weekday` | `crates/outl-actions/src/dates.rs` |
| Current date / time in the user's configured timezone (`[calendar] timezone`, DST-aware via chrono-tz; OS local when unset). Call `init` once per client at boot; `page::today` delegates here, so use `now_local` / `today` instead of `chrono::Local::now()` (issue #107) | `outl_actions::clock::init` / `now_local` / `today` | `crates/outl-actions/src/clock.rs` |
| Parse a **human-typed date** in any supported spelling (`2026-04-22`, `2026/04/22`, `22/04/2026`, Roam's `April 22nd, 2026`, `Sept 3rd, 2025`, `22 April 2026`) into a `NaiveDate`, or into the ISO label outl uses for journal slugs / `[[date]]` refs — the one owner of the ordinal-stripping logic that used to be copied in four places (paste normalization, `outl daily`, `outl import`, Obsidian frontmatter). `parse_date_arg` layers **relative offsets** (`+3d`, `-2w`, `+1m`, bare `5d`) on top for slash-command / CLI arguments | `outl_actions::dates::parse_flexible_date` / `parse_date_label` / `parse_date_arg` | `crates/outl-actions/src/dates.rs` |
| Parse an `outl://` deep link URL into a navigation target (one parser, every GUI client routes the result to its own `open_*` command — never reparse per client) | `outl_actions::parse_deep_link` / `DeepLinkTarget` / `DeepLinkError` / `DEEP_LINK_SCHEME` | `crates/outl-actions/src/deeplink.rs` |
| Filesystem paths for journals / pages / a specific page | `outl_actions::journal::journals_dir` / `pages_dir` / `page_md_path` | `crates/outl-actions/src/journal.rs` |
| Render a page node out to `.md` | `outl_actions::journal::render_page_md` | `crates/outl-actions/src/journal.rs` |
| Apply an edited `.md` back into the workspace (with / without sidecar) | `outl_actions::journal::apply_page_md` / `apply_page_md_with_sidecar` | `crates/outl-actions/src/journal.rs` |
| Apply every page's `.md` to disk in one pass | `outl_actions::journal::apply_all_pages_md` | `crates/outl-actions/src/journal.rs` |
| Run a closure that mutates a page's `.md` (read → modify → write atomically) | `outl_actions::journal::mutate_page_md` | `crates/outl-actions/src/journal.rs` |
| Atomic `.md` write (crash-safe, wraps `outl_md::atomic::write_atomic`) | `outl_actions::journal::write_md_atomic` | `crates/outl-actions/src/journal.rs` |

## 5. Parse / render (outl-md::parse + render)

| Intent | Use this | File |
|---|---|---|
| Parse `.md` → outline AST (no IDs) | `outl_md::parse::parse` → `ParsedPage` (includes `warnings: Vec<ParseWarning>`) | `crates/outl-md/src/parse.rs` |
| Render outline AST → `.md` (clean, no IDs) | `outl_md::render::render` | `crates/outl-md/src/render.rs` |
| Non-fatal parser recovery records (heading instead of bullet, etc.) | `outl_md::ParseWarning` + `outl_md::ParseWarningKind` (re-exported from `parse`) | `crates/outl-md/src/parse.rs` |
| The outline AST node DTO (UI-friendly, no `Workspace` coupling) | `outl_md::OutlineNode` / `outl_actions::outline::OutlineNode` | `crates/outl-md/src/parse.rs` + `crates/outl-actions/src/outline.rs` |
| Project the workspace tree under a node into the UI DTO | `outl_actions::outline::project_outline` / `project_outline_node` | `crates/outl-actions/src/outline.rs` |
| Flatten an `OutlineNode` subtree to DFS paths (for selection / navigation) | `outl_actions::outline::flatten_subtree_paths` | `crates/outl-actions/src/outline.rs` |
| Read a page from disk + project to outline view in one call | `outl_actions::outline::read_page_view` / `read_page_view_with_workspace` | `crates/outl-actions/src/outline.rs` |
| Read a page **and** surface parser warnings (banner, doctor, status line) | `outl_actions::outline::read_page_outline` / `read_page_outline_with_workspace` → `PageOutline { nodes, warnings }` | `crates/outl-actions/src/outline.rs` |
| Slugify a user-visible page name into a filesystem-safe slug (lowercase, folds Latin diacritics `á` → `a`, non-alphanumerics collapse to `-`; empty input → `UNTITLED_SLUG`) — never hand-roll an ASCII-only copy | `outl_md::slug::slugify` / `UNTITLED_SLUG` | `crates/outl-md/src/slug.rs` |

## 6. External markdown coercion & ingest (outl-md::frontmatter + wikilink, outl-actions::paste + ingest)

| Intent | Use this | File |
|---|---|---|
| Coerce **external markdown** (line endings, indent unit 4→2, Roam/GitHub/Logseq tokens, long-form dates → ISO, strip `id::` with Crockford validation, strip unknown `{{…}}` / `^^…^^`) | `outl_actions::paste::normalize_external_syntax` | `crates/outl-actions/src/paste/normalize.rs` |
| Split a leading `---` YAML frontmatter fence off a `.md` body (CRLF-safe, honours `...` end marker, no-fence → verbatim body) | `outl_md::frontmatter::split_frontmatter` | `crates/outl-md/src/frontmatter.rs` |
| Parse a YAML frontmatter block into flat `key:: value` properties (`title` lifted, `tags` normalized to `#name`, caller-supplied drop-list; values verbatim — date normalization stays with the caller) | `outl_md::frontmatter::parse_frontmatter` → `Frontmatter` | `crates/outl-md/src/frontmatter.rs` |
| Lift a leading `# H1` line into a page title (first non-blank line only) | `outl_md::frontmatter::extract_leading_h1` | `crates/outl-md/src/frontmatter.rs` |
| Collapse external wiki-link variants (`[[Note\|alias]]`, `[[Note#h]]`, `[[Note^blk]]`, `[[folder/Note]]`) to canonical `[[Note]]` | `outl_md::wikilink::rewrite_wikilinks` (whole text) / `clean_wikilink_target` (one target) | `crates/outl-md/src/wikilink.rs` |
| Convert image wiki-links / embeds (`![[img.png]]`, `[[a/b.jpeg\|cap]]`) into CommonMark links, folder path preserved | `outl_md::wikilink::convert_image_links` (+ `is_image_target` predicate) | `crates/outl-md/src/wikilink.rs` |
| "Does this clipboard look like an outline?" classifier | `outl_actions::paste::looks_like_outline` | `crates/outl-actions/src/paste/mod.rs` |
| Convert clipboard markdown into outl ops grafted at a position | `outl_actions::paste::paste_markdown` → `PasteOutcome` (anchor described by `PasteAnchor`) | `crates/outl-actions/src/paste/mod.rs` |
| Paste raw text as a single block with no normalisation or outline parsing (the "without formatting" path) | `outl_actions::paste::paste_plain(workspace, hlc, anchor, raw)` → `PasteOutcome` | `crates/outl-actions/src/paste/mod.rs` |
| Serialize a block selection (+ subtrees) to clean outl markdown **for the clipboard** (the inverse of `paste_markdown` / `parse`) | `outl_actions::clipboard::copy_markdown` (workspace + `NodeId`s; GUI backends) / `copy_markdown_nodes` (already-projected `OutlineNode`s; the TUI's AST-first yank) | `crates/outl-actions/src/clipboard.rs` |
| **Ingest a `.md` as a real page** (creates page node + reconciles blocks; used by import / `serve` / mobile + TUI orphan scanners) | `outl_actions::ingest::ingest_md_file` / `ingest_dir` | `crates/outl-actions/src/ingest.rs` |
| Create stub pages for every `[[ref]]` with no file of its own (Logseq "implicit pages") | `outl_actions::ingest::create_missing_ref_pages` | `crates/outl-actions/src/ingest.rs` |

## 7. Reconcile & matching (outl-md::reconcile + matching + diff)

| Intent | Use this | File |
|---|---|---|
| Reconcile an existing `.md` against its sidecar (3-level matching → diff → min ops) | `outl_md::reconcile::reconcile_md` (no sidecar = fresh random id) / `reconcile_md_with_page_id` (pin id for first ingest) | `crates/outl-md/src/reconcile.rs` |
| Reconcile every `.md` in a directory | `outl_md::reconcile::reconcile_dir` | `crates/outl-md/src/reconcile.rs` |
| Materialise a page root in the tree (`Op::Create` at root + `page-slug` / `page-kind` `SetProp`s, idempotent) without running the full reconcile pipeline | `outl_md::reconcile::ensure_page_root_in_tree` | `crates/outl-md/src/reconcile.rs` |
| Reconcile error / report types | `outl_md::ReconcileError` / `ReconcileReport` | `crates/outl-md/src/reconcile.rs` |
| 3-level matching algorithm (hash → similarity → orphan log) | `outl_md::matching::match_blocks` → `Match` / `MatchLevel` | `crates/outl-md/src/matching.rs` |
| Diff old AST + new AST + old sidecar → minimum sequence of `Op`s | `outl_md::diff::diff_to_ops` → `DiffPlan` | `crates/outl-md/src/diff.rs` |
| Same diff, **plus** propagate page-level properties (`title::`, `type::`, `pinned::`, `icon::`, …) into the op log as `Op::SetProp` on the page root so the CRDT tree agrees with what's on disk (legacy `.md` files populated via fixtures / external editors get materialised here on the next reconcile) | `outl_md::diff::diff_to_ops_with_page_props` | `crates/outl-md/src/diff.rs` |
| Reconcile-pipeline version number stamped on every sidecar — orphan scanner re-runs `reconcile_md` when a sidecar's version is below this constant, so a binary that gains a new pipeline pass automatically rematerialises every legacy page on the next boot | `outl_md::sidecar::CURRENT_PIPELINE_VERSION` | `crates/outl-md/src/sidecar.rs` |

## 8. Sidecar (outl-md::sidecar + atomic)

| Intent | Use this | File |
|---|---|---|
| The full sidecar struct + per-block entries | `outl_md::Sidecar` / `SidecarBlock` | `crates/outl-md/src/sidecar.rs` |
| Construct a fresh sidecar for a new page | `outl_md::sidecar::Sidecar::new_for_page(page_id, &file_hash)` | `crates/outl-md/src/sidecar.rs` |
| Read / write sidecar (JSON, version 2, backward-reads v1) | `outl_md::sidecar::read` / `write` | `crates/outl-md/src/sidecar.rs` |
| Sidecar path resolution for a `.md` | `outl_md::sidecar::sidecar_path_for` / `resolve_sidecar_path` | `crates/outl-md/src/sidecar.rs` |
| Derive `((blk-XXXXXX))` ref handle from `NodeId` (deterministic, collision-aware) | `outl_md::sidecar::derive_ref_handle` | `crates/outl-md/src/sidecar.rs` |
| Hash block / file content for sidecar (`content_hash` = single block; `file_hash` = whole `.md`) | `outl_md::sidecar::content_hash` / `file_hash` | `crates/outl-md/src/sidecar.rs` |
| Low-level crash-safe write (use the `journal::write_md_atomic` wrapper unless you have a reason) | `outl_md::atomic::write_atomic` | `crates/outl-md/src/atomic.rs` |

## 9. In-flight outline AST helpers (outl-md::outline_ops)

These operate on `Vec<OutlineNode>` **before** the tree is rebuilt from the op log — typing into a buffer that hasn't been parsed back yet.
UI-agnostic; both TUI and mobile consume them.

| Intent | Use this | File |
|---|---|---|
| Flat count / TODO+DONE counts across an outline | `outline_ops::flat_count` / `count_todos` | `crates/outl-md/src/outline_ops.rs` |
| Convert flat index ↔ path / look up a node at a path | `outline_ops::path_for_index` / `index_for_path` / `node_at_path` / `node_at_path_mut` | `crates/outl-md/src/outline_ops.rs` |
| Count descendants under a path / grab a mutable siblings slice | `outline_ops::descendants_count_at_path` / `siblings_mut` | `crates/outl-md/src/outline_ops.rs` |
| Insert a sibling before / after a path | `outline_ops::insert_sibling_before` / `outline_ops::insert_sibling_after` | `crates/outl-md/src/outline_ops.rs` |
| Indent / outdent / delete / move up / move down at a path | `outline_ops::indent_at_path` / `outdent_at_path` / `delete_at_path` / `move_up_at_path` / `move_down_at_path` | `crates/outl-md/src/outline_ops.rs` |

## 10. Indices and search (outl-md::index + block_index)

| Intent | Use this | File |
|---|---|---|
| Build / query the workspace-wide index (slug → page, backlinks, block lookups) | `outl_md::WorkspaceIndex::build` / `by_slug` / `by_title` / `pages` / `pages_by_title_prefix` / `pages_by_type` | `crates/outl-md/src/index.rs` |
| Patch / remove a page in an existing index | `WorkspaceIndex::patch_page` / `remove_page` | `crates/outl-md/src/index.rs` |
| Resolve `((blk-XXXXXX))` to a block / look a block up by id or location | `WorkspaceIndex::resolve_block_ref` / `block_by_id` / `block_at_location` | `crates/outl-md/src/index.rs` |
| Reverse refs to a block / iterate / search | `WorkspaceIndex::block_refs_to` / `iter_blocks` / `search_block_text` / `block_count` / `block_index` (borrow the inner `BlockIndex`) | `crates/outl-md/src/index.rs` |
| Stand-alone block-level index (when you don't need the page facade) | `outl_md::BlockIndex` + `BlockEntry` + `BlockReference` | `crates/outl-md/src/block_index.rs` |
| `PageEntry` DTO returned by `WorkspaceIndex` lookups | `outl_md::PageEntry` | `crates/outl-md/src/index.rs` |

## 11. View helpers for editors (outl-md::view + inline)

| Intent | Use this | File |
|---|---|---|
| Char ↔ (line, col) on a buffer (both TUI and mobile editors share) | `outl_md::view::char_to_line_col` / `line_col_to_char` | `crates/outl-md/src/view.rs` |
| Project a block to renderable rows (with `BlockRowKind` discrimination) | `outl_md::view::block_to_rows` → `BlockRow` / `BlockRowKind` | `crates/outl-md/src/view.rs` |
| Tokenize inline markdown (`**bold**`, `[[refs]]`, `#tags`, `((blk-…))`, `!((blk-…))`) | `outl_md::inline::tokenize` → `InlineTok` | `crates/outl-md/src/inline.rs` |
| Tokenize inline markdown into an **owned, Serde-friendly** form for wire / DTO payloads (mobile renders these straight; no parallel TS tokenizer) | `outl_md::inline::tokenize_owned` → `InlineToken` | `crates/outl-md/src/inline.rs` |
| Reconstruct the source markdown from a `Vec<InlineTok>` (Bold / Italic / Strike now carry recursively-tokenized inners; use this when a surface wants the whole inner span as one styled string instead of dispatching per-variant) | `outl_md::inline::inline_to_source` | `crates/outl-md/src/inline.rs` |
| Resolve the ref under a caret position (`Page` / `Journal` / `Tag` / `Block`) | `outl_md::inline::ref_at_cursor` → `RefTarget` | `crates/outl-md/src/inline.rs` |
| Does a text mention `#tag` as a whole tag token? Boundary-correct via the tokenizer (`#tag-longer` / `#tagged` never match `tag`; `#tag` inside a code span is not a tag) — never use `text.contains("#tag")` | `outl_md::tag::text_contains_tag` | `crates/outl-md/src/tag.rs` |
| Validate a `((blk-XXXXXX))` handle string | `outl_md::inline::is_valid_block_handle` | `crates/outl-md/src/inline.rs` |
| Byte offset for a char index (UTF-8 safe) | `outl_md::inline::byte_index_for_char` | `crates/outl-md/src/inline.rs` |
| Canonicalize a fence info-string (`rs` → `rust`, `js`/`javascript`/`node` → `js`, …) — single source of truth for both `outl-exec`'s runtime dispatch and the frontend syntax highlighter | `outl_md::lang::canonical`, `outl_md::lang::KNOWN_ALIASES` | `crates/outl-md/src/lang.rs` |
| Resolve a `:shortcode:` to its unicode glyph (one-way; never retro-translate glyph → shortcode, multiple shortcodes can alias the same codepoint) | `outl_md::emoji::shortcode_to_unicode` | `crates/outl-md/src/emoji.rs` |
| Validate the `[a-z0-9_+-]+` shape of an emoji shortcode (does **not** check the catalog — that's `shortcode_to_unicode`) | `outl_md::emoji::is_valid_shortcode` | `crates/outl-md/src/emoji.rs` |
| Validate **one char** of a shortcode (`[a-z0-9_+-]`) — use this when walking the buffer char-by-char (`try_emoji`, TUI's `detect_trigger`) so you don't allocate a 1-char `String` per keystroke just to call `is_valid_shortcode` | `outl_md::emoji::is_valid_shortcode_char` | `crates/outl-md/src/emoji.rs` |
| Search the GitHub gemoji catalog for shortcodes matching a query (exact → prefix → substring; shorter shortcodes win ties) — powers the `:emoji:` autocomplete in every client through one shared `outl_emoji_search` Tauri command | `outl_md::emoji::search` → `EmojiHit` | `crates/outl-md/src/emoji.rs` |

## 12. Backlinks (outl-actions::backlinks)

| Intent | Use this | File |
|---|---|---|
| Extract `[[ref]]` tokens out of a block's text (tolerates unbalanced openers) | `outl_actions::backlinks::extract_refs` | `crates/outl-actions/src/backlinks.rs` |
| Backlink DTO returned by the queries below | `outl_actions::backlinks::Backlink` | `crates/outl-actions/src/backlinks.rs` |
| Walk every backlink for a target / a `PageMeta` (matches `[[ref]]` literally **and** `#tag` via slugify — same resolution a tag click uses) | `outl_actions::backlinks::backlinks_for_target` / `backlinks_for_page` | `crates/outl-actions/src/backlinks.rs` |
| Order a backlinks list chronologically (group-stable by source page, newest- or oldest-first; drives the issue-#142 direction toggle on every client) | `outl_actions::sort_backlinks` | `crates/outl-actions/src/backlinks_sort.rs` |

## 13. Code-block execution (outl-actions::exec)

The **cross-client glue** every UI uses to wire a "run this fence" gesture (TUI `g x`, desktop `Cmd+Shift+X`, mobile long-press → "Run code") through to `outl-exec` and back.
`outl_actions::exec::run_code_block` is the **only** entry point a Tauri command / TUI action should call — never re-implement the flat-DFS walk, the `.md` path lookup, or the DTO shape per client.

| Intent | Use this | File |
|---|---|---|
| Resolve a `NodeId` to its flat DFS index inside an outline forest (the order `outl_exec::run_block_at_index` expects) | `outl_actions::flat_index_for_block` | `crates/outl-actions/src/outline.rs` |
| Orchestrate execution: walk DFS, resolve `.md` path, call `outl_exec::run_block_at_index`, build DTO | `outl_actions::exec::run_code_block` | `crates/outl-actions/src/exec.rs` |
| Serializable mirror of `outl_exec::ExecOutput` (stdout/stderr/duration_ms/exit) | `outl_actions::ExecOutputDto` | `crates/outl-actions/src/exec.rs` |
| Outcome shipped to the client (`language` + `result_ok` xor `error`; client adds the refreshed `view`) | `outl_actions::RunCodeBlockOutcome` | `crates/outl-actions/src/exec.rs` |

The runtime catalog (which languages are available) is selected by the **binary** that consumes this crate, via `outl-exec` features in its own `Cargo.toml`.
`outl-actions` itself depends on `outl-exec` with `default-features = false` so it doesn't drag `wasmtime` (Rust runtime) into the mobile IPA via the back door.

The `query` runtime (`outl_exec::runtimes::query`) is a special case: it returns `OutputFormat::Embeds` instead of `OutputFormat::Text`, so the orchestrator renders results as live `!((blk-XXXXXX))` embeds rather than a code-fence stdout dump.
It also overrides `Runtime::auto_run()` to return `true`, so query blocks always re-run on page load without needing the `auto-run::` property or manual `gx`.

The query engine also exposes a **structured API** for plugins and code that runs outside the ` ```query ` fence:

| Intent | Use this | File |
|---|---|---|
| Structured query from a `QueryParams` object (plugin-facing) | `outl_exec::run_query_structured` | `crates/outl-exec/src/runtimes/query.rs` |
| DSL query from a string (user-facing) | `outl_exec::run_query_dsl` | `crates/outl-exec/src/runtimes/query.rs` |
| Query parameters struct (status, tag, kind, since, text, sort, limit) | `outl_exec::QueryParams` | `crates/outl-exec/src/runtimes/query.rs` |
| Query result hit (handle, text, status, page) | `outl_exec::QueryHit` | `crates/outl-exec/src/runtimes/query.rs` |

In JS code blocks, the same API is available as `outl.query({ status: "todo", … })`.

## 14. Sync engine, locks, storage trait

| Intent | Use this | File |
|---|---|---|
| The shared sync entry point (TUI poller + mobile iCloud watcher both use it) | `outl_actions::SyncEngine::new` | `crates/outl-actions/src/sync.rs` |
| Bind a sync engine to an explicit transport (iroh, test doubles) | `SyncEngine::with_transport` | `crates/outl-actions/src/sync.rs` |
| Start the transport's background tasks once the caller's channel is ready | `SyncEngine::start_transport(tx)` | `crates/outl-actions/src/sync.rs` |
| Announce new local ops to connected peers (no-op for file transport) | `SyncEngine::announce_local_ops(workspace_id, hlc)` | `crates/outl-actions/src/sync.rs` |
| Reload workspace from disk after a peer change | `SyncEngine::reload_workspace` | `crates/outl-actions/src/sync.rs` |
| Re-project a page's `.md` + sidecar to disk / reload + reproject in one call | `SyncEngine::reproject_page` / `refresh_page` | `crates/outl-actions/src/sync.rs` |
| Snapshot every / peer-only `ops-*.jsonl` (size + mtime) for change detection | `SyncEngine::snapshot` / `snapshot_peers` (`OpsFileSnapshot`) | `crates/outl-actions/src/sync.rs` |
| Scan `journals/` + `pages/` for orphan `.md` (no sidecar / stale hash) | `SyncEngine::scan_for_orphans` | `crates/outl-actions/src/sync.rs` |
| Detect projections that ran **ahead of the op log** (sidecar hash-in-sync but referencing ids no op log ever created — e.g. app killed after writing `.md`+sidecar but before the ops append) | `outl_actions::scan_for_desynced_projections(ws, root)` / `SyncEngine::scan_for_desynced_projections(ws)` | `crates/outl-actions/src/desync.rs` |
| Recover a desynced projection: re-emit `Create`/`Edit`/`SetProp` ops for the sidecar ids the tree has never seen (ids preserved, strictly additive — never resurrects a trashed block, never touches existing ones), then re-project the merged page | `outl_actions::recover_desynced_projection(ws, hlc, root, md_path)` | `crates/outl-actions/src/desync.rs` |
| Transport abstraction (iroh QUIC default; file/iCloud polling opt-in) | `outl_actions::SyncTransport` (trait) | `crates/outl-actions/src/sync.rs` |
| Filesystem / iCloud opt-in transport (polls `ops/` every 2 s, delivery is no-op) | `outl_actions::FileSyncTransport` | `crates/outl-actions/src/sync.rs` |
| Per-peer reachability snapshot from the running transport's own dials (GUI status; never bind a probe endpoint) | `SyncTransport::peer_health` → `outl_actions::PeerHealthSnapshot` | `crates/outl-actions/src/sync.rs` |
| Acquire the cross-process workspace lock (one writer at a time) | `outl_core::WorkspaceLock::acquire` | `crates/outl-core/src/lock.rs` |
| Acquire the per-actor write lock (one process writing this actor's jsonl) | `outl_core::ActorWriteLock::try_acquire` | `crates/outl-core/src/lock.rs` |
| Resolve which actor this process writes as | `outl_core::resolve_write_actor` | `crates/outl-core/src/lock.rs` |
| The `Storage` trait every persistent backend implements (invariant #5) | `outl_core::Storage` / `StorageError` | `crates/outl-core/src/storage/mod.rs` |

## 15. Undo / redo history (outl-actions::history)

Bounded snapshot stacks with vim semantics (a new edit clears redo) shared by GUI clients — the desktop's `Cmd+Z` / `Cmd+Shift+Z` ride these.
Restores route through `outl_md::reconcile_md`, so an undo is **new ops in the log**, never a rewrite (invariant #1 holds).
This is *not* per-keystroke undo inside an uncommitted draft — that belongs to the client's editor widget.

| Intent | Use this | File |
|---|---|---|
| Bounded undo / redo stacks over any snapshot type (`record` / `undo` / `redo` / `can_undo` / `can_redo` / `clear`) | `outl_actions::history::HistoryStacks` | `crates/outl-actions/src/history.rs` |
| Default per-stack bound (matches the TUI's session cap) | `outl_actions::DEFAULT_HISTORY_CAP` | `crates/outl-actions/src/history.rs` |
| Restore a page to a previously-rendered `.md` snapshot (write + reconcile → min ops through `Workspace::apply`) | `outl_actions::restore_page_md` | `crates/outl-actions/src/history.rs` |

---

## 16. Templates

| Intent | Use this | File |
|---|---|---|
| List all template pages (any page with a non-empty `template::` property), sorted by name (each entry flags `duplicate` when another page shares its name) | `outl_actions::list_templates` → `TemplateEntry` | `crates/outl-actions/src/template/list.rs` |
| Resolve the page node for a `template:: <name>` (first in tree order; `tracing::warn!` on a name collision) | `outl_actions::template::list::find_template_by_name` | `crates/outl-actions/src/template/list.rs` |
| Instantiate (deep-copy) a structural template's subtree under a target block, with `{{token}}` substitution and `from-template::` traceability on each root clone | `outl_actions::instantiate_template` | `crates/outl-actions/src/template/instantiate.rs` |
| Resolve a callable template's code block (language, source, declared `params::`) | `outl_actions::resolve_call` → `CallResolution` | `crates/outl-actions/src/template/call.rs` |
| Parse a ` ```call:<name> ` block's `key: value` body into params | `outl_actions::parse_call_params` | `crates/outl-actions/src/template/call.rs` |
| The template name invoked by a ` ```call:<name> ` fence (inverse of the exec path's fence read; drives the backlinks traceability match) | `outl_actions::call_target_name` | `crates/outl-actions/src/template/call.rs` |
| Inject a `params` binding into a callable template's source (serde_json-escaped, language canonicalized via `outl_md::lang::canonical`, so quotes/newlines in a value can't break or inject into the generated program) | `outl_actions::inject_call_params` | `crates/outl-actions/src/template/call.rs` |
| Detect + parse a ` ```call:<name> ` block into `(name, params)` — the shared "is this a call invocation?" check every client uses before running normal exec | `outl_actions::parse_call_invocation` | `crates/outl-actions/src/template/run.rs` |
| Execute a callable template (resolve → inject params → run via a client `RuntimeRegistry` → write the `> **result:**` subtree). The single owner every client wraps for `call:` execution | `outl_actions::run_callable_block` | `crates/outl-actions/src/template/run.rs` |
| Template property key constant | `outl_actions::TEMPLATE_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Traceability property key constant (set on structural-instance root blocks) | `outl_actions::FROM_TEMPLATE_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Callable params key constant | `outl_actions::PARAMS_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Reserved template name for the daily journal body — a page with `template:: journal` is auto-instantiated (untraced) into every fresh daily note | `outl_actions::JOURNAL_TEMPLATE_NAME` | `crates/outl-actions/src/template/mod.rs` |

---

## When your need isn't in this catalog

If you've grepped honestly and the primitive doesn't exist, that's a fair sign — add it in the upstream crate that owns the concept:

- **`outl-md`** for parse / render / sidecar / inline / tokenizers
- **`outl-actions`** for workspace mutations, ingest, page/journal helpers
- **`outl-core`** for op-log / tree / HLC / storage trait

Then update this catalog **in the same commit**, and sync the mirror at `.github/copilot-instructions.md` §5.1.
The `PostToolUse` hook will flag drift, but the discipline starts before the hook fires.

For the broader reuse-first rule and past drift incidents that justify this catalog, see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).
