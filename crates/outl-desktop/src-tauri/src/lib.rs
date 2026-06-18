//! outl-desktop — Tauri 2 desktop client.
//!
//! Thin glue layer between the Solid frontend and `outl-actions`.
//! Same architectural rule as `outl-mobile` and `outl-tui`: this
//! crate adds zero business logic — every workspace mutation is a
//! shim that delegates to `outl-actions`.
//!
//! ## Lifecycle
//!
//! 1. Boot: load (or generate) the device's `ActorId`, load
//!    `settings.json`, start the Tauri runtime.
//! 2. If `settings.last_workspace` points at a valid directory, open
//!    it on a background thread and emit `workspace-ready` when done.
//! 3. Otherwise the frontend renders the `WorkspacePicker` and the
//!    user picks a directory; the resulting Tauri call to
//!    `set_workspace` opens the workspace, persists the path in
//!    settings, and emits `workspace-ready`.
//! 4. After `workspace-ready` the frontend calls `open_today_journal`
//!    and renders the outline.
//!
//! The `ActorId` is generated once per device and lives in
//! `<app_config_dir>/actor` — switching workspaces does *not* rotate
//! the actor (actors identify devices, not workspaces). The op log
//! within each workspace tracks `ops-<actor>.jsonl` per device.
//!
//! ## Module map
//!
//! - `settings` — JSON IO for user preferences + `last_workspace`.
//! - `state` — `AppState`, wire types (`PageView`, `WorkspaceSummary`).
//! - `helpers` — argument parsers, workspace-lock acquisition,
//!   `finish_in_page` (the mutation funnel).
//! - `workspace_open` — open / reconcile / boot opener primitives.
//! - `commands` — Tauri command surface split by responsibility
//!   (`workspace`, `page`, `block`).

mod commands;
mod fs_watcher;
mod helpers;
mod settings;
mod state;
mod workspace_open;

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::Manager;

use crate::commands::{
    copy_block_markdown, create_block, current_workspace, date_title, delete_block, edit_block,
    get_settings, get_theme, indent_block, list_all_pages, list_shortcut_bindings, list_themes,
    move_block_after, move_block_down, move_block_up, next_day, open_journal_for,
    open_page_by_slug, open_ref, open_today_journal, outdent_block, outl_emoji_search,
    paste_block_after, paste_markdown_at, previous_day, redo_page, reload_workspace, resolve_ref,
    run_code_block, search_pages, search_persons, set_block_collapsed, set_workspace,
    today_slug_cmd, toggle_quote, toggle_todo, undo_page, update_settings, workspace_stats,
};
use crate::state::AppState;
use crate::workspace_open::{load_or_create_actor, spawn_workspace_opener};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            // Local-only state (the per-device `actor` ULID and the
            // `config.toml`) lives at `~/.config/outl/` — the XDG
            // path the TUI also reads, so the two clients share a
            // single source of truth. Tauri's
            // `app.path().app_config_dir()` (which would resolve to
            // `~/Library/Application Support/app.outl.desktop/` on
            // macOS) is deliberately ignored here.
            let app_config_dir = outl_config::config_dir();
            std::fs::create_dir_all(&app_config_dir)?;

            let actor = load_or_create_actor(&app_config_dir)?;
            let hlc = HlcGenerator::new(actor);

            let settings = settings::load(&app_config_dir);
            let last_workspace = settings.last_workspace.clone();

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));
            let storage_root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

            let registry = Arc::new(RuntimeRegistry::with_builtins());
            let fs_watcher = Arc::new(Mutex::new(None));

            if let Some(path) = last_workspace {
                spawn_workspace_opener(
                    workspace.clone(),
                    storage_root.clone(),
                    fs_watcher.clone(),
                    path,
                    hlc.clone(),
                    app.handle().clone(),
                );
            }

            app.manage(AppState {
                workspace,
                storage_root,
                hlc,
                settings: Arc::new(Mutex::new(settings)),
                app_config_dir,
                registry,
                fs_watcher,
                history: Mutex::new(std::collections::HashMap::new()),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Workspace lifecycle
            set_workspace,
            current_workspace,
            workspace_stats,
            reload_workspace,
            // Settings
            get_settings,
            update_settings,
            // Theme
            list_themes,
            get_theme,
            // Shortcuts
            list_shortcut_bindings,
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
            resolve_ref,
            // Block mutations
            create_block,
            edit_block,
            toggle_todo,
            toggle_quote,
            delete_block,
            indent_block,
            outdent_block,
            move_block_up,
            move_block_down,
            move_block_after,
            copy_block_markdown,
            paste_block_after,
            set_block_collapsed,
            paste_markdown_at,
            // Undo / redo
            undo_page,
            redo_page,
            // Code execution
            run_code_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running outl-desktop application");
}
