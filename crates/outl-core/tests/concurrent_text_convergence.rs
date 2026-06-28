//! Cross-device convergence of block *text*, exercised at the `Workspace`
//! layer (not the bare `Tree`).
//!
//! The rest of the battery drives the structural CRDT through `Replica`
//! (`Tree` + `OpLog`) and never touches the `ContentStore` that
//! materializes block text. This test closes that gap: two workspaces
//! edit the same block concurrently, cross-deliver the `Op::Edit`s, and
//! must converge to the same string — the property the issue #108
//! two-tier `ContentStore` rewrite must not break.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::Workspace;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// Two actors edit the same block without seeing each other's edit, then
/// exchange ops. Both replicas must end on the same text, and redelivery
/// (sync re-sending an op) must not change it.
#[test]
fn concurrent_edits_converge_and_are_idempotent() {
    let a1 = ActorId::new();
    let a2 = ActorId::new();
    let g1 = HlcGenerator::new(a1);
    let g2 = HlcGenerator::new(a2);

    let mut ws1 = Workspace::open_in_memory(a1).unwrap();
    let mut ws2 = Workspace::open_in_memory(a2).unwrap();

    // Shared block under root: actor 1 creates it, both apply the Create.
    let b = NodeId::new();
    let create = logop(
        &g1,
        Op::Create {
            node: b,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    );
    ws1.apply(create.clone()).unwrap();
    ws2.apply(create).unwrap();

    // Concurrent edits: neither workspace has seen the other's yet.
    let u1 = ws1.build_text_replace_update(b, "hello");
    let edit1 = logop(
        &g1,
        Op::Edit {
            node: b,
            text_op: u1,
        },
    );
    ws1.apply(edit1.clone()).unwrap();

    let u2 = ws2.build_text_replace_update(b, "world");
    let edit2 = logop(
        &g2,
        Op::Edit {
            node: b,
            text_op: u2,
        },
    );
    ws2.apply(edit2.clone()).unwrap();

    assert_eq!(ws1.block_text(b).as_deref(), Some("hello"));
    assert_eq!(ws2.block_text(b).as_deref(), Some("world"));

    // Cross-deliver.
    ws1.apply(edit2.clone()).unwrap();
    ws2.apply(edit1.clone()).unwrap();

    let s1 = ws1.block_text(b);
    let s2 = ws2.block_text(b);
    assert_eq!(s1, s2, "replicas diverged on block text: {s1:?} vs {s2:?}");
    let converged = s1.expect("block has text");

    // Redelivery of every op is a no-op on the converged text.
    for op in [edit1, edit2] {
        ws1.apply(op.clone()).unwrap();
        ws2.apply(op).unwrap();
    }
    assert_eq!(ws1.block_text(b).as_deref(), Some(converged.as_str()));
    assert_eq!(ws2.block_text(b).as_deref(), Some(converged.as_str()));
}

/// A workspace reloaded from storage rebuilds the same converged text from
/// the op log alone, with no resident `Doc` carried across the reopen.
#[test]
fn converged_text_survives_reopen() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let b = NodeId::new();
    let converged = {
        let storage = Box::new(outl_core::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
        let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();
        ws.apply(logop(
            &g,
            Op::Create {
                node: b,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        for text in ["draft", "draft v2", "final"] {
            let u = ws.build_text_replace_update(b, text);
            ws.apply(logop(
                &g,
                Op::Edit {
                    node: b,
                    text_op: u,
                },
            ))
            .unwrap();
        }
        ws.block_text(b).expect("block has text")
    };

    let storage = Box::new(outl_core::storage::JsonlStorage::open(dir, actor).unwrap());
    let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();
    assert_eq!(ws2.block_text(b).as_deref(), Some(converged.as_str()));
}
