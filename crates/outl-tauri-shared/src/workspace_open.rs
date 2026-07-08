//! Workspace open / reconcile / actor-id primitives.
//!
//! The boot **openers** stay in the client crates (the desktop wires an
//! FS watcher + background reconcile + iroh slots; the mobile reconciles
//! inline and returns through `AppState`) — but every step they compose
//! lives here so the two can't drift on semantics.

use std::path::Path;
use std::str::FromStr;

use outl_actions::{migrate_legacy_into_today, open_today};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tracing::{info, warn};

/// Open (or create) the workspace rooted at `path`.
///
/// Idempotent: the `ops/`, `journals/`, `pages/` directories are created
/// if missing, and `migrate_legacy_into_today` reshuffles any
/// pre-page-model blocks under today's journal (also idempotent).
///
/// **Does NOT run the orphan-md reconcile pass** — that work scales with
/// the number of pages; each client decides whether to run
/// [`reconcile_orphan_md`] inline (mobile) or on a background thread
/// (desktop).
///
/// `lru_cap` is the in-memory op cache bound (RFC #137). `0` keeps the
/// legacy unbounded behaviour; any positive value sheds cold history
/// after boot completes (so RSS stays constant regardless of workspace
/// age).
pub fn open_workspace_at(
    actor: ActorId,
    hlc: &HlcGenerator,
    path: &Path,
    lru_cap: usize,
) -> anyhow::Result<Workspace> {
    std::fs::create_dir_all(path.join("ops"))?;
    std::fs::create_dir_all(path.join("journals"))?;
    std::fs::create_dir_all(path.join("pages"))?;

    let storage = JsonlStorage::open(path.join("ops"), actor)?;
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(path.to_path_buf()))?;

    if let Err(e) = migrate_legacy_into_today(&mut workspace, hlc) {
        warn!("legacy migration: {e}");
    }
    if let Err(e) = open_today(&mut workspace, hlc) {
        warn!("could not pre-open today: {e}");
    }

    // Apply the LRU cap AFTER boot so `ops_for_node` (used to rebuild
    // Yrs `Doc`s) had the full history in RAM. After this point cold
    // ops are read back from disk on demand via the offset index.
    workspace.apply_lru_cap(lru_cap);

    outl_actions::storage_scope::register_per_page_storages(
        &mut workspace,
        &path.join("ops"),
        actor,
        lru_cap,
        path,
    );
    if workspace.has_page_storages() {
        workspace.reboot_with_all_storages()?;
        workspace.apply_lru_cap(lru_cap);
    }

    Ok(workspace)
}

/// Load (or generate-and-persist) the device's actor id.
///
/// The actor identifies the device, not the workspace — it's reused
/// across whatever directory the user picks. Lives at
/// `<local_dir>/actor` as a plain ULID string.
pub fn load_or_create_actor(local_dir: &Path) -> std::io::Result<ActorId> {
    let path = local_dir.join("actor");
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        let raw = raw.trim();
        if let Ok(ulid) = ulid::Ulid::from_str(raw) {
            info!("loaded existing actor id {ulid}");
            return Ok(ActorId(ulid));
        }
        warn!("invalid actor id in {}, regenerating", path.display());
    }
    let actor = ActorId::new();
    std::fs::write(&path, actor.to_string())?;
    info!("generated fresh actor id {actor}");
    Ok(actor)
}

/// Scan `<root>/journals/` and `<root>/pages/` for `.md` files that are
/// not represented in the op log yet — either no sidecar exists (file
/// was just imported, dropped in by vim, or written by a peer that only
/// shipped the projection) or the sidecar's `last_synced_hash` is stale
/// (the file was edited externally since the last reconcile). Runs
/// `reconcile_md` on each so the workspace, the sidecar, and `.md`
/// converge.
///
/// Then runs the **desynced-projection** pass: pages whose sidecar is
/// hash-in-sync with the `.md` but references block ids no op log ever
/// created (projection written, ops append lost — e.g. the OS killed
/// the app right after an offline edit). The hash gate above can't see
/// those; `recover_desynced_projection` re-emits the lost ops with the
/// sidecar ids preserved so the blocks finally reach the log and sync.
pub fn reconcile_orphan_md(workspace: &mut Workspace, hlc: &HlcGenerator, storage_root: &Path) {
    let engine = outl_actions::SyncEngine::new(storage_root.to_path_buf(), hlc.actor());
    for path in &engine.scan_for_orphans() {
        if let Err(e) = outl_md::reconcile::reconcile_md(workspace, hlc, path, None) {
            warn!("orphan reconcile failed for {}: {e}", path.display());
        }
    }
    for path in &engine.scan_for_desynced_projections(workspace) {
        match outl_actions::recover_desynced_projection(workspace, hlc, storage_root, path) {
            Ok(n) if n > 0 => info!(
                "recovered {n} lost op(s) from desynced projection {}",
                path.display()
            ),
            Ok(_) => {}
            Err(e) => warn!("desync recovery failed for {}: {e}", path.display()),
        }
    }
}
