//! Op log primitives: the `Op` enum and the `LogOp` envelope.
//!
//! Every mutation to the materialized tree is expressed as an `Op` wrapped
//! in a `LogOp` (HLC + actor + op). The op log is the source of truth;
//! the tree is a projection.
//!
//! Adding a new `Op` variant is non-trivial — see `/new-op` slash command.

use crate::fractional::Fractional;
use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::property::PropValue;
use serde::{Deserialize, Serialize};

/// A single mutation to the outline.
///
/// `Move` is the operation whose concurrent semantics are the heart of the
/// algorithm. `Edit` carries a Yrs binary update for block content.
/// `SetProp` and `Create` round out the surface.
///
/// Note: there is no `Delete` variant. Deletion is `Move(node, TRASH_ROOT)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Move a node to a new parent and/or position.
    ///
    /// `old_parent` and `old_position` are populated by `do_op` so that
    /// `undo_op` can revert. They are not meaningful before the op is
    /// applied for the first time.
    Move {
        /// The node being moved.
        node: NodeId,
        /// New parent of the node.
        new_parent: NodeId,
        /// Position of the node among siblings of `new_parent`.
        position: Fractional,
        /// Filled by `do_op`. Required for `undo_op`.
        old_parent: NodeId,
        /// Filled by `do_op`. Required for `undo_op`.
        old_position: Fractional,
    },

    /// Apply a Yrs binary update to a block's content.
    Edit {
        /// The block whose content is edited.
        node: NodeId,
        /// Yrs `encode_update_v1` bytes.
        text_op: Vec<u8>,
    },

    /// Set or clear a property on a node.
    ///
    /// `old_value` is populated by `do_op` for undo.
    SetProp {
        /// The node owning the property.
        node: NodeId,
        /// Property key.
        key: String,
        /// `None` removes the property.
        value: Option<PropValue>,
        /// Filled by `do_op` for `undo_op`.
        old_value: Option<PropValue>,
    },

    /// Create a new node under a given parent and position.
    ///
    /// Idempotent: re-applying for an already-existing node is a no-op.
    Create {
        /// The new node's id.
        node: NodeId,
        /// Initial parent.
        parent: NodeId,
        /// Initial position among siblings.
        position: Fractional,
    },

    /// Set the **collapsed** (folded) flag of a node.
    ///
    /// Controls whether the block's children are hidden in the outline
    /// view. UI presentation, but globally meaningful — folding a
    /// block on one device shows up folded on every other device.
    ///
    /// **Going through `Op` is the canonical path for any per-block
    /// state that must converge between devices.** Writing such state
    /// straight to a sidecar would lose under iCloud / Syncthing's
    /// last-write-wins-per-file semantics; the op log gives each
    /// device its own `ops-<actor>.jsonl` and lets the CRDT merge
    /// concurrent flips by HLC ordering. Idempotent re-apply of the
    /// same `LogOp` is a no-op (the HLC dedup at the top of
    /// [`crate::tree::Tree::apply_op`] guarantees this).
    ///
    /// `old_value` is populated by `do_op` for `undo_op`.
    SetCollapsed {
        /// The node being folded / unfolded.
        node: NodeId,
        /// Desired flag.
        value: bool,
        /// Filled by `do_op` for `undo_op`.
        old_value: bool,
    },
}

/// Extract the `NodeId` an op targets, if any. Every `Op` variant
/// carries one — there is no op that touches zero nodes. Returns
/// `Option` so callers can `filter_map` cleanly. Used by the migrate
/// CLI to route ops to per-page shards (RFC #137 Phase B).
pub fn op_node(op: &Op) -> Option<NodeId> {
    match op {
        Op::Create { node, .. }
        | Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::SetCollapsed { node, .. } => Some(*node),
    }
}

/// An op wrapped with its HLC and actor.
///
/// `LogOp`s are what is stored, sorted, and exchanged between peers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogOp {
    /// HLC timestamp; defines total order.
    pub ts: Hlc,
    /// Originating actor (also embedded inside `ts` for tiebreak).
    pub actor: ActorId,
    /// The mutation itself.
    pub op: Op,
}
