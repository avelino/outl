# CLAUDE.md — outl-tauri-shared

Shared Tauri backend for the two GUI clients (`outl-desktop/src-tauri`, `outl-mobile/src-tauri`).
Before this crate existed, both clients kept near-identical copies of the same nine files and every fix was ported by hand twice — that drift is the bug this crate deletes.

## What lives here

| Module | Owns |
|---|---|
| `state.rs` | Wire DTOs every command returns: `PageView` (includes `backlinks_order: outl_config::BacklinksOrder`, serialized `"newest"`/`"oldest"`, so a client knows the current direction without a separate settings read; **`backlinks` now always comes back empty from the open commands** — see `BacklinksReply`), `BacklinksReply` (the lazy `page_backlinks` reply: `backlinks` + `backlinks_order`, fetched off the page-open path because `backlinks_for_page` is an O(blocks) scan that used to block the first journal paint), `CreateBlockReply`, `WorkspaceSummary`, `BlockHit` (a `((…))` block-ref autocomplete hit — `handle` + `text` + `source_slug`), `ERR_LOADING` |
| `host.rs` | `AppHost` + `StorageRootProvider` — the two traits that absorb the one real client divergence (desktop storage root is `Arc<Mutex<Option<PathBuf>>>`, mobile is a plain `PathBuf`). `AppHost::backlink_index() -> Option<Arc<Mutex<Option<BacklinkIndex>>>>` is the client's pre-computed backlinks index slot (default `None`); `Some` lets `page_backlinks` serve `O(refs)` lookups and rebuild the `O(blocks)` index only when it's stale instead of re-scanning the workspace every navigation. `AppHost::projection_writer() -> Option<&ProjectionWriter>` (default `None`) is the client's off-thread `.md`+sidecar writer slot — see `projection.rs` below and "Async projection writes" |
| `helpers.rs` | `parse_node_id`, `parse_date`, `with_ws*`, `build_page_view` (**does NOT compute backlinks** — it's on the first-paint AND post-mutation path, and `backlinks_for_page` is O(blocks); returns `backlinks: []`), `build_page_view_from_tree(workspace, page_id) -> Result<PageView, ActionError>` (projects the view straight from the in-memory tree via `outl_actions::project_outline`, no disk read; `warnings` always empty — feeds the async-projection commit path, see below), `invalidate_backlink_index` (drops the host's cached index so the next `page_backlinks` rebuilds it — called from `finish_in_page*` after every local mutation), `finish_in_page*` (see "Async projection writes" below), `storage_root_or_err`. There is no `compute_backlinks` here anymore — building the index from the in-memory `Workspace` (materializing every block's text under the workspace lock) was the freeze; the rebuild now happens in `commands/page.rs::compute_backlinks_offloaded`, straight from disk. |
| `projection.rs` | `ProjectionWriter` — a single background worker thread that serializes every `.md`+sidecar projection write and coalesces bursts (drains its queue into a dedup set, re-renders each queued page from the current tree via `apply_page_md_with_sidecar`). `spawn<R: StorageRootProvider>(workspace, root) -> Self` starts the thread; `queue(&self, page: NodeId)` enqueues a page (best-effort, never blocks the caller). Every write happens under the workspace lock, same as every synchronous projection path, so `.md` and sidecar can never interleave with another writer — no torn pair, no sync corruption. A crash with queued writes leaves the `.md` briefly behind the op log, never a data loss: the op log is truth, next boot re-projects via `apply_page_md_with_sidecar_if_stale` + the orphan scanner, and peers sync ops over iroh, never the `.md`. Exported at the crate root as `outl_tauri_shared::ProjectionWriter`. |
| `commands/` | The command *bodies* (`block`, `page`, `peers`, `plugin`, `exec`) — generic over `S: AppHost`. `commands/block.rs` also owns `split_block(page_id, id, char_offset)` (splits a block at the caret via `outl_actions::split_block`; `char_offset` is a codepoint offset the client converts from the textarea's UTF-16 `selectionStart`; tolerates a stale anchor exactly like `create_block`, degrading to an empty sibling via `create_after_or_append`). `commands/page.rs` owns page navigation, search, `delete_page` (calls `outl_actions::delete_page` + `remove_page_projection`, returns today's-journal `PageView` so the caller navigates away from the deleted slug), `page_backlinks(slug)` (the lazy backlinks fetch the frontend fires after the outline paints: `compute_backlinks_offloaded` runs three phases off the IPC thread — a brief workspace lock for `list_pages` + this page's meta, an `O(blocks)` rebuild via `outl_actions::build_backlink_index_from_disk` when the host's `backlink_index()` slot is stale (reads the `.md` projection, touches no `Workspace`, holds **no** lock; when the host has no slot at all this is a one-shot from-disk build instead of the old direct workspace scan), then an `O(refs)` lookup under a brief lock, shipping each hit through `Backlink::into_shallow` to trim the IPC payload; returns `BacklinksReply`), and `set_backlinks_order(order, slug)` (persists `[display] backlinks_order` via `outl_config::save` and returns the re-sorted `BacklinksReply`, issue #142) |
| `workspace_open.rs` | `resolve_storage_root` / `reconcile_orphan_md` primitives |
| `iroh_sync.rs` | `build_iroh_transport` / `start_with_reload_bridge` |
| `plugin_service.rs` + `plugin_thread.rs` | `PluginService` — the dedicated plugin thread (Boa `Context` is `!Send`), parametrized by client id + capability set + `StorageRootProvider` |
| `plugin_dto.rs` | Plugin wire shapes (`PluginCommandDto`, `ToolbarButtonDto`, …) |

## Async projection writes

`finish_in_page_with` (the tail every mutating command calls to build its reply) branches on `state.projection_writer()`:

- **`Some(writer)` (async path, both GUI clients today):** `writer.queue(page)` hands the page off to the background thread.
  The reply's `PageView` is built straight from the tree via `helpers::build_page_view_from_tree` — no disk read, no render on the IPC thread.
- **`None` (sync fallback):** the page is projected inline via `apply_page_md_with_sidecar_rendered` and the reply reads the view back from the freshly written `.md`.

Either way, the undo snapshot (`HistoryStacks`) still renders the pre/post `.md` under the workspace lock — that render is needed for the diff regardless of who writes the projection to disk.

A client that wires `AppHost::projection_writer()` to `Some` **must** spawn the `ProjectionWriter` at boot with the same `Arc<Mutex<Option<Workspace>>>` every command locks, or the queued writes race a different workspace instance.
`tests/projection_view.rs` asserts the tree-built view and the `.md`-built view agree — if you change either path, keep both in sync.

## What this crate does NOT own

- The `AppState` structs (fields differ per client) — each client implements `AppHost` on its own state.
- `#[tauri::command]` fns — Tauri's `generate_handler!` needs concrete fns in the app crate, so each client registers 1–3 line wrappers that delegate here.
  **The body always lives here; the wrapper never grows logic.**
- Client-specific surface: desktop `settings.rs` / `fs_watcher.rs` / undo-history invalidation; mobile `bg_sync.rs` / `workspace_picker.rs`.
- Business logic.
  Everything that mutates the workspace shape delegates to `outl-actions` — same hard rule as the client crates.

## Rules

- Adding a Tauri command that both clients need: body here (generic over `AppHost`), thin wrapper + `invoke_handler!` entry in **both** clients.
  A command registered in only one client is drift — the exact failure mode this crate exists to prevent.
- A client that wires `AppHost::backlink_index()` must call `helpers::invalidate_backlink_index` after **every** path that can change what a page's backlinks are.
  That's local mutation (`finish_in_page*` already does this), a peer/workspace reload (`reload_workspace`, desktop's `set_workspace`), and a plugin run that applied ops (`commands/plugin.rs::run` / `sync_hooks`, guarded on `applied > 0`).
  Missing one of these serves stale backlinks until the next unrelated invalidation happens to fire.
- Never change a DTO shape without checking the TS side (`@outl/shared/api/types`) — the frontends depend on the wire format.
- Client identity (the `CLIENT` str + capability set) stays in each client's `plugin_service.rs` shim, never here.
