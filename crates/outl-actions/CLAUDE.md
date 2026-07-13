# CLAUDE.md — outl-actions

The **UI-agnostic** workspace operations layer.
Every outl client (`outl-tui`, `outl-mobile`, future Tauri desktop) consumes this crate so we never duplicate edit / indent / toggle / journal-render logic.

If you add a workspace operation that two or more clients need, **it belongs here**, not in the binary that asked for it first.

## Layering

```text
outl-core           (CRDT, op log, storage trait)
   ↑
outl-md             (.md parse/render, sidecar, matching)
   ↑
outl-actions        ← you are here
   ↑
outl-cli / outl-tui / outl-mobile / future clients
```

## Public surface

> The **canonical reuse index** for the whole workspace is the ["Shared primitives catalog" in the root `CLAUDE.md`](../../CLAUDE.md#shared-primitives-catalog) (mirrored at [`.github/copilot-instructions.md`](../../.github/copilot-instructions.md) §5.1).
> The table below describes this crate's surface in module-by-module detail; the root catalog is the "intent → use this" cross-crate index you should grep first when adding any helper.

| Module      | What it owns                                                                 |
|-------------|-------------------------------------------------------------------------------|
| `block`     | `append_block`, `create_before`, `create_after`, `create_under`, `edit_text`, `toggle_todo`, `toggle_quote`, `delete`, `indent`, `outdent`, `move_up`, `move_down`, `move_under` (re-parent under an arbitrary page/block — the cross-page move the plugin host applies for `Move` intents) |
| `collapsed` | `set_block_collapsed`, `toggle_block_collapsed`. Both generate `Op::SetCollapsed` and route it through `Workspace::apply`, so the fold flag converges between devices on top of the existing per-actor jsonl + HLC infrastructure. **Never** write fold state to the sidecar — that's last-write-wins per file under iCloud and loses concurrent flips. See the root `CLAUDE.md` invariants. |
| `tree`      | Read-only helper: `children_of`. Sibling / fractional-position helpers (`previous_sibling`, `next_sibling`, `position_before`, `position_after`, `position_for_new_last_child`) — `position_before` and `position_after` are `pub`; the rest remain `pub(crate)` until a real caller asks. |
| `exec`      | `run_code_block`, `ExecOutputDto`, `RunCodeBlockOutcome`. Shared "run a fence" orchestration: walks DFS for the block's flat index (`outline::flat_index_for_block`), resolves the page's `.md` path via `journal::page_md_path`, calls `outl_exec::run_block_at_index`, returns a Serde-friendly outcome. Every client (TUI, desktop, mobile) wraps **this** function instead of re-implementing the flow. The runtime catalog is selected by the consuming binary's `outl-exec` features — `outl-actions` declares the dep with `default-features = false` so the mobile IPA never picks up `wasmtime` by accident. |
| `todo`      | `TodoState`, `split_todo`, `cycle_todo` — TODO/DONE encoded as text prefix     |
| `quote`     | `QUOTE_PREFIX`, `is_quote`, `split_quote`, `toggle_quote` — CommonMark `"> "` prefix encoding a per-block blockquote marker. Same wire-format policy as `todo` (text prefix, no AST field, every client renders its own visual). |
| `outline`   | `OutlineNode` DTO + `project_outline` — UI-friendly tree projection. Also owns `flatten_subtree_paths` and `flat_index_for_block` (the DFS index used by `exec::run_code_block`). **`PageOutline { nodes, warnings }`** + `read_page_outline` / `read_page_outline_with_workspace` bundle the outline with parser recovery records (`outl_md::ParseWarning`) so every client surface (banner, status line, doctor) can warn the user that their `.md` has lines outside the dialect — without re-parsing the file. The legacy `read_page_view*` shims silently drop warnings for back-compat. |
| `page`      | `PageMeta` (`id`, `slug`, `title`, `kind`, `icon`, **`pinned`** — surfaced from the `pinned::` page property so every client that consumes `list_all` sees the flag without re-querying the workspace index), `PageKind` (Page / Journal), `open_or_create`, `open_journal`, `open_today`, `today` (delegates to `clock`), `find_by_slug`, `list_all`, `migrate_legacy_into_today`, `page_id_from_slug` (deterministic ID derivation so two peers agree on a fresh page's NodeId), **`delete(ws, hlc, slug) -> Result<PageMeta, ActionError>`** (moves the page root to `NodeId::trash()` via a single `Op::Move`; the whole subtree travels with it; returns the meta so callers can drop projections + navigate away). **`merge_duplicate_slug_roots(ws, hlc) -> Result<usize, ActionError>`** (split-brain repair: when >1 root shares a slug, re-parents every child under the canonical root — `page_id_from_slug` id if present, else most-descendants / smallest-id — and trashes the emptied duplicates via `Op`s, so it converges on every device; idempotent, returns the count merged. Impl lives in the sibling `page_merge` module, re-exported through `page` + the crate root. Clients call it on boot alongside `migrate_legacy_into_today`). `open_or_create` creates the root with **no text** and, when `title != slug`, stores the title in the **`title::` property** (`TITLE_KEY`, `Op::SetProp`, last-write-wins by HLC) instead of the root's Yrs text — two devices minting the same deterministic root offline used to run concurrent Yrs text inserts that concatenated (`"2026-06-252026-06-25"`); a property converges to one value instead. Journals (`journal_title == slug`) never get a `title::` property — `page_meta` falls back to the slug, so a journal's `.md` stays title-line-free. Regular pages created in-app now render a `title:: <title>` line at the top of their `.md`. Journal date **labels** live in `dates`; user-typed name/ref **resolution** lives in `resolve` |
| `page_repair_titles` | `repair_doubled_journal_titles(ws, hlc) -> Result<usize, ActionError>` — repairs journal roots corrupted by the pre-`title::` concurrent-create bug above: any journal whose root text is its slug repeated k >= 2 times gets that text cleared via `edit_text` (an `Op`, so the fix converges to every device through the op log). Idempotent (`0` on a clean workspace), journal-only (a regular page's title is never a slug repetition by construction). Sibling of `page_merge` — same "background-pass repair, not boot-path" pattern. Clients run it on their **background** reconcile pass (desktop: `spawn_background_reconcile`; mobile: after `reconcile_orphan_md` in `spawn_workspace_opener`), never the synchronous boot path, since it scales with page count. |
| `resolve`   | `open_or_create_by_name` (slugifies a human-typed name + keeps it as the title — drives `[[ref]]`/`#tag` click handlers in TUI + mobile), `open_or_create_by_ref` (the canonical "user tapped a ref" decision tree — date → journal, `@` mention → person, else the shared ladder), `resolve_or_create_by_name` (`pub(crate)` — literal slug → slugified → case-insensitive title → create; consumed by both `open_or_create_by_ref` and `person::ensure_person_by_name` so the two resolution paths cannot drift) |
| `deeplink`  | `parse_deep_link`, `DeepLinkTarget` (`Today` / `Daily(NaiveDate)` / `Page(slug)`), `DeepLinkError`, `DEEP_LINK_SCHEME`. Pure `outl://` URL → navigation-target parser shared by the desktop + mobile deep-link handlers. Validates ISO dates via `chrono` and page slugs per-segment via `page::is_valid_slug` (rejects `..` / control chars / empty segments, allows `/` nesting). Touches no `Workspace`, no storage, no Tauri type — each client maps the parsed target onto its own `open_*` command. One parser, every client; never reparse `outl://` per client. |
| `backlinks` | `Backlink`, `backlinks_for_target`, `backlinks_for_page` (a mention is a literal `[[target]]` **or** a `#tag` whose slug form resolves to the page — the same `slugify` rule a tag click goes through, so navigation and "Linked from" can't drift), `extract_refs` (parse `[[ref]]` tokens) |
| `backlinks_sort` | `sort_backlinks(links: &mut [Backlink], newest_first: bool)` — group-stable chronological order for a backlinks list (issue #142): each source page's blocks stay contiguous, pages sort by their most-recently-referenced block (`block_id` is a ULID, so lexicographic order tracks creation time), blocks within a page keep DFS order. Driven by `[display] backlinks_order` in `outl-config`; every client (TUI, desktop, mobile) calls this one function so the direction can't drift. |
| `journal`   | `render_page_md`, `apply_page_md`, `apply_page_md_with_sidecar`, `apply_page_md_with_sidecar_if_absent` (lazy: projects only when `.md` is absent — use on read paths to avoid sidecar churn), **`apply_page_md_with_sidecar_if_stale`** (the re-projection counterpart: projects when the `.md` is absent OR the tree has moved past a *faithful* projection — a peer's ops landed but the `.md` the view reads was never refreshed, issue #166; a no-op on an in-sync page, and never clobbers a `.md` whose hash no longer matches its sidecar since that is a pending external edit the `.md → tree` reconcile owns. Every GUI open path — `open_journal_for`, `open_today_journal`, `open_page_by_slug`, `open_ref` — calls this before `build_page_view`, which reads the `.md`), `apply_all_pages_md`, `mutate_page_md`, `journals_dir`, `pages_dir`, `page_md_path`, `write_md_atomic`, **`remove_page_projection(root, meta) -> io::Result<()>`** (the inverse of `apply_page_md_with_sidecar` — removes the page's `.md` and `.outl` from disk; idempotent on missing files; pairs with `page::delete` so a client can drop the projection after the trash op lands) |
| `history`   | `HistoryStacks<T>` (bounded undo / redo stacks, vim semantics: a new edit clears redo), `DEFAULT_HISTORY_CAP`, `restore_page_md` (write a previously-rendered `.md` snapshot + reconcile it back — the restore is new ops through `Workspace::apply`, never a log rewrite). Drives the desktop's `Cmd+Z` / `Cmd+Shift+Z`; per-keystroke undo inside an uncommitted draft stays in the client's editor widget. |
| `desync`    | `scan_for_desynced_projections`, `recover_desynced_projection`. Detection + repair for projections that ran **ahead of the op log**: `.md` + sidecar written but the ops append lost (app killed mid-commit), so the sidecar is hash-in-sync while its ids exist in no op log — the state the hash gate in `sync` is structurally blind to. Recovery is strictly additive: recreates the missing blocks preserving the sidecar ids (ref handles keep resolving), never touches blocks the tree knows (a trashed block is **not** resurrected — a remote delete IS an op), then re-projects the merged page. Wired into the GUI boot via `outl-tauri-shared::workspace_open::reconcile_orphan_md`. |
| `sync`      | `SyncEngine`, `OpsFileSnapshot`, `SyncTransport`, `FileSyncTransport`. Reload workspace from disk, re-project a page's `.md` + sidecar, snapshot peer jsonls (skipping own), scan for orphan `.md` files (no sidecar / stale hash); `SyncEngine::scan_for_desynced_projections(ws)` is the workspace-aware companion scan (see `desync`). Shared by TUI poller + mobile iCloud watcher. **`SyncTransport`** abstracts *how* ops travel between devices (iroh QUIC is the default transport; `FileSyncTransport` is the opt-in filesystem/iCloud polling alternative) — both end up writing `ops-<peer>.jsonl` to disk, so `reload_workspace` is transport-agnostic. `SyncEngine::with_transport` binds one in; `start_transport` spawns its background tasks; `announce_local_ops` is the post-commit hook (no-op for files, gossip for iroh). |
| `paste`     | `paste_markdown`, `paste_plain`, `PasteAnchor`, `PasteOutcome`, `normalize_external_syntax`. `paste_markdown` converts external clipboard markdown (Roam `{{[[TODO]]}}`, GitHub `[ ]/[x]`, Logseq `id::`, 4-space indent; multi-paragraph plain text → one block per paragraph) into outl syntax and grafts the bullet structure as blocks. `paste_plain` inserts raw text as a single block at the anchor with no normalisation or paragraph splitting — the "without formatting" path. Drives `Event::Paste` / `Event::PastePlain` in the TUI and the `paste_markdown_at` / `paste_plain_at` Tauri commands. |
| `clipboard` | `copy_markdown`. The **inverse** of `paste`: serializes a block selection (each root + its full subtree) to clean canonical outl markdown for the OS clipboard — `- ` bullets, 2-space indent, inline block props (alpha-sorted), `TODO`/`DONE`/`> ` prefixes verbatim. Copy-out then paste-in reconstructs the same tree (tested as a pair against `paste_markdown`). Core emits **only** the canonical format; other output formats are the domain of optional format plugins (see `docs/design/clipboard.md`). |
| `dates`     | The pure date domain. `parse_flexible_date`, `parse_date_label` — the one owner of "human-typed date → `NaiveDate` / ISO label" (`April 22nd, 2026`, `Sept 3rd, 2025`, `2026/04/22`, `22/04/2026`, `22 April 2026`, ISO); used by `paste::normalize` for `[[date]]` rewriting and by the CLI (`daily`, `import`, Obsidian frontmatter). `parse_date_arg` layers relative offsets (`+3d`, `-2w`, `+1m`, bare `5d`) on top for slash-command / CLI arguments. Journal labels: `journal_slug`, `journal_title`, `journal_ref` (`[[YYYY-MM-DD]]`), `date_from_slug`, `previous_journal_date`, `next_journal_date`. Week arithmetic: `week_tag` (`#YYYY-Www`, ISO `%G`), `days_until_next_weekday` (same weekday → 7, never 0). Everything pure, chrono-validated (`February 30th` is not a date); no clock — functions take the anchor date as a parameter, and keyword shortcuts (`today` / `yesterday`) stay in the caller because "what does today mean" belongs to `clock`. |
| `clock`     | `init`, `now_local`, `today` — process-wide "now"/"today" in the user's configured timezone (`[calendar] timezone`, DST-aware via `chrono-tz`; OS local when unset). A client calls `init(tz)` once at boot; every "today" goes through here (`page::today` delegates) so the journal date + status-line clock honour the configured zone instead of trusting `chrono::Local`, which reads UTC inside containers / Crostini (issue #107). |
| `error`     | `ActionError` (incl. `PageNotFound(String)` returned by `page::delete` when a slug doesn't resolve)                                                                  |
| `template`  | Template engine — a page with a non-empty `template::` property is a template; its outline is the body. Split across `list.rs` (`list_templates`, `TemplateEntry`), `vars.rs` (`{{date}}`/`{{today}}`/`{{yesterday}}`/`{{tomorrow}}`/`{{page}}`/`{{time}}` substitution, `pub(crate)`), `call.rs` (`resolve_call`, `CallResolution`, `parse_call_params`, `call_target_name`, `inject_call_params`), `instantiate.rs` (`instantiate_template`), `run.rs` (`parse_call_invocation`, `run_callable_block` — the shared "detect + execute a `call:` block" every client wraps; also intercepted inside `exec::run_code_block` so desktop/mobile get `call:` for free). Two invocation modes: **structural** (`/template <name>` deep-copies the subtree, stamping `from-template:: <slug>` on each root clone) and **callable** (a ` ```call:<name> ` fence resolves the template's first code block for execution with params — `inject_call_params` injects a `params` binding via `serde_json`, so a quote/newline in a param value can't break or inject into the generated program; language is canonicalized first via `outl_md::lang::canonical` so aliases like `py`/`node` still get the prelude). `JOURNAL_TEMPLATE_NAME = "journal"` is the reserved name `page::open_journal` auto-instantiates (untraced) into every fresh daily note. `backlinks::backlinks_for_page` surfaces a template page's render/instantiation sites (`from-template::` property or `call:<name>` fence) so the template's backlinks panel lists every place it fired. |

## Contract

Every mutating function:

1. Takes `&mut Workspace` (caller-owned) and `&HlcGenerator` (caller-owned).
2. Reads tree state, computes op parameters, generates a `LogOp` with a fresh HLC.
3. Routes the op through `Workspace::apply` so the op log stays the single source of truth (invariant #1 of `outl-core`).
4. Returns `Result<T, ActionError>` — never panics on user error.

Functions **never**:

- Touch storage directly.
  Storage is `Workspace::apply`'s responsibility.
- Touch the filesystem outside of `journal::write_md_atomic`.
- Hold per-client state (selections, modes, toasts, keymaps).
- Round-trip through `.md` to reconstruct workspace state.
  The op log is the source of truth; `.md` is a projection.

## Page model

Pages are **regular nodes** directly under [`NodeId::root`] tagged with a `page-slug` property.
A `page-kind` property says whether the page is a regular `page` or a date-keyed `journal`.
The node's text is the page's title; its children are the page's blocks.
Keeping pages as ordinary nodes lets the tree CRDT handle move / delete / re-parent for free.

Disk layout when projected to `.md`:

```text
<root>/
├── journals/YYYY-MM-DD.md     ← page-kind = "journal"
├── pages/<slug>.md            ← page-kind = "page"
├── pages/<slug>.outl          ← sidecar (block IDs + hashes)
└── ops/ops-<actor>.jsonl      ← op log, one file per actor
```

`migrate_legacy_into_today` reshuffles any pre-page-model blocks (direct children of root that lack `page-slug`) under today's journal.
Clients call it once on startup; it's idempotent.

## TODO/DONE convention

TODO state lives **in the block's text** as a prefix:

```
"foo"             ← plain block
"TODO foo"        ← open task
"DONE foo"        ← completed task
```

This matches the TUI's existing wire format.
`cycle_todo` walks `None → TODO → DONE → None`.
`edit_text` writes the caller's text **verbatim** — including the prefix — so the user can drop a TODO just by erasing `TODO `/`DONE ` in the editor.
UIs that surface state separately (mobile checkbox) must reattach the prefix before calling `edit_text`; helper `rawTextWithTodo` on the mobile side does this.
The historical "auto-preserve prefix" behaviour was removed because it made `TODO`/`DONE` impossible to delete from the editor.

## What this crate does NOT own

- **UI state.**
  Selections, modes, keymaps, and the undo stack for **in-flight text editing** (per-keystroke history inside an uncommitted draft) live in the clients.
  Committed-mutation undo is different: the bounded snapshot stacks + `.md` restore live here in `history` so every GUI shares one engine.
- **In-flight outline AST.**
  When the user is typing into a buffer that hasn't been parsed yet, the manipulation happens on `Vec<OutlineNode>` via `outl_md::outline_ops` (re-exported through the `outl-tui/src/outline_ops.rs` shim).
  We don't pull that up because it's not workspace-grounded — it's a stage *before* ops exist.
  It lives in `outl-md` because the mobile client also needs it, but no `Workspace` is touched, so it stays out of `outl-actions`.
- **Storage backends.** `JsonlStorage`, future `ChronDbStorage` implement `outl_core::Storage` and live in the binary that needs them.

## Reuse-first

This is the **shared layer**.
Every client (TUI, mobile, future desktop) consumes it — and they all consume the same struct, the same constants, the same policy.
Two parallel implementations of the same concept across clients is the bug we paid to delete,
see the `outl_md::index::Backlink` → `outl_actions::Backlink` consolidation,
where policy drifted on self-references and the user was the one who caught it.

When adding a new operation here:

1. **Search first.** `rg` for the symbol across `outl-core`, `outl-md`, and this crate before writing it.
2. **Promote, don't fork.**
   If a client crate already has a helper for the same concept, lift it here (and delete the client copy) — even if it's a small refactor.
   The `flatten_backlink_subtree` → `flatten_subtree_paths` move from `outl-md` is the canonical pattern: one owner, every client wraps.
3. **Generalize the parameter set** when migrating.
   The Backlink rewrite added `source_block: OutlineNode` + `source_path` so *both* the mobile linear renderer and the TUI subtree renderer could share the same struct.
   Capping features at "what mobile needs today" would force the TUI to keep its own copy.

The root [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations) "Reuse-first" section documents the policy at the workspace level.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-actions -- -D warnings`
3. `cargo test -p outl-actions`
4. If you changed the public API surface, update the table in "Public surface" above and the matching entry in the root `CLAUDE.md`.
