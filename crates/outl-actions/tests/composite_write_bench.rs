//! Baseline benchmark for composite writes through `outl-actions`.
//!
//! Ignored by default — measures the current (pre-fsync-optimization)
//! cost of the two composite mutation shapes the MCP / batch surface
//! performs most: "create a page with content + properties" and "full
//! replace of an existing page". Run with:
//!
//! ```sh
//! cargo test -p outl-actions --release --test composite_write_bench -- --ignored --nocapture
//! ```
//!
//! Output is plain text (`--nocapture` shows it inline). Not run in
//! CI — the numbers are a diagnostic baseline, not a correctness
//! guard.

use std::time::Instant;

use outl_actions::{
    append_forest, children_of, delete, open_or_create_page, set_property, BlockTreeSpec, PageKind,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::property::PropValue;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

const ITERS: usize = 20;

/// ~7 blocks with 2 levels of nesting (3 roots, 3 children, 1 grandchild).
fn sample_forest(tag: usize) -> Vec<BlockTreeSpec> {
    let leaf = |text: String| BlockTreeSpec {
        text,
        children: vec![],
    };
    vec![
        BlockTreeSpec {
            text: format!("root one {tag}"),
            children: vec![
                leaf(format!("child a {tag}")),
                BlockTreeSpec {
                    text: format!("child b {tag}"),
                    children: vec![leaf(format!("grandchild {tag}"))],
                },
            ],
        },
        BlockTreeSpec {
            text: format!("root two {tag}"),
            children: vec![leaf(format!("child c {tag}"))],
        },
        leaf(format!("root three {tag}")),
    ]
}

#[test]
#[ignore = "baseline diagnostic; run with --release --nocapture"]
fn bench_composite_writes() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let mut w = Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    println!("\n=== composite write baseline ({ITERS} iterations each) ===");

    // Scenario A: new page + append_forest (~7 blocks, 2 nesting levels)
    // + 3 set_property. A fresh page per iteration.
    let start = Instant::now();
    for i in 0..ITERS {
        let slug = format!("bench-create-{i}");
        let page = open_or_create_page(&mut w, &hlc, &slug, &format!("Create {i}"), PageKind::Page)
            .unwrap();
        append_forest(&mut w, &hlc, page, &sample_forest(i)).unwrap();
        for key in ["status", "owner", "priority"] {
            set_property(
                &mut w,
                &hlc,
                page,
                key,
                Some(PropValue::Text(format!("{key}-{i}"))),
            )
            .unwrap();
        }
    }
    let elapsed_a = start.elapsed();
    println!(
        "A) page create + 7-block forest + 3 props: total {:?}  ({:.2} ms/op)",
        elapsed_a,
        elapsed_a.as_secs_f64() * 1000.0 / ITERS as f64,
    );

    // Scenario B: full replace of an existing page — delete every
    // top-level block, then append_forest a fresh ~7-block forest.
    // Page setup (create + initial fill) happens outside the timed
    // window; only the replace is measured.
    let mut pages = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let slug = format!("bench-replace-{i}");
        let page =
            open_or_create_page(&mut w, &hlc, &slug, &format!("Replace {i}"), PageKind::Page)
                .unwrap();
        append_forest(&mut w, &hlc, page, &sample_forest(i)).unwrap();
        pages.push(page);
    }
    let start = Instant::now();
    for (i, page) in pages.iter().enumerate() {
        for (child, _) in children_of(&w, *page) {
            delete(&mut w, &hlc, child).unwrap();
        }
        append_forest(&mut w, &hlc, *page, &sample_forest(ITERS + i)).unwrap();
    }
    let elapsed_b = start.elapsed();
    println!(
        "B) full replace (delete blocks + 7-block forest): total {:?}  ({:.2} ms/op)",
        elapsed_b,
        elapsed_b.as_secs_f64() * 1000.0 / ITERS as f64,
    );

    // Scenario C: same logical write as A, but the caller wraps the
    // whole thing in one outer batch — the "1 logical mutation =
    // 1 fsync" shape composite consumers (MCP, embedders) should use.
    let start = Instant::now();
    for i in 0..ITERS {
        let slug = format!("bench-batched-create-{i}");
        let mut batch = w.begin_batch();
        let page = open_or_create_page(
            &mut batch,
            &hlc,
            &slug,
            &format!("Batched create {i}"),
            PageKind::Page,
        )
        .unwrap();
        append_forest(&mut batch, &hlc, page, &sample_forest(i)).unwrap();
        for key in ["status", "owner", "priority"] {
            set_property(
                &mut batch,
                &hlc,
                page,
                key,
                Some(PropValue::Text(format!("{key}-{i}"))),
            )
            .unwrap();
        }
        batch.commit().unwrap();
    }
    let elapsed_c = start.elapsed();
    println!(
        "C) scenario A inside one outer begin_batch: total {:?}  ({:.2} ms/op)",
        elapsed_c,
        elapsed_c.as_secs_f64() * 1000.0 / ITERS as f64,
    );

    // Scenario D: same logical write as B under one outer batch.
    let mut pages = Vec::with_capacity(ITERS);
    for i in 0..ITERS {
        let slug = format!("bench-batched-replace-{i}");
        let page = open_or_create_page(
            &mut w,
            &hlc,
            &slug,
            &format!("Batched replace {i}"),
            PageKind::Page,
        )
        .unwrap();
        append_forest(&mut w, &hlc, page, &sample_forest(i)).unwrap();
        pages.push(page);
    }
    let start = Instant::now();
    for (i, page) in pages.iter().enumerate() {
        let mut batch = w.begin_batch();
        for (child, _) in children_of(&batch, *page) {
            delete(&mut batch, &hlc, child).unwrap();
        }
        append_forest(&mut batch, &hlc, *page, &sample_forest(2 * ITERS + i)).unwrap();
        batch.commit().unwrap();
    }
    let elapsed_d = start.elapsed();
    println!(
        "D) scenario B inside one outer begin_batch: total {:?}  ({:.2} ms/op)",
        elapsed_d,
        elapsed_d.as_secs_f64() * 1000.0 / ITERS as f64,
    );
}
