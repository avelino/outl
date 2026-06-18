//! outl-mobile — Tauri 2 mobile companion app.
//!
//! Thin glue layer:
//!
//! - **Storage:** `outl_core::JsonlStorage` writes the op log into the
//!   iCloud Ubiquity Container's `Documents/ops/` directory. Each
//!   device only writes to its own `ops-<actor>.jsonl`; iCloud syncs
//!   the files in for free.
//! - **Actions:** delegated wholesale to `outl-actions` so the TUI,
//!   the desktop, and this mobile app all share the same semantics
//!   for edit / indent / outdent / TODO / delete / move / journal /
//!   page / backlinks.
//! - **Tauri commands:** lightweight wrappers split across
//!   `commands::{workspace, page, block, exec}` that parse `String`
//!   ids, call into `outl-actions`, and return the new outline so the
//!   Solid frontend renders in a single round-trip. The split mirrors
//!   `outl-desktop` 1:1 — same module names, same shape — so a
//!   contributor reading either crate immediately knows the other.
//!
//! ## Async startup
//!
//! The Tauri `setup` callback returns immediately so the WebView
//! starts painting right away. Opening the iCloud workspace (filesystem
//! reads + op-log replay) runs on a background thread (see
//! `workspace_open::spawn_workspace_opener`); commands that need
//! the workspace return a `workspace_loading` error until it's ready,
//! and the frontend retries on a short interval. As soon as the
//! workspace lands, Tauri emits a `workspace-ready` event the frontend
//! can listen for to refresh proactively.

mod commands;
mod helpers;
mod icloud_path;
mod state;
mod workspace_open;

use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::Manager;

use crate::commands::{
    add_block, create_block, date_title, delete_block, edit_block, exec, indent_block,
    list_all_pages, list_outline, move_block_down, move_block_up, next_day, open_journal_for,
    open_page_by_slug, open_ref, open_today_journal, outdent_block, outl_emoji_search,
    paste_markdown_at, previous_day, reload_workspace, resolve_ref, search_pages, search_persons,
    set_block_collapsed, today_slug_cmd, toggle_quote, toggle_todo, workspace_stats,
};
use crate::state::AppState;
use crate::workspace_open::{
    load_or_create_actor, resolve_storage_root, spawn_workspace_opener, workspace_root_in,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let local_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            std::fs::create_dir_all(&local_dir)?;

            let actor = load_or_create_actor(&local_dir)?;
            // `resolve_storage_root` returns the iCloud Ubiquity Container
            // root (the device-local mount). The workspace lives in
            // `Documents/` directly inside the container — the
            // container itself is already the outl namespace, so
            // there's no need for a second `outl/` folder. The TUI
            // is expected to be pointed at this same path via
            // `--path "<container>/Documents"`.
            let container_root = resolve_storage_root(&local_dir);
            let storage_root = workspace_root_in(&container_root);
            std::fs::create_dir_all(&storage_root)?;
            let hlc = HlcGenerator::new(actor);

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));

            spawn_workspace_opener(
                workspace.clone(),
                storage_root.clone(),
                hlc.clone(),
                app.handle().clone(),
            );

            let registry = Arc::new(RuntimeRegistry::with_builtins());

            app.manage(AppState {
                workspace,
                hlc,
                storage_root,
                registry,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Page / journal navigation
            list_all_pages,
            search_pages,
            search_persons,
            outl_emoji_search,
            open_today_journal,
            open_journal_for,
            open_page_by_slug,
            open_ref,
            previous_day,
            next_day,
            today_slug_cmd,
            date_title,
            workspace_stats,
            resolve_ref,
            // Mutations
            create_block,
            edit_block,
            toggle_todo,
            toggle_quote,
            delete_block,
            indent_block,
            outdent_block,
            move_block_up,
            move_block_down,
            set_block_collapsed,
            paste_markdown_at,
            reload_workspace,
            // Code execution
            exec::run_code_block,
            // Legacy
            list_outline,
            add_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
