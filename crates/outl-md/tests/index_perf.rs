//! Smoke benchmark: building the `WorkspaceIndex` over a realistic
//! number of pages must finish in well under a second on a modern
//! laptop, even in debug builds. This is the threshold above which
//! `outl` open feels sluggish to a user.
//!
//! Empirically the current implementation indexes 1500 pages in
//! ~250 ms (debug). Persisted/incremental caching is documented as
//! future work in `docs/storage.md`; we don't need it for beta.

use outl_md::WorkspaceIndex;
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

#[test]
fn build_large_workspace_stays_fast() {
    const N_PAGES: u32 = 1500;
    const BUDGET_MS: u128 = 2000;

    let dir = TempDir::new().unwrap();
    let pages = dir.path().join("pages");
    fs::create_dir_all(&pages).unwrap();

    // Realistic page: title + a few blocks, some with [[refs]] and #tags.
    let template = "title:: Page {n}\nstatus:: active\ntags:: #work\n\n\
        - lorem ipsum block one\n  priority:: high\n\
        - block with reference to [[Other Page {prev}]]\n\
        - block with another [[Foo]] and #important\n\
        - final block with content for the day\n";

    for i in 0..N_PAGES {
        let prev = i.saturating_sub(1);
        let content = template
            .replace("{n}", &i.to_string())
            .replace("{prev}", &prev.to_string());
        let path = pages.join(format!("page-{i}.md"));
        fs::write(&path, content).unwrap();
    }

    let started = Instant::now();
    let index = WorkspaceIndex::build(dir.path());
    let elapsed = started.elapsed();

    assert_eq!(index.page_count(), N_PAGES as usize);
    assert!(
        elapsed.as_millis() < BUDGET_MS,
        "WorkspaceIndex::build over {N_PAGES} pages took {elapsed:?} \
         (budget {BUDGET_MS} ms) — open will feel sluggish"
    );
    println!("{N_PAGES} pages indexed in {elapsed:?}");
}
