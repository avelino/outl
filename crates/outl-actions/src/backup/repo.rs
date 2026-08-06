//! [`BackupRepo`] — the git plumbing: staging, committing, verifying,
//! listing, restoring.
//!
//! Every decision about *where* the repository lives and *what* it
//! captures is documented in the parent module; this file is the
//! mechanics.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::warn;

use super::{
    default_git_dir, git_available, run_git, BackupEntry, BackupError, AUTHOR_EMAIL, AUTHOR_NAME,
    FORCED_ADD_EXCLUDES, GITIGNORE, REQUIRED_DIRS, REQUIRED_FILES,
};

/// A workspace's backup repository: a git directory **outside** the
/// workspace, with the workspace as its work tree.
///
/// [`BackupRepo::for_workspace`] is the constructor every caller wants.
/// [`BackupRepo::at`] exists so a test (or a future `--repo` flag) can
/// point the same logic at an explicit pair of directories.
#[derive(Debug, Clone)]
pub struct BackupRepo {
    git_dir: PathBuf,
    work_tree: PathBuf,
}

impl BackupRepo {
    /// The repository this device keeps for `root`.
    ///
    /// The location is derived from the workspace's **canonical path**,
    /// so a workspace that is moved or renamed starts a fresh history
    /// rather than silently writing into the old one. The previous
    /// history is not deleted; it stays under
    /// [`outl_core::device_dir`]`/backups/`.
    pub fn for_workspace(root: &Path) -> Self {
        let work_tree = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let git_dir = default_git_dir(&work_tree);
        Self { git_dir, work_tree }
    }

    /// An explicit `(git dir, work tree)` pair.
    pub fn at(git_dir: impl Into<PathBuf>, work_tree: impl Into<PathBuf>) -> Self {
        Self {
            git_dir: git_dir.into(),
            work_tree: work_tree.into(),
        }
    }

    /// Where this repository's git directory lives.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The workspace this repository backs up.
    pub fn work_tree(&self) -> &Path {
        &self.work_tree
    }

    /// Has the repository been created?
    pub fn is_initialized(&self) -> bool {
        self.git_dir.join("HEAD").is_file()
    }

    /// Create (or refresh) the repository.
    ///
    /// Idempotent: safe to call on one that already exists, which is
    /// what lets the exclusion list evolve.
    pub fn init(&self) -> Result<(), BackupError> {
        if !git_available() {
            return Err(BackupError::GitUnavailable);
        }
        if !self.work_tree.is_dir() {
            return Err(BackupError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("workspace {} does not exist", self.work_tree.display()),
            )));
        }
        if !self.is_initialized() {
            std::fs::create_dir_all(&self.git_dir)?;
            self.git(&["init", "--quiet"])?;
        }

        let exclude = self.git_dir.join("info").join("exclude");
        if let Some(parent) = exclude.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&exclude, GITIGNORE)?;

        if self.work_tree.join(".git").exists() {
            warn!(
                "{} has a .git of its own — outl does not use it; backups live in {}",
                self.work_tree.display(),
                self.git_dir.display()
            );
        }
        Ok(())
    }

    /// Commit the current state of the workspace.
    ///
    /// Returns the new [`BackupEntry`], or `None` when there was nothing
    /// to commit — the overwhelmingly common case on a periodic timer,
    /// and deliberately not an error.
    ///
    /// Either way, the op log's presence in the history is **verified**
    /// before returning: a snapshot that silently dropped `ops/` is
    /// [`BackupError::OpLogNotCaptured`], not a success.
    pub fn snapshot(&self, message: &str) -> Result<Option<BackupEntry>, BackupError> {
        if !git_available() {
            return Err(BackupError::GitUnavailable);
        }
        if !self.is_initialized() {
            return Err(BackupError::NotInitialized(self.work_tree.clone()));
        }

        // A failure between staging and the commit must not leave the
        // index holding a whole workspace: the next snapshot would
        // commit a half-staged state under a message describing a
        // different moment.
        let committed = match self.stage_and_commit(message) {
            Ok(v) => v,
            Err(e) => {
                let _ = self.git(&["reset", "--quiet"]);
                return Err(e);
            }
        };

        self.verify_op_log_captured()?;
        if !committed {
            return Ok(None);
        }
        Ok(self.list(1)?.into_iter().next())
    }

    /// Stage everything and commit. `Ok(false)` means "nothing changed".
    fn stage_and_commit(&self, message: &str) -> Result<bool, BackupError> {
        self.stage()?;

        // `diff --cached --quiet` exits 1 when there *are* staged
        // changes. That inversion is why this isn't an `?`.
        let has_changes = self.git(&["diff", "--cached", "--quiet"]).is_err();
        if !has_changes {
            return Ok(false);
        }

        self.git(&[
            "commit",
            "--quiet",
            // Belt and braces on top of the `-c` overrides every
            // invocation carries: the user's global config must never
            // turn a background snapshot into a hook run or a
            // hardware-key prompt.
            "--no-verify",
            "--no-gpg-sign",
            "--message",
            message,
        ])?;
        Ok(true)
    }

    /// Two staging passes: one that honours the workspace's ignore
    /// rules, one that overrides them for the paths that carry state.
    fn stage(&self) -> Result<(), BackupError> {
        self.git(&["add", "--all", "."])?;

        // `.gitignore` in the work tree beats `$GIT_DIR/info/exclude`
        // and a negation there can't win it back, so the only reliable
        // answer for "this must be captured" is `--force` on an explicit
        // pathspec. Missing paths are dropped first: git fails the whole
        // command on a pathspec that matches nothing.
        let mut required: Vec<String> = Vec::new();
        for dir in REQUIRED_DIRS {
            if self.work_tree.join(dir).is_dir() {
                required.push((*dir).to_string());
            }
        }
        for file in REQUIRED_FILES {
            if self.work_tree.join(file).is_file() {
                required.push((*file).to_string());
            }
        }
        if required.is_empty() {
            return Ok(());
        }

        let mut args: Vec<&str> = vec!["add", "--force", "--"];
        args.extend(required.iter().map(String::as_str));
        args.extend(FORCED_ADD_EXCLUDES.iter().copied());
        self.git(&args)?;
        Ok(())
    }

    /// Every `ops/*.jsonl` on disk must be in `HEAD`.
    ///
    /// Without this the failure mode is silent and total: the CLI prints
    /// a commit id, `outl backup status` reports a healthy history, and
    /// the one thing that can rebuild the workspace was never in it.
    pub(crate) fn verify_op_log_captured(&self) -> Result<(), BackupError> {
        // Recursive, NOT `read_dir`: under the per-page layout
        // (`ops/<actor>/<slug>.jsonl`, what `outl migrate-to-per-page-ops`
        // produces) every top-level entry is a *directory*, so a flat
        // scan finds zero `.jsonl` files, the emptiness check below
        // short-circuits, and this function reports success having
        // verified nothing at all — on the layout with the most files to
        // lose. `doctor/oplog.rs` already walks the same tree this way.
        let ops_dir = self.work_tree.join("ops");
        let mut on_disk: Vec<String> = Vec::new();
        for entry in walkdir::WalkDir::new(&ops_dir).max_depth(3) {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // Git tracks paths relative to the work tree, with `/`.
            if let Ok(rel) = path.strip_prefix(&self.work_tree) {
                on_disk.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        if on_disk.is_empty() {
            return Ok(());
        }

        // No HEAD while op logs sit on disk means staging produced
        // nothing at all — everything was excluded. Report all of them.
        let tracked: HashSet<String> = if self.has_head() {
            self.git(&["ls-tree", "-r", "--name-only", "HEAD", "--", "ops"])?
                .lines()
                .map(|line| line.trim().to_string())
                .collect()
        } else {
            HashSet::new()
        };

        let mut missing: Vec<String> = on_disk
            .into_iter()
            .filter(|path| !tracked.contains(path))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort();
        Err(BackupError::OpLogNotCaptured { missing })
    }

    /// Most recent backups, newest first.
    pub fn list(&self, limit: usize) -> Result<Vec<BackupEntry>, BackupError> {
        if !self.is_initialized() {
            return Err(BackupError::NotInitialized(self.work_tree.clone()));
        }
        // A repo with no commits has no HEAD, and `git log` exits
        // non-zero on it. That is the state right after `init` — an
        // empty history, not a failure — so answer with one rather than
        // an error.
        if !self.has_head() {
            return Ok(Vec::new());
        }
        // Unit separator between fields so a commit message containing
        // the delimiter can't shift the columns.
        let out = self.git(&[
            "log",
            &format!("--max-count={limit}"),
            "--date=iso-strict",
            "--pretty=format:%h\x1f%ad\x1f%s",
        ])?;

        Ok(out
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\x1f');
                Some(BackupEntry {
                    id: parts.next()?.to_string(),
                    at: parts.next()?.to_string(),
                    message: parts.next().unwrap_or_default().to_string(),
                })
            })
            .collect())
    }

    /// Unix timestamp of the newest snapshot, when there is one.
    ///
    /// The automatic pass derives "when did we last run" from this
    /// rather than from a state file: git already records it, and a
    /// separate file would be one more thing to keep honest (and one
    /// more thing to sync between devices by accident).
    pub(crate) fn last_snapshot_epoch(&self) -> Result<Option<i64>, BackupError> {
        if !self.has_head() {
            return Ok(None);
        }
        let out = self.git(&["log", "--max-count=1", "--pretty=format:%ct"])?;
        Ok(out.trim().parse::<i64>().ok())
    }

    /// Copy the workspace as it stood at `id` into `dest`.
    ///
    /// **Deliberately non-destructive.** Restoring in place would mean
    /// overwriting live files — including the op log — from a tool whose
    /// entire purpose is to be trusted when other things have gone
    /// wrong. Extracting to a separate directory lets the user diff
    /// first and decide what to bring back, which is what someone who
    /// has just lost data actually wants.
    pub fn restore(&self, id: &str, dest: &Path) -> Result<(), BackupError> {
        if !self.is_initialized() {
            return Err(BackupError::NotInitialized(self.work_tree.clone()));
        }
        std::fs::create_dir_all(dest)?;

        // `--work-tree` is resolved by git against *its own* cwd. A
        // relative `dest` — `.` above all — would otherwise land inside
        // the live workspace and the "restore" would overwrite the very
        // files the user is trying to recover, while the CLI printed
        // that nothing was modified. Resolve both sides to real paths
        // and refuse anything at or under the workspace.
        let dest_abs = dest.canonicalize()?;
        let root_abs = self
            .work_tree
            .canonicalize()
            .unwrap_or_else(|_| self.work_tree.clone());
        if dest_abs == root_abs || dest_abs.starts_with(&root_abs) {
            return Err(BackupError::RestoreIntoWorkspace {
                dest: dest_abs,
                root: root_abs,
            });
        }

        // Resolve the id to a commit SHA before it reaches `checkout`.
        // An id starting with `-` is otherwise absorbed as an option
        // (`outl backup restore -- -q --into …` gets one past clap), and
        // the user sees an inscrutable git error instead of "no such
        // snapshot". The resolved SHA can never be argument-shaped.
        let commit = self
            .git(&["rev-parse", "--verify", &format!("{id}^{{commit}}")])?
            .trim()
            .to_string();

        // A scratch index, so the checkout cannot touch the
        // repository's own: `git checkout <id> -- .` stages what it
        // writes, and against the live index the next snapshot would
        // commit the restored state as if it were current.
        let scratch_index = dest_abs.join(".outl-restore-index");
        let res = self.run(
            &["checkout", &commit, "--", "."],
            &dest_abs,
            Some(&scratch_index),
        );
        let _ = std::fs::remove_file(&scratch_index);
        res?;
        Ok(())
    }

    fn has_head(&self) -> bool {
        self.git(&["rev-parse", "--verify", "HEAD"]).is_ok()
    }

    /// Run `git` against this repository with the workspace as the work
    /// tree.
    pub(crate) fn git(&self, args: &[&str]) -> Result<String, BackupError> {
        let work_tree = self.work_tree.clone();
        self.run(args, &work_tree, None)
    }

    /// [`BackupRepo::git`] with an explicit work tree (restore targets a
    /// scratch directory) and an optional scratch index.
    fn run(
        &self,
        args: &[&str],
        work_tree: &Path,
        index: Option<&Path>,
    ) -> Result<String, BackupError> {
        let git_dir_arg = format!("--git-dir={}", self.git_dir.display());
        let work_tree_arg = format!("--work-tree={}", work_tree.display());
        // A directory that does not exist: git finds no hooks there and
        // runs none. A `core.hooksPath` in the user's global config must
        // never fire on outl's own commits.
        let hooks_arg = format!("core.hooksPath={}", self.git_dir.join("no-hooks").display());
        let name_arg = format!("user.name={AUTHOR_NAME}");
        let email_arg = format!("user.email={AUTHOR_EMAIL}");

        let mut full: Vec<&str> = vec![
            &git_dir_arg,
            &work_tree_arg,
            "-c",
            &hooks_arg,
            // Our own identity, so a machine with no `user.email`
            // configured still commits, and one that has a real identity
            // doesn't get the user's name onto a robot's commit.
            "-c",
            &name_arg,
            "-c",
            &email_arg,
            // A global `commit.gpgsign = true` wired to a hardware key
            // (1Password's `op-ssh-sign`, a YubiKey) turns every
            // background snapshot into a Touch ID prompt that blocks
            // until answered — on a process with no terminal.
            "-c",
            "commit.gpgsign=false",
            // Keep non-ASCII paths verbatim, so `ls-tree` output can be
            // compared against what is on disk.
            "-c",
            "core.quotePath=false",
        ];
        full.extend_from_slice(args);
        let label = args.first().copied().unwrap_or("?");
        run_git(work_tree, &full, label, index)
    }
}
