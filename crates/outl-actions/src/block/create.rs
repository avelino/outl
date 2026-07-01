//! Block creation: single blocks, sibling inserts, and whole subtrees.
//!
//! Everything here mints **new** nodes (`Op::Create`, optionally
//! followed by an `Op::Edit` for the initial text). Re-parenting and
//! text edits of existing nodes live in [`super::moves`] and
//! [`super::edit`].

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::Op;
use outl_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

use crate::error::ActionError;
use crate::tree::{next_sibling, position_after, position_before, position_for_new_last_child};

use super::edit::edit_text;
use super::{ensure_in_tree, wrap};

/// Recursive spec for building a block + its descendants in one
/// shot. The shape is what agents naturally produce ("write me a
/// page with these bullets and these sub-bullets") and what import
/// pipelines reduce trees to.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlockTreeSpec {
    /// Raw block text (TODO/DONE prefix is left to the caller, same
    /// rule as [`edit_text`]).
    pub text: String,
    /// Children specs, applied left-to-right as last children of
    /// the parent. Empty by default.
    #[serde(default)]
    pub children: Vec<BlockTreeSpec>,
}

/// Outcome of `append_tree` / `create_under_tree`. Mirrors the
/// shape of the input so callers can walk the original spec and the
/// freshly minted ids in lockstep.
#[derive(Debug, Clone, Serialize)]
pub struct BlockTreeOutcome {
    /// Id of the node created for the root of this subtree.
    pub id: NodeId,
    /// Children outcomes, in the same order as the input
    /// `children`. Empty when the spec had no children.
    pub children: Vec<BlockTreeOutcome>,
}

/// Append a brand-new block as the last child of `parent` and return
/// its id. `parent` defaults to [`NodeId::root`] when not supplied.
pub fn append_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: Option<NodeId>,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    let parent = parent.unwrap_or_else(NodeId::root);
    let position = position_for_new_last_child(workspace, parent);
    create_with_position(workspace, hlc, parent, position, text)
}

/// Append a whole subtree under `parent` in a single call.
///
/// `spec.text` becomes a new last child of `parent`; each entry in
/// `spec.children` is then attached recursively as the last child of
/// that new node. The returned `BlockTreeOutcome` mirrors the input
/// shape so the caller can pair every spec node with its freshly
/// minted [`NodeId`].
///
/// Failure mode: if any nested op fails, the previously-applied ops
/// stay in the op log (we intentionally do not roll them back — the
/// CRDT log is append-only and the partial subtree is observable
/// behavior). Callers that need all-or-nothing semantics should run
/// the spec through validation first.
pub fn append_tree(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    spec: &BlockTreeSpec,
) -> Result<BlockTreeOutcome, ActionError> {
    let id = append_block(workspace, hlc, Some(parent), Some(&spec.text))?;
    let children = spec
        .children
        .iter()
        .map(|child| append_tree(workspace, hlc, id, child))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BlockTreeOutcome { id, children })
}

/// Append every entry in `specs` as a contiguous block of new last
/// children under `parent`, preserving order. Convenience for
/// `outl_page_create`-with-content where the caller hands us the
/// page's top-level outline as a forest.
pub fn append_forest(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    specs: &[BlockTreeSpec],
) -> Result<Vec<BlockTreeOutcome>, ActionError> {
    specs
        .iter()
        .map(|spec| append_tree(workspace, hlc, parent, spec))
        .collect()
}

/// Insert a new sibling immediately after `after`, sharing the same
/// parent.
pub fn create_after(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    after: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    ensure_in_tree(workspace, after)?;
    let parent = workspace
        .tree()
        .parent(after)
        .ok_or_else(|| ActionError::NotInTree(after.to_string()))?;
    let position = position_after(workspace, after)
        .ok_or_else(|| ActionError::MissingPosition(after.to_string()))?;
    create_with_position(workspace, hlc, parent, position, text)
}

/// Insert a new sibling immediately before `before`, sharing the same
/// parent.
///
/// Mirror of [`create_after`] for the "open a block above this one"
/// gesture (vim `O`, the desktop's `Cmd/Ctrl+Shift+Enter` with the
/// caret at column 0). The new block lands between `before` and its
/// preceding sibling, so the fractional index is computed by
/// [`position_before`].
pub fn create_before(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    before: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    ensure_in_tree(workspace, before)?;
    let parent = workspace
        .tree()
        .parent(before)
        .ok_or_else(|| ActionError::NotInTree(before.to_string()))?;

    if let Some(position) = position_before(workspace, before) {
        return create_with_position(workspace, hlc, parent, position, text);
    }

    // `before` is the first child sitting at the fractional floor
    // (`Fractional::first()`) — there is no representable slot beneath
    // it. Mirror what `move_up` does: shift `before` up into the gap
    // toward its next sibling, then drop the new block into the freed
    // floor slot so it lands ahead of `before` while every sibling
    // keeps its relative order.
    let floor = workspace
        .tree()
        .position(before)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(before.to_string()))?;
    let next_pos =
        next_sibling(workspace, before).and_then(|n| workspace.tree().position(n).cloned());
    let shifted = Fractional::between(Some(&floor), next_pos.as_ref());
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node: before,
            new_parent: parent,
            position: shifted,
            old_parent: parent,
            old_position: floor.clone(),
        },
    ))?;
    create_with_position(workspace, hlc, parent, floor, text)
}

/// Append a new block as the last child of `parent`. Synonym for
/// [`append_block`] when the parent is explicit.
pub fn create_under(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    let position = position_for_new_last_child(workspace, parent);
    create_with_position(workspace, hlc, parent, position, text)
}

fn create_with_position(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    position: Fractional,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    create_with_explicit_id(workspace, hlc, NodeId::new(), parent, position, text)
}

/// Create variant that uses a caller-supplied `node` id instead of a
/// fresh ULID.
///
/// Used by [`crate::page::open_or_create`] so that two peers
/// independently materialising the same slug end up with the same
/// `NodeId`. With a random id, each device would create a separate
/// page node and the CRDT would have no way to merge them after the
/// fact (different ids = different subtrees).
///
/// Re-creating an already-existing node is a no-op at the CRDT layer
/// (the second `Op::Create` is dropped because the node is already in
/// the tree), which makes this safe to call even when the page
/// already exists locally.
pub(crate) fn create_with_explicit_id(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    parent: NodeId,
    position: Fractional,
    text: Option<&str>,
) -> Result<NodeId, ActionError> {
    workspace.apply(wrap(
        hlc,
        Op::Create {
            node,
            parent,
            position,
        },
    ))?;

    if let Some(body) = text {
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            edit_text(workspace, hlc, node, trimmed)?;
        }
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;

    fn new_workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    /// Regression: when the anchor has children, `create_after` must
    /// return the id of the brand-new sibling — not a descendant.
    ///
    /// Why: clients used to skip this return value and recover the new
    /// id by walking the refreshed outline (`flat[idx + 1]` after the
    /// anchor). That walk lands on `anchor.children[0]` instead of the
    /// new sibling whenever the anchor has expanded children, and the
    /// next `edit_text` would target a stale id and surface
    /// `block <ULID> is not in the tree` toasts on blur. The fix is
    /// to make every Tauri `create_block` command propagate
    /// `create_after`'s `NodeId` to the frontend; this test pins the
    /// contract on the `outl-actions` side so the regression cannot
    /// silently reappear.
    #[test]
    fn create_after_returns_new_sibling_not_a_child_of_anchor() {
        let (mut ws, hlc) = new_workspace();
        let anchor = append_block(&mut ws, &hlc, None, Some("anchor")).unwrap();
        let child = append_block(&mut ws, &hlc, Some(anchor), Some("child")).unwrap();

        let new_id = create_after(&mut ws, &hlc, anchor, Some("sibling")).unwrap();

        assert_ne!(new_id, child, "must not return the existing child");
        assert_eq!(
            ws.tree().parent(new_id),
            ws.tree().parent(anchor),
            "new block must be a sibling of the anchor (same parent)"
        );
        assert_eq!(ws.block_text(new_id).as_deref(), Some("sibling"));
    }

    #[test]
    fn append_tree_creates_root_and_children_in_order() {
        let (mut ws, hlc) = new_workspace();
        let spec = BlockTreeSpec {
            text: "root".into(),
            children: vec![
                BlockTreeSpec {
                    text: "a".into(),
                    children: vec![BlockTreeSpec {
                        text: "a1".into(),
                        children: vec![],
                    }],
                },
                BlockTreeSpec {
                    text: "b".into(),
                    children: vec![],
                },
            ],
        };
        let outcome = append_tree(&mut ws, &hlc, NodeId::root(), &spec).unwrap();

        assert_eq!(ws.block_text(outcome.id).as_deref(), Some("root"));
        assert_eq!(outcome.children.len(), 2);

        let a = &outcome.children[0];
        let b = &outcome.children[1];
        assert_eq!(ws.block_text(a.id).as_deref(), Some("a"));
        assert_eq!(ws.block_text(b.id).as_deref(), Some("b"));
        assert_eq!(ws.tree().parent(a.id), Some(outcome.id));
        assert_eq!(ws.tree().parent(b.id), Some(outcome.id));

        // Children of `root` come back in insertion order.
        let kids: Vec<NodeId> = crate::tree::children_of(&ws, outcome.id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(kids, vec![a.id, b.id]);

        // a's nested child landed under a.
        assert_eq!(a.children.len(), 1);
        let a1 = &a.children[0];
        assert_eq!(ws.block_text(a1.id).as_deref(), Some("a1"));
        assert_eq!(ws.tree().parent(a1.id), Some(a.id));
    }

    #[test]
    fn append_forest_preserves_order_and_targets_parent() {
        let (mut ws, hlc) = new_workspace();
        let parent = append_block(&mut ws, &hlc, None, Some("parent")).unwrap();
        let specs = vec![
            BlockTreeSpec {
                text: "one".into(),
                children: vec![],
            },
            BlockTreeSpec {
                text: "two".into(),
                children: vec![],
            },
            BlockTreeSpec {
                text: "three".into(),
                children: vec![],
            },
        ];
        let outcomes = append_forest(&mut ws, &hlc, parent, &specs).unwrap();
        let ids: Vec<NodeId> = outcomes.iter().map(|o| o.id).collect();
        let kids: Vec<NodeId> = crate::tree::children_of(&ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(kids, ids);
    }

    #[test]
    fn append_tree_empty_text_creates_node_without_edit() {
        let (mut ws, hlc) = new_workspace();
        let spec = BlockTreeSpec {
            text: "".into(),
            children: vec![],
        };
        let outcome = append_tree(&mut ws, &hlc, NodeId::root(), &spec).unwrap();
        // Empty text path skips the Edit op; block_text returns empty
        // string (Yrs default).
        assert_eq!(
            ws.block_text(outcome.id).as_deref().unwrap_or(""),
            "",
            "empty-text spec must still create the node"
        );
    }

    #[test]
    fn create_before_inserts_sibling_directly_ahead_of_anchor() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();

        let new_id = create_before(&mut ws, &hlc, b, Some("between")).unwrap();

        assert_eq!(
            ws.tree().parent(new_id),
            ws.tree().parent(b),
            "new block must be a sibling of the anchor (same parent)"
        );
        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![a, new_id, b], "new block lands between a and b");
        assert_eq!(ws.block_text(new_id).as_deref(), Some("between"));
    }

    #[test]
    fn create_before_first_child_lands_at_the_front() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();

        let new_id = create_before(&mut ws, &hlc, a, Some("head")).unwrap();

        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            order,
            vec![new_id, a],
            "new block becomes the first sibling"
        );
    }

    #[test]
    fn create_before_first_of_many_reorders_via_floor_shift() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        let c = append_block(&mut ws, &hlc, None, Some("c")).unwrap();

        // `a` sits at the fractional floor; inserting before it must
        // shift `a` up and keep b / c in order.
        let new_id = create_before(&mut ws, &hlc, a, Some("head")).unwrap();

        let order: Vec<_> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![new_id, a, b, c]);
    }
}
