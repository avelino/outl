//! Workspace-level coordination locks.
//!
//! Two distinct locks live under `<workspace>/`, in different
//! subdirectories that map to their scopes:
//!
//! - [`WorkspaceLock`] — a **shared** advisory flock on
//!   `<workspace>/.outl/.lock`. Every legit `outl` process (TUI,
//!   MCP server, subprocess CLI, sink-outl plugin, future Tauri
//!   shell) holds this one for the duration of its open. It only
//!   fails when the filesystem itself can't be locked (e.g. some
//!   network mounts). The historical "another outl process is
//!   already open" rejection was a mistake inherited from the
//!   SQLite era. JSONL storage is one-file-per-actor, so concurrent
//!   processes are safe as long as each one writes to its own file
//!   — which is what [`ActorWriteLock`] enforces.
//!
//! - [`ActorWriteLock`] — an **exclusive** advisory flock on
//!   `<workspace>/ops/.lock-<actor>`. Lives next to the JSONL files
//!   it's gating, not under `.outl/`. Held by exactly one process
//!   at a time per actor id. The flow:
//!
//!   1. Read `actor_id` from `config.toml`.
//!   2. Try `ActorWriteLock::try_acquire(ops, config_actor)`.
//!   3. On success: this process owns the device's "default" actor;
//!      writes go to `ops-<config_actor>.jsonl`.
//!   4. On `AlreadyHeld`: another `outl` already owns that actor.
//!      The caller generates a fresh `ActorId::new()` and locks that
//!      one instead. Writes go to a brand-new
//!      `ops-<ephemeral>.jsonl`, mergeable by every reader.
//!
//! Both locks are advisory (POSIX flock semantics, Windows
//! LockFileEx). They only protect against well-behaved `outl`
//! processes — a `rm .outl/.lock` won't break anything, just makes
//! a second `outl` skip the polite handshake.

use fs2::FileExt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::id::ActorId;

/// Failure modes when acquiring a workspace-level lock.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another `outl` process already holds the same scope (only
    /// possible for [`ActorWriteLock`]; [`WorkspaceLock`] is shared
    /// and accepts multiple holders).
    #[error("workspace is already open in another outl process: {0}")]
    AlreadyHeld(PathBuf),
    /// Filesystem error creating or opening the lock file.
    #[error("io error on {path}: {source}")]
    Io {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Holds a **shared** advisory lock on `<workspace>/.outl/.lock`.
///
/// Multiple `outl` processes can hold it simultaneously. The lock
/// only signals "this workspace is currently in use by at least one
/// `outl` process" — useful as a probe (`outl doctor` looks at it)
/// and for the rare filesystem where `flock` itself reports an I/O
/// error we want to surface, but it never rejects a legitimate
/// second opener.
///
/// Per-actor write coordination lives in [`ActorWriteLock`].
///
/// Drop releases the lock automatically. Don't try to release manually
/// — `Drop` is the API.
#[must_use = "the lock is released when the value is dropped; bind it to a variable"]
#[derive(Debug)]
pub struct WorkspaceLock {
    file: File,
    /// Path of the lock file. Kept for diagnostics; not stripped on drop
    /// because other processes may legitimately reuse the file.
    path: PathBuf,
}

impl WorkspaceLock {
    /// Try to acquire the shared workspace lock. Returns immediately —
    /// blocking semantics would deadlock the TUI on a stale lock.
    pub fn acquire(workspace_root: &Path) -> Result<Self, LockError> {
        let dir = workspace_root.join(".outl");
        if !dir.exists() {
            fs::create_dir_all(&dir).map_err(|e| LockError::Io {
                path: dir.clone(),
                source: e,
            })?;
        }
        let lock_path = dir.join(".lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| LockError::Io {
                path: lock_path.clone(),
                source: e,
            })?;
        // Shared (LOCK_SH) — every well-behaved opener piles on.
        // The historical exclusive variant came from the SQLite era
        // (two writers on a single log.db would race); JSONL stores
        // writes per actor, so two `outl` processes are safe as long
        // as each one owns its own `ActorWriteLock`.
        //
        // Preserve the underlying `io::Error` instead of wrapping the
        // failure in a generic message. Permission denied, unsupported
        // locking on the mount, ENOLCK from a full kernel table — the
        // root cause matters for `outl doctor` and bug reports.
        if let Err(e) = file.try_lock_shared() {
            return Err(LockError::Io {
                path: lock_path.clone(),
                source: e.into(),
            });
        }
        Ok(Self {
            file,
            path: lock_path,
        })
    }

    /// Path of the lock file (for diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Holds an **exclusive** advisory lock on
/// `<workspace>/ops/.lock-<actor>`.
///
/// One process per actor id. The companion to [`WorkspaceLock`]:
/// the workspace lock is shared, this one is the per-actor writer
/// gate. The caller's flow is:
///
/// ```text
/// let _ws = WorkspaceLock::acquire(root)?;
/// let actor = match ActorWriteLock::try_acquire(ops, config_actor) {
///     Ok(lock) => (config_actor, lock),         // first process: own device default
///     Err(LockError::AlreadyHeld(_)) => {
///         let ephemeral = ActorId::new();
///         let lock = ActorWriteLock::try_acquire(ops, ephemeral)?;
///         (ephemeral, lock)
///     }
///     Err(other) => return Err(other),
/// };
/// // ...open JsonlStorage with `actor.0` as the write actor...
/// ```
///
/// Drop releases the lock and leaves the lock file in place (it's
/// cheap to recreate; deletion could race the next opener).
#[must_use = "the lock is released when the value is dropped; bind it to a variable"]
#[derive(Debug)]
pub struct ActorWriteLock {
    file: File,
    path: PathBuf,
    actor: ActorId,
}

impl ActorWriteLock {
    /// Try to acquire the exclusive write lock for `actor` under
    /// `ops_dir`. Returns immediately. Creates `ops_dir` if missing
    /// because sync transports sometimes garbage-collect empty
    /// directories.
    pub fn try_acquire(ops_dir: &Path, actor: ActorId) -> Result<Self, LockError> {
        if !ops_dir.exists() {
            fs::create_dir_all(ops_dir).map_err(|e| LockError::Io {
                path: ops_dir.to_path_buf(),
                source: e,
            })?;
        }
        let lock_path = ops_dir.join(format!(".lock-{actor}"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| LockError::Io {
                path: lock_path.clone(),
                source: e,
            })?;
        // `try_lock_exclusive` failing on `WouldBlock` means another
        // process owns this actor — the expected, recoverable case
        // that `resolve_write_actor` reacts to. Any other error
        // (permission denied, ENOLCK, unsupported locking on the
        // mount, ...) must NOT pretend to be "already held": the
        // ephemeral fallback would then loop trying to acquire a
        // lock the filesystem can't grant. Surface those as
        // [`LockError::Io`] with the original source.
        if let Err(e) = file.try_lock_exclusive() {
            return Err(if e.kind() == std::io::ErrorKind::WouldBlock {
                LockError::AlreadyHeld(lock_path)
            } else {
                LockError::Io {
                    path: lock_path,
                    source: e,
                }
            });
        }
        Ok(Self {
            file,
            path: lock_path,
            actor,
        })
    }

    /// Actor id this lock is gating.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Path of the lock file (for diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ActorWriteLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Resolve which actor this process should write under.
///
/// Tries `config_actor` first — that's the "device default" stored
/// in `config.toml`, and the common case (no other `outl` process
/// open). On `AlreadyHeld`, generates a fresh ephemeral actor and
/// returns its own lock + id, so the caller writes to a brand-new
/// `ops-<ephemeral>.jsonl` without colliding.
///
/// Returns the lock (caller must keep it alive) and the actor id to
/// use when opening `JsonlStorage`.
pub fn resolve_write_actor(
    ops_dir: &Path,
    config_actor: ActorId,
) -> Result<(ActorWriteLock, ActorId), LockError> {
    match ActorWriteLock::try_acquire(ops_dir, config_actor) {
        Ok(lock) => Ok((lock, config_actor)),
        Err(LockError::AlreadyHeld(_)) => {
            let ephemeral = ActorId::new();
            let lock = ActorWriteLock::try_acquire(ops_dir, ephemeral)?;
            Ok((lock, ephemeral))
        }
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn workspace_lock_allows_concurrent_opens() {
        let dir = TempDir::new().unwrap();
        let a = WorkspaceLock::acquire(dir.path()).unwrap();
        // Used to fail with AlreadyHeld; under shared semantics both
        // holders coexist.
        let b = WorkspaceLock::acquire(dir.path()).unwrap();
        assert!(a.path().exists());
        assert!(b.path().exists());
    }

    #[test]
    fn workspace_lock_creates_missing_outl_dir() {
        let dir = TempDir::new().unwrap();
        let lock = WorkspaceLock::acquire(dir.path()).unwrap();
        assert!(lock.path().parent().unwrap().is_dir());
    }

    #[test]
    fn workspace_lock_drop_releases() {
        let dir = TempDir::new().unwrap();
        {
            let _lock = WorkspaceLock::acquire(dir.path()).unwrap();
        }
        let _second = WorkspaceLock::acquire(dir.path()).unwrap();
    }

    #[test]
    fn actor_write_lock_rejects_second_holder_for_same_actor() {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("ops");
        let actor = ActorId::new();
        let _first = ActorWriteLock::try_acquire(&ops, actor).unwrap();
        match ActorWriteLock::try_acquire(&ops, actor) {
            Err(LockError::AlreadyHeld(_)) => {}
            other => panic!("expected AlreadyHeld, got {other:?}"),
        }
    }

    #[test]
    fn actor_write_lock_allows_different_actors() {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("ops");
        let a = ActorWriteLock::try_acquire(&ops, ActorId::new()).unwrap();
        let b = ActorWriteLock::try_acquire(&ops, ActorId::new()).unwrap();
        // Both held without contention — that's the whole point.
        assert_ne!(a.actor(), b.actor());
    }

    #[test]
    fn resolve_write_actor_uses_config_when_free() {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("ops");
        let cfg_actor = ActorId::new();
        let (_lock, resolved) = resolve_write_actor(&ops, cfg_actor).unwrap();
        assert_eq!(resolved, cfg_actor);
    }

    #[test]
    fn resolve_write_actor_falls_back_to_ephemeral_when_config_held() {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("ops");
        let cfg_actor = ActorId::new();
        let _first_lock = ActorWriteLock::try_acquire(&ops, cfg_actor).unwrap();
        let (_second_lock, resolved) = resolve_write_actor(&ops, cfg_actor).unwrap();
        assert_ne!(
            resolved, cfg_actor,
            "second process must NOT reuse the config actor"
        );
    }
}
