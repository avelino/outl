//! Benchmarks for the block-level index — `resolve_block_ref` lookup
//! and `search_block_text` autocomplete on synthetic workspaces of the
//! sizes target by `#12` (10k pages, 100k blocks).
//!
//! Run:
//! ```text
//! cargo bench -p outl-md --bench block_index
//! ```
//!
//! The contract `#12` validates: lookup stays under 1ms P99 even at
//! 100k indexed blocks. Anything significantly above that suggests
//! a regression in the HashMap path or that something now does
//! linear work where O(1) was promised.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use outl_core::id::NodeId;
use outl_md::block_index::BlockIndex;
use outl_md::parse::OutlineNode;
use outl_md::sidecar::{content_hash, derive_ref_handle, SidecarBlock};
use std::hint::black_box;
use std::path::PathBuf;

/// Build a `BlockIndex` populated with `n` blocks spread across
/// `pages_count` synthetic pages — closer to real workspace shape
/// than a single page with N children would be.
fn build_index(blocks_total: usize, pages_count: usize) -> (BlockIndex, Vec<String>) {
    let mut idx = BlockIndex::default();
    let mut handles = Vec::with_capacity(blocks_total);
    let per_page = blocks_total.div_ceil(pages_count);
    for p in 0..pages_count {
        let slug = format!("page-{p}");
        let path = PathBuf::from(format!("pages/{slug}.md"));
        let mut ast: Vec<OutlineNode> = Vec::with_capacity(per_page);
        let mut sidecar: Vec<SidecarBlock> = Vec::with_capacity(per_page);
        for b in 0..per_page {
            if handles.len() >= blocks_total {
                break;
            }
            let id = NodeId::new();
            let text = format!("page {p} block {b} — decide backend item");
            let handle = derive_ref_handle(id);
            handles.push(handle.clone());
            sidecar.push(SidecarBlock {
                id,
                line: b + 1,
                indent: 0,
                content_hash: content_hash(&text),
                ref_handle: handle,
            });
            ast.push(OutlineNode {
                text,
                properties: Vec::new(),
                children: Vec::new(),
            });
        }
        idx.collect_page_blocks(&slug, &path, &ast, &sidecar);
    }
    (idx, handles)
}

fn bench_resolve(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_index_resolve");
    for &n in &[1_000usize, 10_000, 100_000] {
        let pages = (n / 10).max(1);
        let (idx, handles) = build_index(n, pages);
        let target = handles[handles.len() / 2].clone();
        group.bench_with_input(BenchmarkId::from_parameter(n), &target, |bencher, t| {
            bencher.iter(|| {
                let e = idx.resolve(black_box(t));
                black_box(e);
            });
        });
    }
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_index_search");
    // Search costs scale with workspace size; cap samples on the
    // large variant so the bench finishes in under a minute on a
    // dev laptop.
    group.sample_size(30);
    for &n in &[1_000usize, 10_000, 100_000] {
        let pages = (n / 10).max(1);
        let (idx, _) = build_index(n, pages);
        group.bench_with_input(BenchmarkId::from_parameter(n), &"backend", |bencher, q| {
            bencher.iter(|| {
                let hits = idx.search_text(black_box(q), 8);
                black_box(hits);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_resolve, bench_search);
criterion_main!(benches);
