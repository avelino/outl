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
///
/// Promoted to `pub` for the CLI's `outl block move --after=…` flow;
/// other clients can use it whenever they need a position immediately
/// after a known node.
pub fn position_after(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
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

/// A fractional position strictly between `node` and the sibling that
/// precedes it, for the "insert before this node" flow (vim `O`,
/// `Cmd+Shift+Tab` at column 0 on the desktop).
///
/// Returns `None` when no such slot is representable — either `node`
/// is not in the tree, or it is the first child sitting at the
/// fractional floor (`Fractional::first()`), below which the `[a-z]`
/// index has no room. Callers handle the floor case by repositioning
/// (see [`crate::block::create_before`]), exactly how `move_up` swaps
/// rather than minting a sub-floor key.
pub fn position_before(workspace: &Workspace, node: NodeId) -> Option<Fractional> {
    let node_pos = workspace.tree().position(node)?.clone();
    match previous_sibling(workspace, node) {
        Some(prev) => {
            let prev_pos = workspace.tree().position(prev)?.clone();
            Some(Fractional::between(Some(&prev_pos), Some(&node_pos)))
        }
        None => {
            // No predecessor: we need a slot below `node`. `between`
            // floors at `Fractional::first()`, so the result is only
            // valid when it actually sorts before `node`.
            let candidate = Fractional::between(None, Some(&node_pos));
            (candidate < node_pos).then_some(candidate)
        }
    }
}

/// Fractional position for a new last child appended under `parent`.
///
/// Promoted to `pub` for the CLI's `outl block move --parent=…` flow.
pub fn position_for_new_last_child(workspace: &Workspace, parent: NodeId) -> Fractional {
    let last = children_of(workspace, parent)
        .into_iter()
        .last()
        .map(|(_, p)| p);
    match last {
        Some(p) => Fractional::between(Some(&p), None),
        None => Fractional::first(),
    }
}

/// Walk `parent`'s subtree in DFS pre-order and invoke `f` on every
/// descendant `NodeId`. Stops early when `f` returns `false`.
///
/// Centralises the "walk all descendants" pattern that the CLI's
/// `search`/`tag`/`query` paths used to reimplement individually.
/// Callers that need access to text/properties call into
/// [`Workspace::block_text`] / [`outl_core::Workspace::tree`] inside
/// the closure.
pub fn walk_subtree<F>(workspace: &Workspace, parent: NodeId, mut f: F)
where
    F: FnMut(NodeId) -> bool,
{
    walk_inner(workspace, parent, &mut f);
}

fn walk_inner<F>(workspace: &Workspace, parent: NodeId, f: &mut F) -> bool
where
    F: FnMut(NodeId) -> bool,
{
    for (id, _) in children_of(workspace, parent) {
        if !f(id) {
            return false;
        }
        if !walk_inner(workspace, id, f) {
            return false;
        }
    }
    true
}

/// Walk up from `node` until we find the page node hosting it — that's
/// the highest ancestor sitting directly under [`NodeId::root`].
///
/// Returns `None` if `node` itself is the root (or detached). Lives in
/// `tree` because every client (CLI, future TUI handler, mobile) needs
/// it to re-render the page after a mutation on a deep block.
pub fn enclosing_page_id(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let mut current = node;
    loop {
        let parent = workspace.tree().parent(current)?;
        if parent == NodeId::root() {
            return Some(current);
        }
        current = parent;
    }
}
