//! A page whose `.md` holds content that exists in no op stops being
//! re-projected (root `CLAUDE.md` invariant 8) — correct, and until now
//! completely silent on the GUI clients: the refusal died in a
//! `tracing::warn!`, so the page simply froze showing old text with
//! nothing on screen to say why or what to do. On iOS there is not even
//! an `outl` binary to run the recovery with.
//!
//! These pin the two halves that make it visible without breaking the
//! hottest path in the app: the condition reaches the reply, and the
//! page still opens.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::page::open_page_by_slug;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Minimal [`AppHost`]: the open path only needs a workspace, an HLC and
/// a storage root. No projection writer on purpose — this exercises the
/// synchronous path, where the view is read back off the `.md`.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
}

/// A projected page plus the host that serves it, sharing one workspace.
fn host_with_page(initial: &str) -> (TempDir, TestHost, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some(initial)).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = tmp.path().join("pages").join("notes.md");

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host, md_path)
}

/// Re-stamp the sidecar so its `last_synced_hash` agrees with whatever
/// is on disk — the exact state the guard exists for: outl wrote these
/// bytes last, and the op log still does not hold all of them.
fn restamp_sidecar_as_faithful(md_path: &PathBuf) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

#[test]
fn opening_a_page_whose_md_outran_the_log_tells_the_user_so() {
    let (_tmp, host, md_path) = host_with_page("first");
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let view = open_page_by_slug(&host, "notes".to_string()).expect("the page must still open");

    let ahead = view
        .md_ahead_of_log
        .expect("the refusal must reach the reply, not stop at a log line");
    assert_eq!(ahead.lines, 1);
    assert!(
        ahead.sample.contains("only ever on disk"),
        "the notice must name the content at risk, got {:?}",
        ahead.sample
    );
    assert!(
        ahead.path.ends_with("notes.md"),
        "the notice must name the file, got {:?}",
        ahead.path
    );
}

/// The hard constraint: this is the hottest path in the app. A condition
/// that used to be a warning must not become a failed open — the user
/// would trade a stale page for no page at all.
#[test]
fn the_page_still_opens_and_still_shows_what_is_on_disk() {
    let (_tmp, host, md_path) = host_with_page("first");
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let view = open_page_by_slug(&host, "notes".to_string()).expect("the page must still open");

    let rendered: Vec<&str> = view.outline.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(
        rendered,
        vec!["first", "only ever on disk"],
        "the view must keep showing the file as it stands — the guard withheld a write, not the page"
    );
    // And the bytes are untouched: the whole point of refusing.
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- first\n- only ever on disk\n"
    );
}

/// A healthy page must stay quiet. A banner that shows up on pages with
/// nothing wrong is a banner users learn to dismiss without reading, and
/// then the one that matters goes unread too.
#[test]
fn a_healthy_page_carries_no_notice() {
    let (_tmp, host, _md_path) = host_with_page("first");

    let view = open_page_by_slug(&host, "notes".to_string()).unwrap();

    assert!(view.md_ahead_of_log.is_none());
}

/// An open reply says so, and that is what lets a client clear a banner
/// the recovery already fixed.
///
/// The clients cannot read `md_ahead_of_log` off every reply — a
/// mutation is built from the tree and never carries it, so doing that
/// drops the warning on the user's first edit. They hold it instead, and
/// without a positive "this reply ran the check" they hold it forever:
/// the page keeps wearing "isn't syncing" after `outl reconcile
/// --ahead-of-log` fixed it, which is the same silence in the other
/// direction.
#[test]
fn an_open_reply_marks_itself_as_having_run_the_check() {
    let (_tmp, host, md_path) = host_with_page("first");

    let healthy = open_page_by_slug(&host, "notes".to_string()).unwrap();
    assert!(
        healthy.md_ahead_of_log_checked,
        "a healthy open still ran the check — absent must mean healthy, not unknown"
    );

    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);
    let ahead = open_page_by_slug(&host, "notes".to_string()).unwrap();
    assert!(
        ahead.md_ahead_of_log_checked && ahead.md_ahead_of_log.is_some(),
        "the refusing open must carry both halves"
    );
}
