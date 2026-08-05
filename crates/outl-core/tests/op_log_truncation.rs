//! Regression tests for the two ways a damaged op log used to shrink the
//! workspace **without saying so**.
//!
//! Both are the same class of bug as the torn-tail / non-UTF8 cases in
//! `op_log_io_robustness.rs`, but they sit on the two paths that file
//! doesn't cover: the sequential full replay, and the index-driven read
//! that feeds the snapshot-boot delta.
//!
//! 1. A single unreadable line used to `break` the sequential replay, so
//!    every op *after* it was discarded and boot carried on as if the
//!    truncated tree were the whole workspace.
//! 2. An op the offset index lists but that can't be read back used to be
//!    dropped from the result set silently. That is worse than it sounds:
//!    the next snapshot's cutoff is derived from the index, so the
//!    omission gets recorded as "already folded in" and no later boot
//!    ever replays that op again — permanent loss, with the bytes still
//!    sitting on disk.

use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::{JsonlStorage, Storage};

fn make_create(actor: ActorId, physical: u64) -> LogOp {
    LogOp {
        ts: Hlc::new(physical, 0, actor),
        actor,
        op: Op::Create {
            node: NodeId::new(),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    }
}

/// A corrupt line in the **middle** of the log must cost exactly that
/// line — never the entire remainder of the file.
///
/// This is the one that mattered most: an unreadable line at op 5_000 of
/// 200_000 silently truncated the tree to the first 5_000 ops, and the
/// app opened looking like a workspace that had lost months of work.
#[test]
fn corrupt_middle_line_does_not_discard_the_rest_of_the_log() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();

    {
        let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        for i in 1..=9 {
            s.append_op(&make_create(actor, i)).unwrap();
        }
    }

    // Replace the middle line with garbage, keeping the line count.
    let path = ops_dir.join(format!("ops-{actor}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 9, "fixture should have one line per op");
    lines[4] = "{ this is not valid json".to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let s = JsonlStorage::open(ops_dir, actor).unwrap();
    let ops = s.all_ops().unwrap();

    // 8 of 9 survive: we lose the damaged line and nothing else. The old
    // `break` returned 4.
    assert_eq!(
        ops.len(),
        8,
        "a corrupt line must cost one op, not every op after it"
    );

    // Specifically: ops *after* the damage are present.
    let physicals: Vec<u64> = ops.iter().map(|o| o.ts.physical_ms).collect();
    for expected in [6, 7, 8, 9] {
        assert!(
            physicals.contains(&expected),
            "op {expected} came after the corrupt line and must still be replayed; got {physicals:?}"
        );
    }
}

/// Same guarantee for a line that is valid UTF-8 and valid JSON but not a
/// `LogOp` — the shape a partially-synced file lands in.
#[test]
fn unparseable_op_line_does_not_discard_the_rest_of_the_log() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();

    {
        let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        for i in 1..=5 {
            s.append_op(&make_create(actor, i)).unwrap();
        }
    }

    let path = ops_dir.join(format!("ops-{actor}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[1] = r#"{"unexpected":"shape"}"#.to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let s = JsonlStorage::open(ops_dir, actor).unwrap();
    let ops = s.all_ops().unwrap();
    assert_eq!(ops.len(), 4);
    let physicals: Vec<u64> = ops.iter().map(|o| o.ts.physical_ms).collect();
    assert!(
        physicals.contains(&5),
        "the tail must survive: {physicals:?}"
    );
}

/// An op the offset index knows about but whose bytes cannot be read back
/// must surface as an **error**, not as a shorter result set.
///
/// Truncating the file behind the index reproduces what a partial sync or
/// a damaged sector does: the index (built on a previous, complete boot,
/// or from the persisted `.idx` sidecar) still lists offsets past the end
/// of the file.
///
/// A short read here is indistinguishable from a healthy short read, and
/// `build_snapshot_body` derives the next snapshot's cutoff from the
/// index — so silently dropping the op writes a snapshot that claims to
/// have folded it in. That is the permanent-loss path this pins shut.
#[test]
fn op_listed_in_index_but_unreadable_is_an_error_not_a_short_read() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();

    let first = make_create(actor, 1);
    let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
    s.append_op(&first).unwrap();
    for i in 2..=6 {
        s.append_op(&make_create(actor, i)).unwrap();
    }

    // The in-memory index still points at all six ops; the file now holds
    // only the first line. Reads driven off the index must notice.
    let path = ops_dir.join(format!("ops-{actor}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();
    let first_line_end = text.find('\n').unwrap() + 1;
    std::fs::write(&path, &text[..first_line_end]).unwrap();

    // Defeat the warm LRU so the read actually has to touch disk.
    s.resize_cache(1);

    let err = s
        .ops_since(Hlc::new(0, 0, actor))
        .expect_err("an op in the index that disk won't return is corruption, not an empty tail");
    let msg = err.to_string();
    assert!(
        msg.contains("offset index"),
        "the error must say the index and the file disagree, got: {msg}"
    );
}

/// `ops_for_actor` is the other index-driven collector and must behave the
/// same way — it feeds peer sync, so a short read there ships an
/// incomplete log to another device.
#[test]
fn ops_for_actor_surfaces_missing_ops_instead_of_dropping_them() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();

    let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
    for i in 1..=4 {
        s.append_op(&make_create(actor, i)).unwrap();
    }

    let path = ops_dir.join(format!("ops-{actor}.jsonl"));
    std::fs::write(&path, "").unwrap();
    s.resize_cache(1);

    assert!(
        s.ops_for_actor(actor).is_err(),
        "shipping a silently-short op set to a peer is how divergence starts"
    );
}
