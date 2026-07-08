//! Per-page storage shard registration (RFC #137 Phase B).
//!
//! [`register_per_page_storages`] scans `ops/<actor>/` for per-page
//! shards, opens each one, and registers it with the workspace. It
//! also reads `.outl` sidecars under `pages/` and `journals/` to build
//! the `NodeId → slug` map that `Workspace::apply` uses to route ops.
//!
//! Shared by CLI, TUI, desktop, and mobile so the registration logic
//! can't drift across clients.

use std::path::Path;

use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, PageScope};
use outl_core::workspace::Workspace;
use tracing::warn;

/// Scan `ops/<actor>/` for per-page shards and register each one with
/// the workspace. Also reads `.outl` sidecars to build the
/// `NodeId → slug` map that `apply` uses to route ops.
///
/// Shards are opened **unbounded** (cap = 0) so `all_ops()` returns
/// the full history during the subsequent reboot. The caller applies
/// `Workspace::apply_lru_cap(cap)` afterwards to shed cold history.
///
/// No-op when the `<actor>/` subdir doesn't exist (legacy Global
/// layout). Safe to call on every boot.
///
/// After calling this, the caller should check
/// [`Workspace::has_page_storages`] and, if true, call
/// [`Workspace::reboot_with_all_storages`] so the materialized tree
/// includes ops from every shard.
pub fn register_per_page_storages(
    ws: &mut Workspace,
    ops_dir: &Path,
    actor: ActorId,
    workspace_root: &Path,
) {
    let actor_dir = ops_dir.join(actor.to_string());
    if !actor_dir.is_dir() {
        return;
    }

    let mut registered = 0usize;
    for entry in walkdir::WalkDir::new(&actor_dir).max_depth(1) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let slug = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let storage = JsonlStorage::open_with_scope_cap(
            ops_dir.to_path_buf(),
            actor,
            PageScope::PerPage(slug.clone()),
            0, // unbounded — boot needs full history; caller applies LRU cap after reboot
        );
        match storage {
            Ok(s) => {
                ws.register_page_storage(&slug, Box::new(s));
                registered += 1;
            }
            Err(e) => {
                warn!("could not open per-page storage for {slug}: {e}");
            }
        }
    }

    if registered == 0 {
        return;
    }

    tracing::info!("registered {registered} per-page storage shards");

    // Build page-root → slug map from sidecars.
    for dir in [
        workspace_root.join("pages"),
        workspace_root.join("journals"),
    ] {
        if !dir.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir).max_depth(1) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let md_path = entry.path();
            if md_path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let sidecar_path = md_path.with_extension("outl");
            let slug = match md_path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            match outl_md::sidecar::read(&sidecar_path) {
                Ok(sc) => {
                    ws.register_page_root(sc.page_id, &slug);
                }
                Err(e) => {
                    warn!("skip sidecar {}: {e}", sidecar_path.display());
                }
            }
        }
    }
}
