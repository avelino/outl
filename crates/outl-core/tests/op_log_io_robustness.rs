//! Regression tests for op-log I/O robustness (issues #154, #157).
//!
//! The op log is the source of truth. These pin three failure modes a synced,
//! crash-prone workspace hits in the wild — a sync tool's conflict copy of a
//! log file, a torn tail left by a crash mid-write, and a non-UTF8 byte in the
//! middle of a peer's file — and prove none of them can take down the open or
//! silently swallow a durable op.

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

/// #154 — a file-sync tool's conflict copy (`ops-<id> 2.jsonl` from iCloud,
/// `ops-<id>.sync-conflict-*.jsonl` from Syncthing) matches the `ops-*.jsonl`
/// shape but carries no valid actor id. It must be skipped, never abort the
/// whole workspace open.
#[test]
fn conflict_named_ops_file_does_not_break_open() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();

    {
        let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        s.append_op(&make_create(actor, 1)).unwrap();
    }

    // Drop iCloud- and Syncthing-style conflict copies next to the real file.
    let real = ops_dir.join(format!("ops-{actor}.jsonl"));
    std::fs::copy(&real, ops_dir.join(format!("ops-{actor} 2.jsonl"))).unwrap();
    std::fs::copy(
        &real,
        ops_dir.join(format!(
            "ops-{actor}.sync-conflict-20260712-000000-ABCDEFG.jsonl"
        )),
    )
    .unwrap();

    // Before the fix this returned Err and the app couldn't open at all.
    let s = JsonlStorage::open(ops_dir, actor)
        .expect("a conflict-named file must not fail the workspace open");
    assert_eq!(
        s.all_ops().unwrap().len(),
        1,
        "only the real log's op should load; conflict copies are skipped"
    );
}

/// #157 — a crash mid-write can leave a partial last line with no newline.
/// The next append must not glue its JSON onto that fragment (which would lose
/// both); it heals the tail with a newline so the good op lands as its own
/// record and survives the next load.
#[test]
fn torn_tail_is_healed_not_glued_on_next_append() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let actor = ActorId::new();
    let path = ops_dir.join(format!("ops-{actor}.jsonl"));

    // Simulate a crash mid-write: a partial record with NO trailing newline.
    std::fs::write(&path, b"{\"ts\":{\"physical").unwrap();

    {
        let mut s = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        s.append_op(&make_create(actor, 5)).unwrap();
    }

    // Raw bytes: the torn fragment must not be glued onto the good op.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("physical{"),
        "torn fragment was glued onto the appended op: {raw:?}"
    );

    // The good op after a torn tail must load; the fragment is skipped.
    let s = JsonlStorage::open(ops_dir, actor).unwrap();
    assert_eq!(
        s.all_ops().unwrap().len(),
        1,
        "the op appended after a torn tail must survive reload"
    );
}

/// #157 — a non-UTF8 byte partway through a file (a partial sync can leave one)
/// must cost a single skipped line, not every op after it. Before the fix the
/// reader broke out of the loop and dropped the rest of the file.
#[test]
fn non_utf8_line_does_not_truncate_the_rest_of_the_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ops_dir = tmp.path().join("ops");
    std::fs::create_dir_all(&ops_dir).unwrap();
    let peer = ActorId::new();
    let reader_actor = ActorId::new();

    let op1 = make_create(peer, 1);
    let op2 = make_create(peer, 2);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(serde_json::to_string(&op1).unwrap().as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(&[0xff, 0xfe, 0x00, 0x80]); // invalid UTF-8
    bytes.push(b'\n');
    bytes.extend_from_slice(serde_json::to_string(&op2).unwrap().as_bytes());
    bytes.push(b'\n');
    std::fs::write(ops_dir.join(format!("ops-{peer}.jsonl")), &bytes).unwrap();

    // Open as a different actor: the peer file is a read-only mirror.
    let s = JsonlStorage::open(ops_dir, reader_actor).unwrap();
    assert_eq!(
        s.all_ops().unwrap().len(),
        2,
        "both good ops must load even with a non-UTF8 line between them"
    );
}
