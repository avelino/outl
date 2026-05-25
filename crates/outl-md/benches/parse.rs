//! Benchmarks `outl_md::parse` across realistic page sizes.
//!
//! Tells us how much of the index-build cost is parsing vs walking,
//! and tracks regressions when we touch `parse.rs` (which has grown
//! some weight around continuation lines and fenced code).
//!
//! Run:
//! ```text
//! cargo bench -p outl-md --bench parse
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use outl_md::parse::parse;

#[path = "common.rs"]
mod common;

/// Generate one page of `blocks` bullets with a mix of inline tokens
/// and an embedded fenced code block — representative of a "real"
/// notebook page, not a degenerate string of bullets.
fn synth_page(blocks: usize) -> String {
    let mut out = String::from("title:: Bench Page\nicon:: 🧪\n\n");
    for b in 0..blocks {
        match b % 7 {
            0 => out.push_str("- ```lisp\n  (+ 1 2)\n  ```\n"),
            1 => out.push_str("- header item with [[Page X]] and #tag-y\n"),
            2 => out.push_str("- nested example\n  - child block\n  - another child\n"),
            3 => out.push_str("- **bold** + *italic* + `code` line\n"),
            4 => out.push_str("- multi-line block\n  continuation here\n  third line\n"),
            5 => out.push_str(&format!("- TODO finish task #{b}\n")),
            _ => out.push_str(&format!("- plain bullet number {b}\n")),
        }
    }
    out
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for &blocks in &[10usize, 100, 500, 2000] {
        let src = synth_page(blocks);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{blocks}_blocks")),
            &src,
            |b, s| {
                b.iter(|| {
                    let p = parse(black_box(s));
                    black_box(p.blocks.len())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
