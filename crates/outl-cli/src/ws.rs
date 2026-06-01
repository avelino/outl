//! Shared workspace bootstrap used by every machine-shaped subcommand.
//!
//! Opens the SQLite-backed `Workspace`, constructs a matching
//! `HlcGenerator`, and acquires the workspace lock. All callers route
//! through here so error mapping stays consistent (every failure becomes
//! an [`ApiError`] with a stable code).

use std::path::{Path, PathBuf};

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::SqliteStorage;
use outl_core::workspace::Workspace;
use outl_core::{LockError, WorkspaceLock};

use crate::output::{codes, ApiError};
use crate::workspace_layout::{read_config, Paths};

/// Per-command runtime context. The lock is held for the whole command
/// so two CLI invocations don't race against `outl serve`.
pub struct WsCtx {
    /// Workspace path layout (root, .outl/, pages/, journals/, …).
    /// Consumed by surfaces that need on-disk paths (doctor, export,
    /// workspace info) without re-deriving them from `root`.
    pub paths: Paths,
    /// Loaded `Workspace` ready for reads or `apply`.
    pub workspace: Workspace,
    /// HLC generator bound to this workspace's actor id.
    pub hlc: HlcGenerator,
    /// Workspace path on disk.
    pub root: PathBuf,
    /// Actor id for this device.
    pub actor: ActorId,
    /// Hold the workspace lock for the lifetime of the context.
    #[allow(dead_code)]
    lock: WorkspaceLock,
}

/// Open the workspace at `path`. Returns a stable [`ApiError`] on any
/// boot failure (missing workspace, bad config, lock contention).
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

    let cfg = read_config(&paths).map_err(|e| {
        ApiError::new(
            codes::NO_WORKSPACE,
            format!("workspace config missing or unreadable: {e}"),
        )
    })?;
    let actor = cfg
        .actor()
        .map_err(|e| ApiError::new(codes::INTERNAL, format!("invalid actor in config: {e}")))?;

    let lock = WorkspaceLock::acquire(&paths.root).map_err(|e| match e {
        LockError::AlreadyHeld(p) => ApiError::new(
            codes::INVALID_ARG,
            format!(
                "workspace at {} is locked by another outl process (lock: {})",
                paths.root.display(),
                p.display()
            ),
        ),
        other => ApiError::internal(other),
    })?;

    let storage = SqliteStorage::open(&paths.db).map_err(ApiError::internal)?;
    let workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
            .map_err(ApiError::internal)?;
    let hlc = HlcGenerator::new(actor);

    Ok(WsCtx {
        root: paths.root.clone(),
        paths,
        workspace,
        hlc,
        actor,
        lock,
    })
}
