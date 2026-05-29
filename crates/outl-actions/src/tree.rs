//! Read-only navigation helpers over the materialised tree.

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

/// Children of `parent` sorted ascending by their fractional position.
pub fn children_of(workspace: &Workspace, parent: NodeId) -> Vec<(NodeId, Fractional)> {
    let mut rows: Vec<(NodeId, Fractional)> = workspace
        .tree()
        .iter_nodes()
        .filter(|(_, p, _)| *p == parent)
        .map(|(id, _, pos)| (id, pos.clone()))
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows
}

/// Previous sibling of `node` in its parent's children order.
pub(crate) fn previous_sibling(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let parent = workspace.tree().parent(node)?;
    let mut prev: Option<NodeId> = None;
    for (id, _) in children_of(workspace, parent) {
        if id == node {
            return prev;
        }
        prev = Some(id);
    }
    None
}

/// Next sibling of `node` in its parent's children order.
pub fn next_sibling(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let parent = workspace.tree().parent(node)?;
    let mut iter = children_of(workspace, parent).into_iter();
    while let Some((id, _)) = iter.next() {
        if id == node {
            return iter.next().map(|(id, _)| id);
        }
    }
    None
}

/// A fractional position strictly between `node` and the sibling that
/// follows it. Returns `None` when `node` is not in the tree.
pub(crate) fn position_after(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let parent = workspace.tree().parent(node)?;
    let siblings = children_of(workspace, parent);
    let mut iter = siblings.into_iter().peekable();
    while let Some((id, _)) = iter.next() {
        if id == node {
            let left = workspace.tree().position(node)?.clone();
            let right = iter.peek().map(|(_, p)| p.clone());
            return Some(Fractional::between(Some(&left), right.as_ref()));
        }
    }
    None
}

/// Fractional position for a new last child appended under `parent`.
pub(crate) fn position_for_new_last_child(workspace: &Workspace, parent: NodeId) -> Fractional {
    let last = children_of(workspace, parent)
        .into_iter()
        .last()
        .map(|(_, p)| p);
    match last {
        Some(p) => Fractional::between(Some(&p), None),
        None => Fractional::first(),
    }
}
