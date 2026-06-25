//! outl-mobile — Tauri 2 mobile companion app.
//!
//! Thin glue layer:
//!
//! - **Storage:** `outl_core::JsonlStorage` writes the op log into the
//!   `ops/` directory of a workspace folder **the user picks** — local
//!   by default, optionally inside an iCloud container (see
//!   `workspace_open` and `workspace_picker`). Each device only writes
//!   to its own `ops-<actor>.jsonl`; iroh P2P is the primary sync, and
//!   iCloud (when the chosen folder lives there) syncs the files for
//!   free on top.
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
//! starts painting right away. Opening the workspace (filesystem
//! reads + op-log replay) runs on a background thread (see
//! `workspace_open::spawn_workspace_opener`); commands that need
//! the workspace return a `workspace_loading` error until it's ready,
//! and the frontend retries on a short interval. As soon as the
//! workspace lands, Tauri emits a `workspace-ready` event the frontend
//! can listen for to refresh proactively.

mod commands;
mod helpers;
mod iroh_sync;
mod state;
mod workspace_open;
mod workspace_picker;

use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::Manager;
use tracing::info;

use crate::commands::{
    add_block, create_block, date_title, delete_block, edit_block, exec, indent_block,
    list_all_pages, list_outline, move_block_down, move_block_up, next_day, open_journal_for,
    open_page_by_slug, open_ref, open_today_journal, outdent_block, outl_emoji_search,
    outl_peer_list, outl_peer_pair_host, outl_peer_pair_join, outl_peer_remove, outl_peer_status,
    outl_sync_now, paste_markdown_at, previous_day, reload_workspace, resolve_ref, search_pages,
    search_persons, set_block_collapsed, today_slug_cmd, toggle_quote, toggle_todo,
    workspace_stats,
};
use crate::state::AppState;
use crate::workspace_open::{load_or_create_actor, resolve_storage_root, spawn_workspace_opener};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install a log subscriber FIRST so the iroh P2P transport's
    // `info!`/`warn!`/`debug!` lines reach the device console (on iOS, stderr
    // surfaces in idevicesyslog / Xcode). Without this the transport runs
    // blind. `RUST_LOG` overrides the default; `try_init` makes a double-init a
    // no-op instead of a panic.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,outl_sync_iroh=debug,iroh=info")
            }),
        )
        .with_writer(std::io::stderr)
        .try_init();

    // rustls 0.23 needs a process-wide CryptoProvider installed before any
    // rustls consumer builds a client. iroh pulls rustls with
    // `default-features = false`, which drops the feature-selected default
    // provider for the whole workspace — so reqwest (Tauri's asset protocol,
    // serving the webview) panics in `ClientBuilder::build()` at boot. Install
    // `ring` (the provider in our dep graph) explicitly so every rustls user
    // shares it. Ignore the error: a second call just means it's already set.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init());

    // Camera/QR scanning is the device-pairing entry point and only
    // compiles on the mobile targets (Android + iOS). Gate it behind
    // `cfg(mobile)` so the desktop/host build of this crate stays clean.
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
        .setup(|app| {
            let local_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            std::fs::create_dir_all(&local_dir)?;

            let actor = load_or_create_actor(&local_dir)?;
            // Storage is a local folder synced by iroh (no iCloud).
            // `resolve_storage_root` reopens the previously chosen path
            // (`WorkspaceCfg.last`) when present, otherwise defaults to the
            // app-local `<app-data-dir>/outl/`. Change detection is the iroh
            // reload signal — there is no filesystem watcher.
            let persisted = outl_config::load().workspace.last;
            let storage_root = resolve_storage_root(&local_dir, persisted.as_deref());
            std::fs::create_dir_all(&storage_root)?;
            info!("workspace root {}", storage_root.display());
            let hlc = HlcGenerator::new(actor);

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));

            spawn_workspace_opener(
                workspace.clone(),
                storage_root.clone(),
                hlc.clone(),
                app.handle().clone(),
            );

            let registry = Arc::new(RuntimeRegistry::with_builtins());

            // Wire iroh P2P sync. Driven by the `[sync]` section of the
            // global config: iroh is the default on mobile (P2P is the
            // companion app's whole point), and we only fall back to the
            // iCloud file transport when the config explicitly selects
            // `transport = "file"`. A bind/identity failure logs and
            // returns `None` — the app still runs on the native iCloud
            // watcher, so iroh trouble never blocks startup.
            let sync_cfg = outl_config::load().sync;
            let iroh = iroh_sync::wire_iroh_transport(
                &app.handle().clone(),
                storage_root.clone(),
                actor,
                sync_cfg.transport,
            );

            app.manage(AppState {
                workspace,
                hlc,
                storage_root,
                registry,
                iroh,
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
            // Peer / device management
            outl_peer_list,
            outl_peer_remove,
            outl_peer_status,
            outl_sync_now,
            outl_peer_pair_host,
            outl_peer_pair_join,
            // Workspace folder selection (choose where the workspace lives)
            workspace_picker::set_workspace,
            // Code execution
            exec::run_code_block,
            // Legacy
            list_outline,
            add_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
