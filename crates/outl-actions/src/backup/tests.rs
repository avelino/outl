//! Backup tests.
//!
//! Every test drives a [`BackupRepo::at`] pair — an explicit git dir
//! plus an explicit work tree, both inside one `TempDir` — rather than
//! [`BackupRepo::for_workspace`], which resolves the *real*
//! `outl_core::device_dir()`. A test that wrote there would leave a
//! repository in the developer's home directory and could collide with
//! another test run.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

/// Every test here needs a real `git`; skip rather than fail on a
/// machine without one (CI has it, a minimal container may not).
fn git_or_skip() -> bool {
    if git_available() {
        return true;
    }
    eprintln!("skipping: no git on PATH");
    false
}

struct Fixture {
    _tmp: TempDir,
    ws: PathBuf,
    repo: BackupRepo,
}

impl Fixture {
    fn page(&self, name: &str, body: &str) {
        std::fs::write(self.ws.join("pages").join(name), body).unwrap();
    }

    fn tracked(&self) -> String {
        self.repo.git(&["ls-files"]).unwrap()
    }
}

/// A workspace with the shape the projection layer produces, plus an
/// initialised backup repository outside it.
fn fixture() -> Option<Fixture> {
    if !git_or_skip() {
        return None;
    }
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(ws.join("pages")).unwrap();
    std::fs::create_dir_all(ws.join("ops")).unwrap();
    std::fs::write(ws.join("ops").join("ops-x.jsonl"), "{}\n").unwrap();
    let repo = BackupRepo::at(tmp.path().join("backup.git"), &ws);
    repo.init().unwrap();
    Some(Fixture {
        _tmp: tmp,
        ws,
        repo,
    })
}

/// Run plain `git` inside `dir` — used to build and inspect the *user's
/// own* repository, which outl must never touch.
fn user_git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Did a plain `git` command succeed? Used where failure is the
/// assertion (an unborn branch has no `HEAD` to resolve).
fn user_git_ok(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn init_is_idempotent() {
    let Some(fx) = fixture() else { return };
    assert!(fx.repo.is_initialized());
    fx.repo.init().unwrap();
    assert!(fx.repo.is_initialized());
}

#[test]
fn snapshot_captures_pages_and_op_log() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- hello\n");

    let entry = fx.repo.snapshot("first").unwrap().expect("has changes");
    assert_eq!(entry.message, "first");

    let tracked = fx.tracked();
    assert!(tracked.contains("pages/a.md"));
    assert!(
        tracked.contains("ops/ops-x.jsonl"),
        "the op log is the source of truth and must be backed up: {tracked}"
    );
}

#[test]
fn derived_caches_are_not_backed_up() {
    let Some(fx) = fixture() else { return };
    std::fs::create_dir_all(fx.ws.join(".outl").join("snapshots")).unwrap();
    std::fs::write(
        fx.ws.join(".outl").join("snapshots").join("snap-x.bin"),
        "x",
    )
    .unwrap();
    std::fs::write(fx.ws.join("ops").join(".ops-x.idx"), "x").unwrap();
    fx.page("a.md", "- hello\n");

    fx.repo.snapshot("first").unwrap().expect("has changes");
    let tracked = fx.tracked();
    assert!(!tracked.contains("snap-x.bin"), "got: {tracked}");
    assert!(
        !tracked.contains(".idx"),
        "the forced staging pass must not drag derived caches in: {tracked}"
    );
}

/// A periodic timer calls this constantly; "nothing changed" is the
/// normal case and must not look like a failure.
#[test]
fn snapshot_with_no_changes_is_none_not_an_error() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- hello\n");

    assert!(fx.repo.snapshot("first").unwrap().is_some());
    assert!(
        fx.repo.snapshot("second").unwrap().is_none(),
        "an unchanged workspace must produce no commit and no error"
    );
}

/// The scenario this module exists for: content is gone from the live
/// workspace and has to come back.
#[test]
fn restore_recovers_a_deleted_page_without_touching_the_live_one() {
    let Some(fx) = fixture() else { return };
    fx.page("important.md", "- do not lose this\n");
    let entry = fx.repo.snapshot("before the accident").unwrap().unwrap();

    // The accident.
    std::fs::remove_file(fx.ws.join("pages").join("important.md")).unwrap();
    fx.page("other.md", "- unrelated work\n");

    let out = TempDir::new().unwrap();
    fx.repo.restore(&entry.id, out.path()).unwrap();

    let recovered = std::fs::read_to_string(out.path().join("pages").join("important.md"))
        .expect("the deleted page must be recoverable");
    assert_eq!(recovered, "- do not lose this\n");

    // Restoring is non-destructive: work done after the snapshot is
    // still there in the live workspace.
    assert!(
        fx.ws.join("pages").join("other.md").exists(),
        "restore must never overwrite the live workspace"
    );
}

#[test]
fn list_returns_newest_first() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "1");
    fx.repo.snapshot("one").unwrap();
    fx.page("a.md", "2");
    fx.repo.snapshot("two").unwrap();

    let entries = fx.repo.list(10).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "two");
    assert_eq!(entries[1].message, "one");
}

/// Right after `init` there is no HEAD, and `git log` fails on that. An
/// empty history is a state, not an error — `outl backup status` on a
/// fresh repo must say "no snapshots yet", not blow up.
#[test]
fn list_on_a_fresh_repo_is_empty_not_an_error() {
    let Some(fx) = fixture() else { return };
    assert!(fx
        .repo
        .list(10)
        .expect("a repo with no commits is empty")
        .is_empty());
}

/// The one that made the recovery tool a destruction tool.
///
/// `--work-tree` is resolved by git against its own cwd, which is the
/// workspace — so a relative `dest` (`.` worst of all) wrote the
/// snapshot straight over the live files, while the CLI printed "Your
/// live workspace was NOT modified."
#[test]
fn restore_refuses_to_target_the_live_workspace() {
    let Some(fx) = fixture() else { return };
    let page = fx.ws.join("pages").join("important.md");
    fx.page("important.md", "- snapshot content\n");
    let entry = fx.repo.snapshot("v1").unwrap().unwrap();

    // Work the user did after the snapshot — what a bad restore eats.
    std::fs::write(&page, "- work written after the snapshot\n").unwrap();

    for target in [
        fx.ws.clone(),
        fx.ws.join("pages"),
        fx.ws.join("nested/deep"),
    ] {
        let err = fx
            .repo
            .restore(&entry.id, &target)
            .expect_err("restoring into the live workspace must be refused");
        assert!(
            matches!(err, BackupError::RestoreIntoWorkspace { .. }),
            "expected RestoreIntoWorkspace, got {err:?}"
        );
    }

    assert_eq!(
        std::fs::read_to_string(&page).unwrap(),
        "- work written after the snapshot\n",
        "the live workspace must be byte-identical after a refused restore"
    );
}

/// A legitimate restore must not stage anything in the backup
/// repository: a dirtied index means the next snapshot commits the
/// restored state as if it were current.
#[test]
fn restore_leaves_the_backup_index_untouched() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- one\n");
    let entry = fx.repo.snapshot("v1").unwrap().unwrap();
    fx.page("a.md", "- two\n");

    let out = TempDir::new().unwrap();
    let dest = out.path().join("recovered");
    fx.repo.restore(&entry.id, &dest).unwrap();

    assert_eq!(
        std::fs::read_to_string(dest.join("pages").join("a.md")).unwrap(),
        "- one\n"
    );
    assert!(
        fx.repo
            .git(&["diff", "--cached", "--name-only"])
            .unwrap()
            .trim()
            .is_empty(),
        "restore must not stage anything in the backup repository"
    );
    assert!(
        !dest.join(".outl-restore-index").exists(),
        "the scratch index must be cleaned up"
    );
}

#[test]
fn snapshot_without_init_is_an_error_not_a_panic() {
    let tmp = TempDir::new().unwrap();
    let repo = BackupRepo::at(tmp.path().join("g"), tmp.path().join("ws"));
    assert!(matches!(
        repo.snapshot("x"),
        Err(BackupError::NotInitialized(_)) | Err(BackupError::GitUnavailable)
    ));
}

#[test]
fn best_effort_never_propagates() {
    let tmp = TempDir::new().unwrap();
    // A path that is not a workspace: whatever fails, it comes back as
    // `None`, never as an error or a panic.
    assert!(snapshot_best_effort(&tmp.path().join("nope"), "x").is_none());
}

// ---------------------------------------------------------------
// The repository lives outside the workspace (A4 + A1)
// ---------------------------------------------------------------

/// The workspace is a sync surface. A `.git/` inside it means Syncthing
/// / Dropbox / NFS replicate an object store, an index and a lock file
/// under eventual consistency.
#[test]
fn the_repository_is_never_inside_the_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let resolved = repo_dir(&ws);
    let ws_abs = ws.canonicalize().unwrap();
    assert!(
        !resolved.starts_with(&ws_abs),
        "the backup repo must not live on the workspace's sync surface: {}",
        resolved.display()
    );
}

/// `init` must write nothing at all into the workspace — no `.git`, no
/// pointer file, no exclude file of ours next to the user's.
#[test]
fn init_writes_nothing_into_the_workspace() {
    let Some(fx) = fixture() else { return };
    let mut entries: Vec<String> = std::fs::read_dir(&fx.ws)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["ops".to_string(), "pages".to_string()]);
}

/// The workspace is already the user's own git repository, with staged
/// work, a branch of their own, a `pre-commit` hook, and an
/// `info/exclude` they wrote. A snapshot must leave every one of those
/// exactly as it found them.
#[test]
fn the_users_own_repository_is_never_touched() {
    let Some(fx) = fixture() else { return };

    user_git(&fx.ws, &["init", "--quiet"]);
    user_git(&fx.ws, &["config", "user.email", "me@example.com"]);
    user_git(&fx.ws, &["config", "user.name", "The User"]);
    user_git(&fx.ws, &["checkout", "-q", "-b", "my-branch"]);

    // A hook that fails: if outl ever commits through this repository,
    // the commit dies here and the marker file proves it ran.
    let hooks = fx.ws.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    let marker = fx.ws.join(".git").join("hook-ran");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Their own exclusions, and their own partial staging.
    let user_exclude = fx.ws.join(".git").join("info").join("exclude");
    std::fs::create_dir_all(user_exclude.parent().unwrap()).unwrap();
    std::fs::write(&user_exclude, "secrets.env\n").unwrap();
    fx.page("staged.md", "- staged by the user\n");
    fx.page("unstaged.md", "- not staged\n");
    user_git(&fx.ws, &["add", "pages/staged.md"]);
    let staged_before = user_git(&fx.ws, &["diff", "--cached", "--name-only"]);

    fx.repo.snapshot("outl's own snapshot").unwrap().unwrap();

    assert!(
        !marker.exists(),
        "the user's pre-commit hook must never run for an outl snapshot"
    );
    assert_eq!(
        user_git(&fx.ws, &["diff", "--cached", "--name-only"]),
        staged_before,
        "outl must not touch the user's staging area"
    );
    assert_eq!(
        user_git(&fx.ws, &["symbolic-ref", "--short", "HEAD"]).trim(),
        "my-branch",
        "outl must not move the user's branch"
    );
    assert!(
        !user_git_ok(&fx.ws, &["rev-parse", "--verify", "HEAD"]),
        "outl must not commit into the user's repository"
    );
    assert_eq!(
        std::fs::read_to_string(&user_exclude).unwrap(),
        "secrets.env\n",
        "the user's own info/exclude must be preserved verbatim"
    );
}

/// A machine with no git identity configured must still take backups,
/// and the commit must not be attributed to the user.
#[test]
fn snapshots_commit_under_outls_own_identity() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- hello\n");
    fx.repo.snapshot("first").unwrap().unwrap();

    let author = fx
        .repo
        .git(&["log", "-1", "--pretty=format:%an <%ae>"])
        .unwrap();
    assert_eq!(author.trim(), format!("{AUTHOR_NAME} <{AUTHOR_EMAIL}>"));
}

/// A commit that fails must not leave the whole workspace staged: the
/// next snapshot would commit a half-staged state under a message
/// describing a different moment. An empty message is the deterministic
/// way to make git refuse.
#[test]
fn a_failed_commit_leaves_nothing_staged() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- hello\n");

    assert!(
        fx.repo.snapshot("").is_err(),
        "git refuses an empty commit message"
    );
    assert!(
        fx.repo
            .git(&["diff", "--cached", "--name-only"])
            .unwrap()
            .trim()
            .is_empty(),
        "a failed commit must not leave the index holding a whole workspace"
    );
}

// ---------------------------------------------------------------
// A `.gitignore` cannot drop the op log (A2)
// ---------------------------------------------------------------

/// Verified against real git: `.gitignore` in the work tree wins over
/// `$GIT_DIR/info/exclude`, and a negation there cannot win it back. A
/// user who keeps their notes in git and ignores `*.jsonl` used to get
/// snapshots with no op log in them, reported as successes.
#[test]
fn a_workspace_gitignore_cannot_drop_the_op_log() {
    let Some(fx) = fixture() else { return };
    std::fs::write(fx.ws.join(".gitignore"), "*.jsonl\nops/\npages/\n").unwrap();
    fx.page("a.md", "- hello\n");

    fx.repo.snapshot("first").unwrap().expect("has changes");

    let tracked = fx.tracked();
    assert!(
        tracked.contains("ops/ops-x.jsonl"),
        "the op log must be captured regardless of the user's ignore rules: {tracked}"
    );
    assert!(
        tracked.contains("pages/a.md"),
        "pages must be captured too: {tracked}"
    );
}

/// The guard behind the forced staging: if the op log ever fails to land
/// in the commit, that is an error, not a snapshot with a footnote.
#[test]
fn an_op_log_missing_from_the_commit_is_an_error() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- hello\n");
    // A commit that deliberately skips `ops/`, built with raw git rather
    // than through `snapshot` (which force-adds it).
    fx.repo.git(&["add", "--", "pages"]).unwrap();
    fx.repo
        .git(&["commit", "--quiet", "--no-verify", "-m", "pages only"])
        .unwrap();

    let err = fx
        .repo
        .verify_op_log_captured()
        .expect_err("a commit without the op log must not verify");
    match err {
        BackupError::OpLogNotCaptured { missing } => {
            assert_eq!(missing, vec!["ops/ops-x.jsonl".to_string()]);
        }
        other => panic!("expected OpLogNotCaptured, got {other:?}"),
    }
}

/// The same guard when nothing was ever committed: an empty history plus
/// op logs on disk is total exclusion, not an empty workspace.
#[test]
fn an_empty_history_with_op_logs_on_disk_is_an_error() {
    let Some(fx) = fixture() else { return };
    let err = fx
        .repo
        .verify_op_log_captured()
        .expect_err("no history at all cannot contain the op log");
    assert!(matches!(err, BackupError::OpLogNotCaptured { .. }));
}

// ---------------------------------------------------------------
// The automatic pass (A3)
// ---------------------------------------------------------------

/// `[backup] enabled` defaults to on, so the pass has to create the
/// repository itself — otherwise the default means "nothing happens
/// until you run a command you have never heard of".
#[test]
fn the_auto_pass_initializes_the_repository_on_first_run() {
    if !git_or_skip() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(ws.join("pages")).unwrap();
    std::fs::create_dir_all(ws.join("ops")).unwrap();
    std::fs::write(ws.join("ops").join("ops-x.jsonl"), "{}\n").unwrap();
    let repo = BackupRepo::at(tmp.path().join("backup.git"), &ws);
    assert!(!repo.is_initialized());

    let entry = auto::run_on(&repo, 30)
        .unwrap()
        .expect("the first pass has everything to commit");
    assert!(repo.is_initialized());
    assert!(entry.message.starts_with("outl snapshot "));
}

/// The interval is a floor derived from the git history, not a state
/// file — so it survives a restart and cannot drift out of sync with
/// what was actually committed.
#[test]
fn the_auto_pass_respects_the_interval_floor() {
    let Some(fx) = fixture() else { return };
    fx.page("a.md", "- one\n");
    auto::run_on(&fx.repo, 30).unwrap().expect("first snapshot");

    fx.page("a.md", "- two\n");
    assert!(
        auto::run_on(&fx.repo, 30).unwrap().is_none(),
        "a snapshot taken seconds ago means the 30-minute floor has not passed"
    );
    assert_eq!(fx.repo.list(10).unwrap().len(), 1);

    assert!(
        auto::run_on(&fx.repo, 0).unwrap().is_some(),
        "no floor means the changed workspace is snapshotted"
    );
    assert_eq!(fx.repo.list(10).unwrap().len(), 2);
}

/// `enabled = false` must not even resolve the device-local repository
/// path, let alone create it.
#[test]
fn a_disabled_config_takes_no_snapshot() {
    let tmp = TempDir::new().unwrap();
    assert!(maybe_snapshot(tmp.path(), false, 30).is_none());
}
