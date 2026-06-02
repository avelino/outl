# CLAUDE.md — outl-actions

The **UI-agnostic** workspace operations layer. Every outl client
(`outl-tui`, `outl-mobile`, future Tauri desktop) consumes this crate
so we never duplicate edit / indent / toggle / journal-render logic.

If you add a workspace operation that two or more clients need, **it
belongs here**, not in the binary that asked for it first.

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

| Module      | What it owns                                                                 |
|-------------|-------------------------------------------------------------------------------|
| `block`     | `append_block`, `create_after`, `create_under`, `edit_text`, `toggle_todo`, `delete`, `indent`, `outdent`, `move_up`, `move_down` |
| `collapsed` | `set_block_collapsed`, `toggle_block_collapsed`. Both generate `Op::SetCollapsed` and route it through `Workspace::apply`, so the fold flag converges between devices on top of the existing per-actor jsonl + HLC infrastructure. **Never** write fold state to the sidecar — that's last-write-wins per file under iCloud and loses concurrent flips. See the root `CLAUDE.md` invariants. |
| `tree`      | Read-only helper: `children_of`. Sibling / fractional-position helpers (`previous_sibling`, `next_sibling`, `position_after`, `position_for_new_last_child`) are `pub(crate)` — promote them to `pub` when a real caller asks for them. |
| `todo`      | `TodoState`, `split_todo`, `cycle_todo` — TODO/DONE encoded as text prefix     |
| `outline`   | `OutlineNode` DTO + `project_outline` — UI-friendly tree projection            |
| `page`      | `PageMeta`, `PageKind` (Page / Journal), `open_or_create`, `open_journal`, `open_today`, `find_by_slug`, `list_all`, `migrate_legacy_into_today`, `journal_slug`, `journal_title`, `today`, `date_from_slug`, `previous_journal_date`, `next_journal_date`, `page_id_from_slug` (deterministic ID derivation so two peers agree on a fresh page's NodeId) |
| `backlinks` | `Backlink`, `backlinks_for_target`, `backlinks_for_page`, `extract_refs` (parse `[[ref]]` tokens) |
| `journal`   | `render_page_md`, `apply_page_md`, `apply_page_md_with_sidecar`, `apply_all_pages_md`, `mutate_page_md`, `journals_dir`, `pages_dir`, `page_md_path`, `write_md_atomic` |
| `ingest`    | `ingest_md_file(ws, hlc, md_path, orphan_log)`, `ingest_dir(ws, hlc, dir, orphan_log)`, `create_missing_ref_pages(ws, hlc) -> Result<usize, ActionError>`. `ingest_md_file` ingests a `.md` file as a full page: derives slug (filename stem), kind (`journals/` → Journal, else Page), title (`title::` property), calls `open_or_create` to create the page node with a deterministic id (`page_id_from_slug`), then reconciles blocks under that node via `reconcile_md_with_page_id`. `ingest_dir` sweeps all `.md` files in a directory and returns `Vec<(PathBuf, Result<ReconcileReport, ActionError>)>` so callers can log per-file failures without aborting. `create_missing_ref_pages` walks every block in the workspace, collects `[[ref]]` targets, and creates a stub page for each target that has no existing page — title is the raw ref text, slug is `slugify(ref)`, kind is Journal if the slug is a date, Page otherwise. Returns the count of stubs created. Idempotent: a second call is a no-op. Called at the end of `outl import logseq` so backlinks to Logseq implicit pages resolve immediately. **All bulk-ingest call sites** (serve, import, TUI orphan scanner, mobile orphan scanner) use these functions. |
| `sync`      | `SyncEngine`, `OpsFileSnapshot`. Reload workspace from disk, re-project a page's `.md` + sidecar, snapshot peer jsonls (skipping own), scan for orphan `.md` files (no sidecar / stale hash). Shared by TUI poller + mobile iCloud watcher. |
| `paste`     | `paste_markdown`, `PasteAnchor`, `PasteOutcome`, `normalize_external_syntax`. Converts external clipboard markdown (Roam `{{[[TODO]]}}`, GitHub `[ ]/[x]`, Logseq `id::`, 4-space indent) into outl syntax and grafts the bullet structure as blocks. Drives `Event::Paste` in the TUI and the mobile `paste_markdown_at` Tauri command. |
| `error`     | `ActionError`                                                                  |

## Contract

Every mutating function:

1. Takes `&mut Workspace` (caller-owned) and `&HlcGenerator` (caller-owned).
2. Reads tree state, computes op parameters, generates a `LogOp` with
   a fresh HLC.
3. Routes the op through `Workspace::apply` so the op log stays the
   single source of truth (invariant #1 of `outl-core`).
4. Returns `Result<T, ActionError>` — never panics on user error.

Functions **never**:

- Touch storage directly. Storage is `Workspace::apply`'s responsibility.
- Touch the filesystem outside of `journal::write_md_atomic`.
- Hold per-client state (selections, modes, toasts, keymaps).
- Round-trip through `.md` to reconstruct workspace state. The op log
  is the source of truth; `.md` is a projection.

## Page model

Pages are **regular nodes** directly under [`NodeId::root`] tagged
with a `page-slug` property. A `page-kind` property says whether the
page is a regular `page` or a date-keyed `journal`. The node's text
is the page's title; its children are the page's blocks. Keeping
pages as ordinary nodes lets the tree CRDT handle move / delete /
re-parent for free.

Disk layout when projected to `.md`:

```text
<root>/
├── journals/YYYY-MM-DD.md     ← page-kind = "journal"
├── pages/<slug>.md            ← page-kind = "page"
├── pages/<slug>.outl          ← sidecar (block IDs + hashes)
└── ops/ops-<actor>.jsonl      ← op log, one file per actor
```

`migrate_legacy_into_today` reshuffles any pre-page-model blocks
(direct children of root that lack `page-slug`) under today's
journal. Clients call it once on startup; it's idempotent.

## TODO/DONE convention

TODO state lives **in the block's text** as a prefix:

```
"foo"             ← plain block
"TODO foo"        ← open task
"DONE foo"        ← completed task
```

This matches the TUI's existing wire format. `cycle_todo` walks
`None → TODO → DONE → None`. `edit_text` writes the caller's text
**verbatim** — including the prefix — so the user can drop a TODO
just by erasing `TODO `/`DONE ` in the editor. UIs that surface
state separately (mobile checkbox) must reattach the prefix before
calling `edit_text`; helper `rawTextWithTodo` on the mobile side
does this. The historical "auto-preserve prefix" behaviour was
removed because it made `TODO`/`DONE` impossible to delete from
the editor.

## What this crate does NOT own

- **UI state.** Selections, modes, keymaps, undo stack for in-flight
  text editing live in the clients.
- **In-flight outline AST.** When the user is typing into a buffer
  that hasn't been parsed yet, the manipulation happens on
  `Vec<OutlineNode>` via `outl_md::outline_ops` (re-exported through
  the `outl-tui/src/outline_ops.rs` shim). We don't pull that up
  because it's not workspace-grounded — it's a stage *before* ops
  exist. It lives in `outl-md` because the mobile client also needs
  it, but no `Workspace` is touched, so it stays out of `outl-actions`.
- **Storage backends.** `JsonlStorage`, future
  `ChronDbStorage` implement `outl_core::Storage` and live in the
  binary that needs them.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-actions -- -D warnings`
3. `cargo test -p outl-actions`
4. If you changed the public API surface, update the table in
   "Public surface" above and the matching entry in the root
   `CLAUDE.md`.
