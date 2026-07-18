# CLAUDE.md — outl-tauri-shared

Shared Tauri backend for the two GUI clients (`outl-desktop/src-tauri`, `outl-mobile/src-tauri`).
Before this crate existed, both clients kept near-identical copies of the same nine files and every fix was ported by hand twice — that drift is the bug this crate deletes.

## What lives here

| Module | Owns |
|---|---|
| `state.rs` | Wire DTOs every command returns: `PageView` (includes `backlinks_order: outl_config::BacklinksOrder`, serialized `"newest"`/`"oldest"`, so a client knows the current direction without a separate settings read; **`backlinks` now always comes back empty from the open commands** — see `BacklinksReply`), `BacklinksReply` (the lazy `page_backlinks` reply: `backlinks` + `backlinks_order`, fetched off the page-open path because `backlinks_for_page` is an O(blocks) scan that used to block the first journal paint), `CreateBlockReply`, `WorkspaceSummary`, `BlockHit` (a `((…))` block-ref autocomplete hit — `handle` + `text` + `source_slug`), `ERR_LOADING` |
| `host.rs` | `AppHost` + `StorageRootProvider` — the two traits that absorb the one real client divergence (desktop storage root is `Arc<Mutex<Option<PathBuf>>>`, mobile is a plain `PathBuf`) |
| `helpers.rs` | `parse_node_id`, `parse_date`, `with_ws*`, `build_page_view` (**does NOT compute backlinks** — it's on the first-paint AND post-mutation path, and `backlinks_for_page` is O(blocks); returns `backlinks: []`), `compute_backlinks` (the lazy counterpart, behind `page_backlinks`), `finish_in_page*`, `storage_root_or_err` |
| `commands/` | The command *bodies* (`block`, `page`, `peers`, `plugin`, `exec`) — generic over `S: AppHost`. `commands/page.rs` owns page navigation, search, `delete_page` (calls `outl_actions::delete_page` + `remove_page_projection`, returns today's-journal `PageView` so the caller navigates away from the deleted slug), `page_backlinks(slug)` (the lazy backlinks fetch the frontend fires after the outline paints, returning `BacklinksReply`), and `set_backlinks_order(order, slug)` (persists `[display] backlinks_order` via `outl_config::save` and returns the re-sorted `BacklinksReply`, issue #142) |
| `workspace_open.rs` | `resolve_storage_root` / `reconcile_orphan_md` primitives |
| `iroh_sync.rs` | `build_iroh_transport` / `start_with_reload_bridge` |
| `plugin_service.rs` + `plugin_thread.rs` | `PluginService` — the dedicated plugin thread (Boa `Context` is `!Send`), parametrized by client id + capability set + `StorageRootProvider` |
| `plugin_dto.rs` | Plugin wire shapes (`PluginCommandDto`, `ToolbarButtonDto`, …) |

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
- Never change a DTO shape without checking the TS side (`@outl/shared/api/types`) — the frontends depend on the wire format.
- Client identity (the `CLIENT` str + capability set) stays in each client's `plugin_service.rs` shim, never here.
