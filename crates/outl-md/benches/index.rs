//! Benchmarks `WorkspaceIndex::build` across synthetic workspaces.
//!
//! What we're measuring: the cost of the "scan everything" pass that
//! `outl-tui::App::new` *used* to run on the critical path. Since the
//! TUI now spawns this on a worker thread, these numbers tell us
//! roughly how long the user has to wait for backlinks / icons /
//! autocomplete candidates to populate after boot.
//!
//! Run:
//! ```text
//! cargo bench -p outl-md --bench index
//! ```
//!
//! HTML reports land in `target/criterion/`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use outl_md::index::WorkspaceIndex;
use std::hint::black_box;

#[path = "common.rs"]
mod common;

use common::{synth_workspace, workspace_bytes, WorkspaceShape};

fn bench_index_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("workspace_index_build");
    // Keep large workspaces honest — small ones take μs, big ones can
    // take seconds. Criterion's default 100-sample size would make
    // the `large` case take minutes; cap it.
    group.sample_size(20);

    // All four tiers ship in the binary; `cargo bench -- small`
    // (or `medium` / `large` / `xlarge`) filters to one via
    // criterion's substring matcher. CI uses that to split the cheap
    // tiers (PR) from the 10k-file tier (scheduled).
    let shapes = [
        WorkspaceShape::small(),
        WorkspaceShape::medium(),
        WorkspaceShape::large(),
        WorkspaceShape::xlarge(),
    ];
    for shape in shapes {
        let dir = synth_workspace(shape);
        let bytes = workspace_bytes(dir.path());
        group.throughput(Throughput::Bytes(bytes));

        // Label format: `<tier>_<pages>p_<journals>j_<blocks>b`. The
        // leading tier word makes filtering trivial; the rest stays
        // legible in criterion's HTML report.
        let label = format!(
            "{}_{}p_{}j_{}b",
            shape.tier, shape.pages, shape.journals, shape.blocks_per_page
        );
        group.bench_with_input(BenchmarkId::from_parameter(label), &dir, |b, d| {
            b.iter(|| {
                let idx = WorkspaceIndex::build(black_box(d.path()));
                // Force the result to not be optimized away; cheapest
                // observable side effect of "the build actually ran".
                black_box(idx.page_count())
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_index_build);
criterion_main!(benches);
