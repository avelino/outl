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

/// Bounded LRU keeps RSS constant — ops past the cap are evicted from
/// RAM — while the offset index stays complete, so `all_ops` still
/// returns every op (rehydrated from disk) and `last_ts_per_actor`
/// still sees the newest. RAM is bounded; the logical op set is not.
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

    // RAM is bounded: only the last 3 of the 5 ops stay resident in the
    // LRU (`file_stats` counts cache entries).
    let resident: usize = storage.file_stats().iter().map(|(_, n)| n).sum();
    assert_eq!(resident, 3, "LRU should hold only the last 3 ops in RAM");

    // But the logical op set is complete: `all_ops` rehydrates the two
    // evicted ops from the offset index — no silent loss.
    let all = storage.all_ops().unwrap();
    assert_eq!(all.len(), 5, "all_ops returns every op via the index");
    for op in &ops {
        assert!(
            all.iter().any(|o| o.ts == op.ts),
            "every appended op must come back, evicted or not"
        );
    }

    // The offset index still knows every op the cache evicted.
    // `last_ts_per_actor` reads the index, not the cache.
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
    // The LRU now holds a single resident op (a filler), so every
    // target op is guaranteed cold. `all_ops` would still return them
    // (it reads the complete index), so check RAM residency directly.
    let resident: usize = storage.file_stats().iter().map(|(_, n)| n).sum();
    assert_eq!(resident, 1, "resize_cache(1) leaves one op resident");

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

/// Reopening rebuilds the offset index from disk and leaves the LRU
/// empty (boot no longer reparses the full log into the cache). The
/// cap bounds only resident RAM, not the logical op set, so `all_ops`
/// recovers every op via the index regardless of the cap.
#[test]
fn reload_with_bounded_lru_rehydrates_full_log() {
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
    // Boot leaves the cache empty; the index still holds all 4 offsets.
    let reopened = JsonlStorage::open_with_cap(dir, actor, 2).unwrap();
    let resident: usize = reopened.file_stats().iter().map(|(_, n)| n).sum();
    assert_eq!(resident, 0, "boot leaves the LRU empty");
    assert_eq!(
        reopened.all_ops().unwrap().len(),
        4,
        "all_ops rehydrates the full log from the offset index"
    );
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

/// A second boot loads the persisted `.idx`/`.nodes.idx` and, when they
/// exactly cover the `.jsonl`, uses them AS-IS — no reparse of the log
/// body. Sentinel: only the fresh branch skips `save()`, so byte-identical
/// sidecars across a reboot prove the body was never re-streamed for
/// indexing (a rebuild would rewrite them).
#[test]
fn reload_uses_fresh_idx_without_reparse() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    // Boot + append 5 ops. Each append mirrors into both sidecars.
    {
        let mut s = JsonlStorage::open(dir.clone(), actor).unwrap();
        for _ in 0..5 {
            s.append_op(&mk_create(&g)).unwrap();
        }
    }
    let idx_path = dir.join(format!(".ops-{actor}.idx"));
    let node_idx_path = dir.join(format!(".ops-{actor}.nodes.idx"));
    let jsonl_path = dir.join(format!("ops-{actor}.jsonl"));
    let idx_before = std::fs::read(&idx_path).unwrap();
    let node_before = std::fs::read(&node_idx_path).unwrap();
    // The sidecar covers all 5 ops.
    assert_eq!(
        String::from_utf8(idx_before.clone())
            .unwrap()
            .lines()
            .count(),
        5
    );

    // Corrupt the FIRST line of the `.jsonl` in place, same byte length so
    // every later offset (and the last record) stays put. A full parse-lite
    // REBUILD would skip this line and rewrite the sidecar to 4 entries; the
    // fresh path only reads the LAST record, so it must NOT reparse or rewrite.
    let body = std::fs::read(&jsonl_path).unwrap();
    let first_nl = body.iter().position(|&b| b == b'\n').unwrap();
    let mut corrupted = body.clone();
    for b in corrupted[..first_nl].iter_mut() {
        *b = b'x'; // not JSON → parse-lite Skip, same length, offsets intact
    }
    std::fs::write(&jsonl_path, &corrupted).unwrap();

    // Reboot: the last record still parses, coverage is byte-exact → fresh.
    let reopened = JsonlStorage::open(dir.clone(), actor).unwrap();

    // Sentinel: the sidecars are byte-identical (5 entries, unchanged). Only
    // the fresh branch skips `save()`; a rebuild would have rewritten them to
    // 4 entries after skipping the corrupt line.
    assert_eq!(std::fs::read(&idx_path).unwrap(), idx_before);
    assert_eq!(std::fs::read(&node_idx_path).unwrap(), node_before);

    // Contrast proves the source: the INDEX (from the trusted sidecar) still
    // knows all 5 ops, while `all_ops` — which re-reads the corrupt body
    // sequentially — sees only 4. If the fresh path had reparsed the body it
    // would have dropped the corrupt line from the index too.
    assert!(reopened.last_ts_per_actor().unwrap().contains_key(&actor));
    assert_eq!(reopened.all_ops().unwrap().len(), 4);
}

/// A `.jsonl` that grew since the sidecars were written (append after the
/// last indexed op, sidecars left stale) is handled by indexing only the
/// TAIL and merging into the loaded indexes — old + new ops all end up
/// indexed, identical to a full rebuild, and the node index gets the tail
/// too (cold `ops_for_node` finds an appended op).
#[test]
fn reload_reindexes_appended_tail() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    // Boot + append 3 ops via storage (sidecars now cover exactly 3).
    {
        let mut s = JsonlStorage::open(dir.clone(), actor).unwrap();
        for _ in 0..3 {
            s.append_op(&mk_create(&g)).unwrap();
        }
    }

    // Append 2 more op LINES directly to the `.jsonl`, leaving the sidecars
    // stale at 3 (simulates a crash after the `.jsonl` fsync but before the
    // sidecar append). One carries a known node so we can probe the node
    // index afterwards.
    let jsonl_path = dir.join(format!("ops-{actor}.jsonl"));
    let tail_node = NodeId::new();
    let tail_ts = g.next();
    let tail_op = LogOp {
        ts: tail_ts,
        actor,
        op: Op::Create {
            node: tail_node,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };
    let extra = [mk_create(&g), tail_op];
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&jsonl_path)
            .unwrap();
        for op in &extra {
            writeln!(f, "{}", serde_json::to_string(op).unwrap()).unwrap();
        }
    }

    // Reboot: covered_end < file_size → grew → tail reindex.
    let reopened = JsonlStorage::open(dir.clone(), actor).unwrap();

    let mut all: Vec<Hlc> = reopened.all_ops().unwrap().iter().map(|o| o.ts).collect();
    all.sort();
    assert_eq!(all.len(), 5, "grew reindex must cover old + appended ops");

    // Node index got the tail: cold read via the per-node index returns the
    // appended op.
    let for_node = reopened.ops_for_node(tail_node).unwrap();
    assert_eq!(for_node.len(), 1);
    assert_eq!(for_node[0].ts, tail_ts);

    // The grew branch persisted the extended sidecar (now 5 entries).
    let idx_path = dir.join(format!(".ops-{actor}.idx"));
    let node_idx_path = dir.join(format!(".ops-{actor}.nodes.idx"));
    assert_eq!(
        std::fs::read_to_string(&idx_path).unwrap().lines().count(),
        5,
        "grew branch must persist the tail into the sidecar"
    );

    // Identical to a full rebuild: delete sidecars, reboot, compare.
    std::fs::remove_file(&idx_path).unwrap();
    std::fs::remove_file(&node_idx_path).unwrap();
    let control = JsonlStorage::open(dir.clone(), actor).unwrap();
    let mut control_ts: Vec<Hlc> = control.all_ops().unwrap().iter().map(|o| o.ts).collect();
    control_ts.sort();
    assert_eq!(
        all, control_ts,
        "grew tail-reindex must equal a full rebuild"
    );
}

/// A missing or corrupt `.idx` falls through to a full parse-lite rebuild —
/// every op is re-indexed and the sidecar is regenerated. The index is a
/// cache; losing it can never lose ops, only cost a reparse.
#[test]
fn reload_rebuilds_on_missing_or_corrupt_idx() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    let ops: Vec<LogOp> = (0..4).map(|_| mk_create(&g)).collect();
    {
        let mut s = JsonlStorage::open(dir.clone(), actor).unwrap();
        for op in &ops {
            s.append_op(op).unwrap();
        }
    }
    let idx_path = dir.join(format!(".ops-{actor}.idx"));

    let expected: Vec<Hlc> = {
        let mut v: Vec<Hlc> = ops.iter().map(|o| o.ts).collect();
        v.sort();
        v
    };

    // (a) Missing `.idx` → rebuild.
    std::fs::remove_file(&idx_path).unwrap();
    {
        let r = JsonlStorage::open(dir.clone(), actor).unwrap();
        let mut got: Vec<Hlc> = r.all_ops().unwrap().iter().map(|o| o.ts).collect();
        got.sort();
        assert_eq!(got, expected, "missing .idx must rebuild the full index");
        assert!(idx_path.exists(), "rebuild regenerates the .idx sidecar");
    }

    // (b) Corrupt `.idx` (non-JSON garbage) → rebuild.
    std::fs::write(&idx_path, b"garbage not json at all\n").unwrap();
    {
        let r = JsonlStorage::open(dir.clone(), actor).unwrap();
        let mut got: Vec<Hlc> = r.all_ops().unwrap().iter().map(|o| o.ts).collect();
        got.sort();
        assert_eq!(got, expected, "corrupt .idx must rebuild the full index");
    }
}

/// An `.idx` pointing past the end of a truncated `.jsonl` (shrank) is
/// suspect and triggers a full rebuild over whatever survived — no panic,
/// no trust in the stale index.
#[test]
fn reload_rebuilds_on_shrunk_jsonl() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    let ops: Vec<LogOp> = (0..4).map(|_| mk_create(&g)).collect();
    {
        let mut s = JsonlStorage::open(dir.clone(), actor).unwrap();
        for op in &ops {
            s.append_op(op).unwrap();
        }
    }

    // Truncate the `.jsonl` at the boundary after the 2nd line — 2 complete
    // ops survive, but the sidecar still points at offsets for all 4.
    let jsonl_path = dir.join(format!("ops-{actor}.jsonl"));
    let body = std::fs::read(&jsonl_path).unwrap();
    let mut newlines = 0usize;
    let mut cut = body.len();
    for (i, &b) in body.iter().enumerate() {
        if b == b'\n' {
            newlines += 1;
            if newlines == 2 {
                cut = i + 1;
                break;
            }
        }
    }
    {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&jsonl_path)
            .unwrap();
        f.set_len(cut as u64).unwrap();
    }

    // Reboot must not panic and must index exactly the two survivors.
    let reopened = JsonlStorage::open(dir.clone(), actor).unwrap();
    let mut got: Vec<Hlc> = reopened.all_ops().unwrap().iter().map(|o| o.ts).collect();
    got.sort();
    let mut expected: Vec<Hlc> = ops[..2].iter().map(|o| o.ts).collect();
    expected.sort();
    assert_eq!(got, expected, "shrunk .jsonl rebuilds to the surviving ops");
}

/// A crash landing between the `.idx` append and the `.nodes.idx` append
/// leaves the two sidecars at different max offsets. The node/offset
/// symmetry guard must catch the lag and rebuild rather than trust a
/// half-updated pair — otherwise the un-indexed tail op is silently dropped
/// from the node-driven `ops_for_node` read (block-text rebuild, #129).
#[test]
fn reload_rebuilds_on_node_offset_asymmetry() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let dir = tmp.path().to_path_buf();
    let g = HlcGenerator::new(actor);

    // Four edits on the same node → four `.nodes.idx` entries, one per op,
    // appended in offset order.
    let target = NodeId::new();
    let mut target_ts: Vec<Hlc> = Vec::new();
    {
        let mut s = JsonlStorage::open(dir.clone(), actor).unwrap();
        for _ in 0..4 {
            let ts = g.next();
            target_ts.push(ts);
            s.append_op(&LogOp {
                ts,
                actor,
                op: Op::Edit {
                    node: target,
                    text_op: vec![1, 2, 3],
                },
            })
            .unwrap();
        }
    }
    target_ts.sort();

    // Simulate the crash: the offset index carries all four entries, but the
    // node index is missing its last (highest-offset) one — as if the process
    // died after appending `.idx` and before finishing `.nodes.idx`. Dropping
    // the tail line makes `NodeIndex::max_offset()` lag `OffsetIndex`'s.
    let node_idx_path = dir.join(format!(".ops-{actor}.nodes.idx"));
    let content = std::fs::read_to_string(&node_idx_path).unwrap();
    let mut lines: Vec<&str> = content.lines().collect();
    lines.pop();
    std::fs::write(&node_idx_path, format!("{}\n", lines.join("\n"))).unwrap();

    // Reboot: the asymmetry must trigger a full rebuild, and every op that
    // touched `target` — including the one whose node entry was dropped — must
    // come back through the node-driven read.
    let r = JsonlStorage::open(dir.clone(), actor).unwrap();
    let mut got: Vec<Hlc> = r
        .ops_for_node(target)
        .unwrap()
        .iter()
        .map(|o| o.ts)
        .collect();
    got.sort();
    assert_eq!(
        got, target_ts,
        "asymmetric sidecars must rebuild so ops_for_node loses nothing"
    );
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
