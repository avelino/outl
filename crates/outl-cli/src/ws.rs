//! Shared workspace bootstrap used by every machine-shaped subcommand.
//!
//! Opens the JSONL-backed `Workspace`, constructs a matching
//! `HlcGenerator`, and acquires the workspace lock. All callers route
//! through here so error mapping stays consistent (every failure becomes
//! an [`ApiError`] with a stable code).

use std::path::{Path, PathBuf};

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_core::{resolve_write_actor, ActorWriteLock, WorkspaceLock};

use crate::output::{codes, ApiError};
use crate::workspace_layout::{ensure_ops_dir, read_or_init_config, Paths};

/// Per-command runtime context. The shared workspace lock plus an
/// exclusive per-actor write lock are held for the whole command —
/// two CLI invocations against the same workspace coexist as long as
/// each one ends up writing under a distinct actor.
pub struct WsCtx {
    /// Workspace path layout (root, .outl/, pages/, journals/, …).
    /// Consumed by surfaces that need on-disk paths (doctor, export,
    /// workspace info) without re-deriving them from `root`.
    pub paths: Paths,
    /// Loaded `Workspace` ready for reads or `apply`.
    pub workspace: Workspace,
    /// HLC generator bound to whatever actor this process resolved to
    /// (config actor on the first opener; ephemeral on the rest).
    pub hlc: HlcGenerator,
    /// Workspace path on disk.
    pub root: PathBuf,
    /// Actor id this process writes under. Equal to the config actor
    /// when this process is the workspace's primary holder; a fresh
    /// `ActorId::new()` when another `outl` (TUI, MCP server, peer
    /// CLI) is already attached.
    pub actor: ActorId,
    /// Whether [`Self::actor`] is the config-default actor (`false`
    /// signals "ephemeral, this process spun a new ops jsonl"). Used
    /// by diagnostics and doctor reporting.
    pub ephemeral_actor: bool,
    /// Hold the shared workspace lock for the lifetime of the context.
    #[allow(dead_code)]
    lock: WorkspaceLock,
    /// Hold the exclusive per-actor write lock too.
    #[allow(dead_code)]
    actor_lock: ActorWriteLock,
}

/// Open the workspace at `path`. Returns a stable [`ApiError`] on any
/// boot failure (missing workspace, bad config, filesystem error).
///
/// **Lock policy.** The shared workspace lock accepts multiple
/// concurrent openers. Per-actor write coordination falls to
/// [`resolve_write_actor`]: this process tries the config actor
/// first; if another `outl` already owns it (TUI running, MCP server
/// proxy active, a peer CLI in flight), this process gets an
/// ephemeral actor and writes to a fresh `ops-<ephemeral>.jsonl`.
/// All readers merge every `ops-*.jsonl` so peers see every write.
pub fn open(path: &Path) -> Result<WsCtx, ApiError> {
    let paths = Paths::at(path.to_path_buf());
    if !paths.dot_outl.exists() {
        return Err(ApiError::new(
            codes::NO_WORKSPACE,
            format!(
                "no outl workspace at {} — run `outl init {}` first",
                paths.root.display(),
                paths.root.display()
            ),
        ));
    }

    let cfg = read_or_init_config(&paths).map_err(|e| {
        ApiError::new(
            codes::NO_WORKSPACE,
            format!("workspace config missing or unreadable: {e}"),
        )
    })?;
    let config_actor = cfg
        .actor()
        .map_err(|e| ApiError::new(codes::INTERNAL, format!("invalid actor in config: {e}")))?;

    // Shared workspace lock first — every well-behaved opener takes one.
    let lock = WorkspaceLock::acquire(&paths.root).map_err(ApiError::internal)?;

    ensure_ops_dir(&paths).map_err(ApiError::internal)?;

    // Exclusive per-actor write lock: config actor if available,
    // ephemeral otherwise. This is the contract that lets multiple
    // `outl` processes share the workspace safely.
    let (actor_lock, actor) =
        resolve_write_actor(&paths.ops, config_actor).map_err(ApiError::internal)?;
    let ephemeral_actor = actor != config_actor;

    let storage = JsonlStorage::open(paths.ops.clone(), actor).map_err(ApiError::internal)?;
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
            .map_err(ApiError::internal)?;
    // CLI is ephemeral: every command opens, mutates, exits. Writing a
    // snapshot here would race with the long-lived TUI/desktop/mobile
    // that own the workspace, and snapshots already pay back at boot
    // (`Workspace::open_with_storage` reads one via `snapshot::read_from_disk`
    // regardless of this policy). So opt out — read, don't write (#109).
    workspace.set_snapshot_policy(false, 0);
    // Register per-page shards if the workspace has been migrated
    // (RFC #137 Phase B). No-op for legacy Global workspaces.
    outl_actions::storage_scope::register_per_page_storages(
        &mut workspace,
        &paths.ops,
        actor,
        &paths.root,
    );
    // If per-page shards were registered, re-boot so the materialized
    // tree includes their ops. The initial boot only saw the Global
    // storage (which is empty after migration).
    if workspace.has_page_storages() {
        workspace
            .reboot_with_all_storages()
            .map_err(ApiError::internal)?;
        workspace.set_snapshot_policy(false, 0);
    }
    let hlc = HlcGenerator::new(actor);

    // Repair split-brain page/journal roots (two roots sharing one slug, e.g. a
    // sidecar-less `.md` reconciled to a fresh id) before any command reads or
    // writes. Idempotent and a no-op when clean; only emits Ops when a duplicate
    // exists, and those converge across devices via the op log. Covers every CLI
    // command and the long-lived MCP server (both open through here).
    //
    // Cost: an O(roots) scan grouping page roots by slug. It runs on every CLI
    // invocation, but the open above already replays the whole op log (O(ops),
    // far larger), so this scan is noise next to it. If a huge workspace ever
    // makes it show up in a profile, gate it on a persisted "merged at op-count
    // N" marker in `.outl/` rather than paying the scan when nothing changed.
    match outl_actions::merge_duplicate_slug_roots(&mut workspace, &hlc) {
        Ok(0) => {}
        Ok(n) => tracing::warn!("merged {n} duplicate slug root(s)"),
        Err(e) => tracing::warn!("duplicate-slug-root repair: {e}"),
    }

    Ok(WsCtx {
        root: paths.root.clone(),
        paths,
        workspace,
        hlc,
        actor,
        ephemeral_actor,
        lock,
        actor_lock,
    })
}
