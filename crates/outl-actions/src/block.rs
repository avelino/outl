//! Workspace mutations expressed as high-level user actions.
//!
//! Every function in this module:
//!
//! 1. Reads the current tree to figure out the right `Op` parameters
//!    (parent id, position, undo fields, ...).
//! 2. Generates a fresh [`LogOp`] via the caller-supplied
//!    [`HlcGenerator`].
//! 3. Routes it through [`Workspace::apply`] so the op log stays the
//!    single source of truth.
//!
//! The functions never reach for storage directly and never touch
//! filesystem state — the caller decides whether (and when) to
//! re-render the markdown projection by calling
//! [`crate::journal::apply_page_md`] or
//! [`crate::journal::apply_all_pages_md`].

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::todo::cycle_todo;
use crate::tree::{next_sibling, position_after, position_for_new_last_child, previous_sibling};

/// Build a [`LogOp`] wrapping `op` with a fresh HLC.
fn wrap(hlc: &HlcGenerator, op: Op) -> LogOp {
    let ts = hlc.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

fn ensure_in_tree(workspace: &Workspace, node: NodeId) -> Result<(), ActionError> {
    if workspace.tree().contains(node) {
        Ok(())
    } else {
        Err(ActionError::NotInTree(node.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Edit
// ---------------------------------------------------------------------------

/// Replace the block's text with `new_text`. If the block has a
/// `TODO`/`DONE` prefix, it is preserved automatically — the caller
/// only sends the body.
pub fn edit_text(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    new_text: &str,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;

    let current = workspace.block_text(node).unwrap_or_default();
    let prefix = if current.starts_with("TODO ") {
        Some("TODO ")
    } else if current.starts_with("DONE ") {
        Some("DONE ")
    } else {
        None
    };
    let final_text = match prefix {
        Some(p) => format!("{p}{new_text}"),
        None => new_text.to_string(),
    };

    let update = workspace.build_text_replace_update(node, &final_text);
    if update.is_empty() {
        return Ok(());
    }
    workspace.apply(wrap(
        hlc,
        Op::Edit {
            node,
            text_op: update,
        },
    ))?;
    Ok(())
}

/// Cycle the block's TODO state: `None → TODO → DONE → None`.
pub fn toggle_todo(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    ensure_in_tree(workspace, node)?;
    let current = workspace.block_text(node).unwrap_or_default();
    let next = cycle_todo(&current);
    let update = workspace.build_text_replace_update(node, &next);
    if update.is_empty() {
        return Ok(());
    }
    workspace.apply(wrap(
        hlc,
        Op::Edit {
            node,
            text_op: update,
        },
    ))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Move (delete is just move-to-trash)
// ---------------------------------------------------------------------------

fn move_to(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    new_parent: NodeId,
    position: Fractional,
) -> Result<(), ActionError> {
    let old_parent = workspace
        .tree()
        .parent(node)
        .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
    let old_position = workspace
        .tree()
        .position(node)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node,
            new_parent,
            position,
            old_parent,
            old_position,
        },
    ))?;
    Ok(())
}

/// Move the block under the trash root. Materialised tree drops it
/// immediately; the op stays in the log forever.
pub fn delete(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    move_to(workspace, hlc, node, NodeId::trash(), Fractional::first())
}

/// Indent `node` so it becomes the last child of its previous sibling.
pub fn indent(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let prev = previous_sibling(workspace, node)
        .ok_or_else(|| ActionError::NoPreviousSibling(node.to_string()))?;
    let pos = position_for_new_last_child(workspace, prev);
    move_to(workspace, hlc, node, prev, pos)
}

/// Outdent `node` so it becomes a sibling of its current parent.
pub fn outdent(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let parent = workspace
        .tree()
        .parent(node)
        .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
    if parent == NodeId::root() {
        return Err(ActionError::AlreadyAtRoot(node.to_string()));
    }
    let grand = workspace
        .tree()
        .parent(parent)
        .ok_or_else(|| ActionError::NoGrandparent(node.to_string()))?;
    let pos = position_after(workspace, parent)
        .ok_or_else(|| ActionError::MissingPosition(parent.to_string()))?;
    move_to(workspace, hlc, node, grand, pos)
}

/// Swap `node` with its previous sibling. No-op if `node` is already
/// the first sibling.
pub fn move_up(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let prev = match previous_sibling(workspace, node) {
        Some(p) => p,
        None => return Ok(()),
    };
    swap_positions(workspace, hlc, node, prev)
}

/// Swap `node` with its next sibling. No-op if `node` is already the
/// last sibling.
pub fn move_down(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<(), ActionError> {
    let next = match next_sibling(workspace, node) {
        Some(n) => n,
        None => return Ok(()),
    };
    swap_positions(workspace, hlc, node, next)
}

/// Swap the positions of two nodes by emitting two `Move` ops. They
/// keep the same parent (this is a sibling-swap helper); use
/// [`indent`] / [`outdent`] for re-parenting.
fn swap_positions(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    a: NodeId,
    b: NodeId,
) -> Result<(), ActionError> {
    let parent_a = workspace
        .tree()
        .parent(a)
        .ok_or_else(|| ActionError::NotInTree(a.to_string()))?;
    let parent_b = workspace
        .tree()
        .parent(b)
        .ok_or_else(|| ActionError::NotInTree(b.to_string()))?;
    let pos_a = workspace
        .tree()
        .position(a)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(a.to_string()))?;
    let pos_b = workspace
        .tree()
        .position(b)
        .cloned()
        .ok_or_else(|| ActionError::MissingPosition(b.to_string()))?;

    // Move a → b's slot.
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node: a,
            new_parent: parent_b,
            position: pos_b.clone(),
            old_parent: parent_a,
            old_position: pos_a.clone(),
        },
    ))?;
    // Move b → a's old slot.
    workspace.apply(wrap(
        hlc,
        Op::Move {
            node: b,
            new_parent: parent_a,
            position: pos_a,
            old_parent: parent_b,
            old_position: pos_b,
        },
    ))?;
    Ok(())
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

    #[test]
    fn append_then_edit_changes_text() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("hello")).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("hello"));

        edit_text(&mut ws, &hlc, n, "hello world").unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("hello world"));
    }

    #[test]
    fn toggle_cycles_through_states() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("DONE ship it"));
        toggle_todo(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("ship it"));
    }

    #[test]
    fn edit_preserves_todo_prefix() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("ship it")).unwrap();
        toggle_todo(&mut ws, &hlc, n).unwrap();
        edit_text(&mut ws, &hlc, n, "ship the feature").unwrap();
        assert_eq!(ws.block_text(n).as_deref(), Some("TODO ship the feature"));
    }

    #[test]
    fn indent_makes_block_child_of_previous_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        indent(&mut ws, &hlc, b).unwrap();
        assert_eq!(ws.tree().parent(b), Some(a));
    }

    #[test]
    fn outdent_promotes_to_grandparent_level() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        indent(&mut ws, &hlc, b).unwrap();
        outdent(&mut ws, &hlc, b).unwrap();
        assert_eq!(ws.tree().parent(b), Some(NodeId::root()));
        // a stays where it is
        assert_eq!(ws.tree().parent(a), Some(NodeId::root()));
    }

    #[test]
    fn delete_moves_to_trash() {
        let (mut ws, hlc) = new_workspace();
        let n = append_block(&mut ws, &hlc, None, Some("trash me")).unwrap();
        delete(&mut ws, &hlc, n).unwrap();
        assert_eq!(ws.tree().parent(n), Some(NodeId::trash()));
    }

    #[test]
    fn indent_rejects_first_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        assert!(matches!(
            indent(&mut ws, &hlc, a),
            Err(ActionError::NoPreviousSibling(_))
        ));
    }

    #[test]
    fn outdent_rejects_root_level_block() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        assert!(matches!(
            outdent(&mut ws, &hlc, a),
            Err(ActionError::AlreadyAtRoot(_))
        ));
    }

    #[test]
    fn move_up_swaps_with_previous_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_up(&mut ws, &hlc, b).unwrap();
        let order: Vec<NodeId> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![b, a]);
    }

    #[test]
    fn move_down_swaps_with_next_sibling() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_down(&mut ws, &hlc, a).unwrap();
        let order: Vec<NodeId> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![b, a]);
    }

    #[test]
    fn move_up_first_sibling_is_noop() {
        let (mut ws, hlc) = new_workspace();
        let a = append_block(&mut ws, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut ws, &hlc, None, Some("b")).unwrap();
        move_up(&mut ws, &hlc, a).unwrap();
        let order: Vec<NodeId> = crate::tree::children_of(&ws, NodeId::root())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![a, b]);
    }
}
