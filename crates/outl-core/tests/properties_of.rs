//! `Tree::properties_of` — enumerate every property currently set on
//! a node, in one pass.
//!
//! Used by projection layers (mobile + TUI outline DTOs) so each
//! `OutlineNode` carries its own properties without scanning the
//! workspace-wide property map per block.

mod common;

use common::{create_op, op_at, pos, Replica};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::property::PropValue;

fn set_prop(node: NodeId, key: &str, value: Option<PropValue>, old: Option<PropValue>) -> Op {
    Op::SetProp {
        node,
        key: key.into(),
        value,
        old_value: old,
    }
}

#[test]
fn lists_every_binding_on_the_target_node() {
    let actor = ActorId::new();
    let n = NodeId::new();
    let other = NodeId::new();

    let mut r = Replica::new(actor);
    r.apply(op_at(actor, 1, 0, create_op(n, NodeId::root(), pos("a"))));
    r.apply(op_at(
        actor,
        2,
        0,
        create_op(other, NodeId::root(), pos("b")),
    ));
    r.apply(op_at(
        actor,
        3,
        0,
        set_prop(n, "priority", Some(PropValue::Text("high".into())), None),
    ));
    r.apply(op_at(
        actor,
        4,
        0,
        set_prop(n, "status", Some(PropValue::Text("active".into())), None),
    ));
    // Another node's binding must not leak into n's view.
    r.apply(op_at(
        actor,
        5,
        0,
        set_prop(other, "priority", Some(PropValue::Text("low".into())), None),
    ));

    let mut got: Vec<(String, PropValue)> = r
        .tree
        .properties_of(n)
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        got,
        vec![
            ("priority".into(), PropValue::Text("high".into())),
            ("status".into(), PropValue::Text("active".into())),
        ]
    );
}

#[test]
fn cleared_bindings_drop_out() {
    let actor = ActorId::new();
    let n = NodeId::new();

    let mut r = Replica::new(actor);
    r.apply(op_at(actor, 1, 0, create_op(n, NodeId::root(), pos("a"))));
    r.apply(op_at(
        actor,
        2,
        0,
        set_prop(n, "priority", Some(PropValue::Text("high".into())), None),
    ));
    r.apply(op_at(
        actor,
        3,
        0,
        set_prop(n, "status", Some(PropValue::Text("active".into())), None),
    ));
    // Unset `priority` — only `status` should remain.
    r.apply(op_at(
        actor,
        4,
        0,
        set_prop(n, "priority", None, Some(PropValue::Text("high".into()))),
    ));

    let keys: Vec<String> = r
        .tree
        .properties_of(n)
        .map(|(k, _)| k.to_string())
        .collect();
    assert_eq!(keys, vec!["status".to_string()]);
}

#[test]
fn empty_node_returns_nothing() {
    let actor = ActorId::new();
    let n = NodeId::new();
    let mut r = Replica::new(actor);
    r.apply(op_at(actor, 1, 0, create_op(n, NodeId::root(), pos("a"))));

    assert_eq!(r.tree.properties_of(n).count(), 0);
}
