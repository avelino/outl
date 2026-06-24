//! Local helpers for the `chaos.rs` concurrency suite.
//!
//! Kept in a sibling module (NOT `common/mod.rs`, which the loopback
//! integration suites own) so the chaos-specific seeded RNG, the raw-bytes
//! corruption assertion, and the HLC-offset seeder live next to the only suite
//! that uses them without bloating `chaos.rs` past the file-size guard.
//!
//! `#![allow(dead_code)]` because each `tests/*.rs` compiles this module as its
//! own crate; only `chaos.rs` `mod chaos_helpers;`s it, so the integration
//! harness would otherwise flag every item as unused.
#![allow(dead_code)]

use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::LogOp;

/// Current wall-clock time in milliseconds since the Unix epoch.
///
/// Inlined (rather than reused from `common`) so this module never loads
/// `common/mod.rs` a second time — `chaos.rs` already owns the single `mod
/// common;`, and a duplicate load trips clippy's `duplicate_mod`.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_millis() as u64
}

/// Tiny seeded xorshift64* PRNG.
///
/// We deliberately do NOT pull `rand` into the test: a self-contained,
/// fully-specified generator makes every shuffle and every duplicate-count
/// reproducible from the seed alone, which is the whole point of a non-flaky
/// chaos suite.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the zero state (xorshift's fixed point).
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, n)` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// In-place Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.below(i + 1);
            slice.swap(i, j);
        }
    }
}

/// Read `ops-<actor>.jsonl` bytes directly and assert every NON-EMPTY physical
/// line is exactly ONE self-delimiting JSON value.
///
/// This bypasses `JsonlStorage`'s read-side glued-op recovery on purpose: a
/// recovered `…}}}{…` line still round-trips through `all_ops`, so only the raw
/// bytes reveal whether the append lock actually held. A line that decodes into
/// two-or-more values is the `}}}{` corruption; a line that fails to decode at
/// all is a torn write.
pub fn assert_every_line_is_one_json_value(ops_dir: &Path, actor: ActorId) {
    let path = ops_dir.join(format!("ops-{actor}.jsonl"));
    let bytes = std::fs::read(&path).expect("read ops file bytes");
    let text = String::from_utf8(bytes).expect("ops file is utf8");
    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let count = serde_json::Deserializer::from_str(line)
            .into_iter::<serde_json::Value>()
            .count();
        assert_eq!(
            count,
            1,
            "ops file {} line {} is not exactly one JSON value \
             (glued/torn append — the append lock failed): {:?}",
            path.display(),
            lineno + 1,
            line
        );
    }
}

/// Append `count` `Op::Create` ops authored by `actor`, starting at HLC counter
/// `start_counter`, returning the new node ids.
///
/// Like `common::seed_ops` but lets the caller offset the HLC counter so several
/// disjoint batches authored by the SAME actor stay strictly monotonic (the
/// responder's last-ts-per-actor delta sync keys on that ordering). Opens its
/// own `JsonlStorage`, so it goes through the exact production append path.
pub fn seed_ops_from(
    workspace_root: &Path,
    actor: ActorId,
    count: u32,
    start_counter: u32,
) -> Vec<NodeId> {
    let ops_dir = workspace_root.join("ops");
    let mut storage = JsonlStorage::open(ops_dir, actor).expect("open storage");
    let base_ms = now_ms();
    let mut nodes = Vec::with_capacity(count as usize);
    for i in 0..count {
        let node = NodeId::new();
        nodes.push(node);
        let c = start_counter + i;
        let op = LogOp {
            ts: Hlc::new(base_ms + c as u64, c, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        };
        storage.append_op(&op).expect("append op");
    }
    nodes
}
