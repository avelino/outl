//! Diagnostic: what does one commit's `reconcile_md` cost, and how many
//! ops does editing a single block emit? Run:
//! `cargo test -p outl-actions --release --test reconcile_bench -- --ignored --nocapture`

use std::path::Path;
use std::time::Instant;

use outl_actions::{append_block, apply_page_md_with_sidecar, open_journal, page_meta};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

#[test]
#[ignore = "diagnostic; run with --release --nocapture"]
fn reconcile_cost_of_one_edit() {
    let n = 11usize;
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let (slug, md_path) = {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();
        let day = open_journal(
            &mut w,
            &hlc,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        )
        .unwrap();
        for b in 0..n {
            append_block(
                &mut w,
                &hlc,
                Some(day),
                Some(&format!("line {b} body text")),
            )
            .unwrap();
        }
        apply_page_md_with_sidecar(&w, &root, day).unwrap();
        let meta = page_meta(&w, day).unwrap();
        (meta.slug.clone(), outl_actions::page_md_path(&root, &meta))
    };

    // Reopen from disk (lazy text, #179) — mirrors a real session.
    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();
    let _ = slug;

    println!("\n=== reconcile cost, {n}-block journal ===");

    // Simulate a commit: rewrite the `.md` with ONE line changed, then
    // reconcile — exactly what the TUI's `save` does.
    let original = std::fs::read_to_string(&md_path).unwrap();
    let edited = original.replacen("line 0 body text", "line 0 EDITED body text", 1);
    assert_ne!(original, edited, "edit must change the md");
    std::fs::write(&md_path, &edited).unwrap();

    let t = Instant::now();
    let report = outl_md::reconcile::reconcile_md(&mut w, &hlc, &md_path, None).unwrap();
    let elapsed = t.elapsed();
    println!(
        "one-block edit: ops_applied={} reconcile={:?}  ({:.2} ms/op)",
        report.ops_applied,
        elapsed,
        elapsed.as_secs_f64() * 1000.0 / (report.ops_applied.max(1) as f64),
    );

    // A second identical-content commit (hash unchanged → short-circuit).
    let t = Instant::now();
    let report2 = outl_md::reconcile::reconcile_md(&mut w, &hlc, &md_path, None).unwrap();
    println!(
        "no-op commit (same hash): ops_applied={} reconcile={:?}",
        report2.ops_applied,
        t.elapsed()
    );

    let _ = Path::new("");
}

/// Regression guard (NOT ignored): a commit that touches one block must
/// emit a handful of ops, not ~2 per block in the whole page.
///
/// The diff defensively re-emitted `Create` + `Move` for every block on
/// every commit, and allocated fresh fractional positions each time, so
/// every `Move` looked like a real reorder. On an 11-block page a
/// one-block edit fsynced 23 ops (the slow TUI commit) and the op log
/// grew by the whole page every keystroke (the slow boot). Reusing the
/// current positions + filtering no-ops fixes it.
#[test]
fn a_one_block_commit_does_not_rewrite_the_whole_page() {
    let n = 12usize;
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let md_path = {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();
        let day = open_journal(
            &mut w,
            &hlc,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        )
        .unwrap();
        for b in 0..n {
            append_block(&mut w, &hlc, Some(day), Some(&format!("line {b} body"))).unwrap();
        }
        apply_page_md_with_sidecar(&w, &root, day).unwrap();
        outl_actions::page_md_path(&root, &page_meta(&w, day).unwrap())
    };

    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // Edit one block's text.
    let original = std::fs::read_to_string(&md_path).unwrap();
    std::fs::write(
        &md_path,
        original.replacen("line 3 body", "line 3 EDITED", 1),
    )
    .unwrap();
    let report = outl_md::reconcile::reconcile_md(&mut w, &hlc, &md_path, None).unwrap();
    assert!(
        report.ops_applied <= 2,
        "editing one block emitted {} ops — the page-churn regression is back",
        report.ops_applied
    );

    // Delete one block.
    let cur = std::fs::read_to_string(&md_path).unwrap();
    let without = cur
        .lines()
        .filter(|l| !l.contains("line 7 body"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&md_path, format!("{without}\n")).unwrap();
    let report = outl_md::reconcile::reconcile_md(&mut w, &hlc, &md_path, None).unwrap();
    assert!(
        report.ops_applied <= 3,
        "deleting one block emitted {} ops — the page-churn regression is back",
        report.ops_applied
    );
}
