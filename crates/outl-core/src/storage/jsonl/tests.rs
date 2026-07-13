use super::*;
use crate::fractional::Fractional;
use crate::hlc::HlcGenerator;
use crate::op::Op;
use crate::storage::Storage;
use tempfile::TempDir;

fn mk_create(g: &HlcGenerator) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op: Op::Create {
            node: NodeId::new(),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    }
}

#[test]
fn roundtrips_through_disk() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    assert_eq!(storage.all_ops().unwrap().len(), 0);

    let op = mk_create(&g);
    storage.append_op(&op).unwrap();

    // Reload from disk: cache must repopulate from the file.
    let reopened = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    assert_eq!(reopened.all_ops().unwrap().len(), 1);
}

#[test]
fn rejects_foreign_actor_writes() {
    let tmp = TempDir::new().unwrap();
    let us = ActorId::new();
    let them = ActorId::new();

    let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), us).unwrap();
    let g = HlcGenerator::new(them);
    let op = mk_create(&g);
    assert!(storage.append_op(&op).is_err());
}

#[test]
fn recovers_glued_ops_on_one_line() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let a = mk_create(&g);
    let b = mk_create(&g);
    let line_a = serde_json::to_string(&a).unwrap();
    let line_b = serde_json::to_string(&b).unwrap();

    let glued = format!("{line_a}{line_b}");
    assert!(glued.contains("}{"), "fixture must be glued JSON objects");

    let path = tmp.path().join(format!("ops-{actor}.jsonl"));
    let healthy = serde_json::to_string(&mk_create(&g)).unwrap();
    std::fs::write(&path, format!("{healthy}\n{glued}\n\n")).unwrap();

    let storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    assert_eq!(storage.all_ops().unwrap().len(), 3);

    let recovered = parse_log_line(&glued).unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[0].ts, a.ts);
    assert_eq!(recovered[1].ts, b.ts);
}

#[test]
fn merges_ops_from_multiple_actor_files() {
    let tmp = TempDir::new().unwrap();
    let me = ActorId::new();
    let peer = ActorId::new();

    {
        let mut peer_storage = JsonlStorage::open(tmp.path().to_path_buf(), peer).unwrap();
        let g = HlcGenerator::new(peer);
        let op = mk_create(&g);
        peer_storage.append_op(&op).unwrap();
    }

    let mine = JsonlStorage::open(tmp.path().to_path_buf(), me).unwrap();
    assert_eq!(mine.all_ops().unwrap().len(), 1);
}

/// Bounded LRU should keep RSS constant: ops past the cap are
/// evicted from RAM, but the offset index still knows about them
/// (visible via `last_ts_per_actor`) so they can be rebuilt from
/// disk on demand by future work.
#[test]
fn bounded_lru_evicts_old_ops() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open_with_cap(tmp.path().to_path_buf(), actor, 3).unwrap();
    let ops: Vec<LogOp> = (0..5).map(|_| mk_create(&g)).collect();
    for op in &ops {
        storage.append_op(op).unwrap();
    }

    // Only the last 3 of the 5 ops fit in the LRU.
    let cached = storage.all_ops().unwrap();
    assert_eq!(cached.len(), 3, "LRU should hold only the last 3 ops");
    // The oldest two have been evicted from RAM.
    assert!(!cached.iter().any(|o| o.ts == ops[0].ts));
    assert!(!cached.iter().any(|o| o.ts == ops[1].ts));
    // The newest three are still resident.
    assert!(cached.iter().any(|o| o.ts == ops[2].ts));
    assert!(cached.iter().any(|o| o.ts == ops[3].ts));
    assert!(cached.iter().any(|o| o.ts == ops[4].ts));

    // The offset index still knows every op the cache evicted.
    // `last_ts_per_actor` walks the index, not the cache.
    let last = storage.last_ts_per_actor().unwrap();
    assert_eq!(last.get(&actor).copied(), Some(ops[4].ts));
}

/// After LRU eviction, `ops_for_node` must still return every op
/// that touched the node — pulled back from disk via the per-node
/// secondary index. This is the correctness guarantee RFC #137
/// Phase A needs: shedding cold history can't lose data.
#[test]
fn ops_for_node_survives_lru_eviction() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // Use the legacy unbounded API so we can append without LRU
    // eviction interfering, then explicitly shrink afterwards.
    let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    let target = NodeId::new();
    // 5 edits on `target`, 1 edit on filler nodes between each so
    // they push `target` ops out of a small LRU.
    let mut target_ts: Vec<Hlc> = Vec::new();
    for _ in 0..5 {
        let ts = g.next();
        target_ts.push(ts);
        storage
            .append_op(&LogOp {
                ts,
                actor,
                op: Op::Edit {
                    node: target,
                    text_op: vec![1, 2, 3, 4],
                },
            })
            .unwrap();
        // Filler edit on a different node — 5 of these between
        // each target edit means a cap of 5 leaves all target ops
        // evicted.
        let _ = g.next();
        storage
            .append_op(&LogOp {
                ts: g.next(),
                actor,
                op: Op::Edit {
                    node: NodeId::new(),
                    text_op: vec![5, 6],
                },
            })
            .unwrap();
    }

    // Sanity: pre-shrink, ops_for_node returns every target op.
    let pre = storage.ops_for_node(target).unwrap();
    assert_eq!(pre.len(), 5);

    // Shrink the LRU so all target ops get evicted. cap=1 keeps
    // only the very last put (a filler), so every target op
    // becomes a cold read.
    storage.resize_cache(1);
    // Cache no longer holds any target op.
    let cached = storage.all_ops().unwrap();
    assert!(
        !cached.iter().any(|o| op_touches_node(&o.op, target)),
        "target ops should be evicted by resize_cache(1)"
    );

    // Cold path must still find every target op via the per-node
    // index + offset index.
    let post = storage.ops_for_node(target).unwrap();
    assert_eq!(
        post.len(),
        5,
        "ops_for_node must return every op even when the LRU has evicted them all"
    );
    // Same HLCs as we appended.
    let mut post_ts: Vec<Hlc> = post.iter().map(|o| o.ts).collect();
    post_ts.sort();
    let mut expected = target_ts.clone();
    expected.sort();
    assert_eq!(post_ts, expected);
}

/// Reload after a bounded-LRU session rehydrates from disk; the cap
/// still applies.
#[test]
fn reload_with_bounded_lru_keeps_cap() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    let ops: Vec<LogOp> = (0..4).map(|_| mk_create(&g)).collect();
    {
        let mut s = JsonlStorage::open_with_cap(dir.clone(), actor, 2).unwrap();
        for op in &ops {
            s.append_op(op).unwrap();
        }
    }
    let reopened = JsonlStorage::open_with_cap(dir, actor, 2).unwrap();
    assert_eq!(reopened.all_ops().unwrap().len(), 2);
}

/// `PageScope::PerPage` writes ops to `ops/<actor>/<slug>.jsonl`,
/// not the legacy `ops-<actor>.jsonl`. Boot reads them back from
/// the same path. RFC #137 Phase B.
#[test]
fn per_page_scope_routes_to_actor_subdir() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open_with_scope_cap(
        tmp.path().to_path_buf(),
        actor,
        PageScope::PerPage("project-x".into()),
        0,
    )
    .unwrap();
    let op = mk_create(&g);
    storage.append_op(&op).unwrap();

    // File landed under `ops/<actor>/project-x.jsonl`, not
    // `ops-<actor>.jsonl`.
    let expected = tmp.path().join(format!("{actor}")).join("project-x.jsonl");
    assert!(
        expected.exists(),
        "expected per-page file at {}",
        expected.display()
    );
    let legacy = tmp.path().join(format!("ops-{actor}.jsonl"));
    assert!(
        !legacy.exists(),
        "legacy global file should not exist under PerPage scope"
    );

    // Reload reads from the per-page path.
    let reopened = JsonlStorage::open_with_scope_cap(
        tmp.path().to_path_buf(),
        actor,
        PageScope::PerPage("project-x".into()),
        0,
    )
    .unwrap();
    let ops = reopened.all_ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].ts, op.ts);

    // Scope is reported back.
    assert_eq!(reopened.scope(), &PageScope::PerPage("project-x".into()));
}

/// PerPage storage with no ops on disk boots cleanly (fresh page).
#[test]
fn per_page_scope_with_missing_file_boots_clean() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let storage = JsonlStorage::open_with_scope_cap(
        tmp.path().to_path_buf(),
        actor,
        PageScope::PerPage("never-existed".into()),
        0,
    )
    .unwrap();
    assert_eq!(storage.all_ops().unwrap().len(), 0);
}

/// Global and PerPage storages coexist in the same `ops/` dir
/// without clobbering each other.
#[test]
fn global_and_per_page_coexist() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // Write one op under Global.
    let mut global = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    let global_op = mk_create(&g);
    global.append_op(&global_op).unwrap();
    drop(global);

    // Write one op under PerPage("home").
    let mut per_page = JsonlStorage::open_with_scope_cap(
        tmp.path().to_path_buf(),
        actor,
        PageScope::PerPage("home".into()),
        0,
    )
    .unwrap();
    let page_op = mk_create(&g);
    per_page.append_op(&page_op).unwrap();
    drop(per_page);

    // Both reload independently and see only their own ops.
    let g2 = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    let p2 = JsonlStorage::open_with_scope_cap(
        tmp.path().to_path_buf(),
        actor,
        PageScope::PerPage("home".into()),
        0,
    )
    .unwrap();
    assert_eq!(g2.all_ops().unwrap().len(), 1);
    assert_eq!(p2.all_ops().unwrap().len(), 1);
    assert_ne!(g2.all_ops().unwrap()[0].ts, p2.all_ops().unwrap()[0].ts);
}
