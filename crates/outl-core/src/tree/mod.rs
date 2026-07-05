//! The tree CRDT.
//!
//! This module implements the algorithm described in:
//!
//! > Martin Kleppmann, Dominic P. Mulligan, Victor B. F. Gomes, Alastair
//! > R. Beresford. *"A highly-available move operation for replicated trees."*
//! > IEEE TPDS 2022. <https://martin.kleppmann.com/papers/move-op.pdf>
//!
//! The four functions that carry the entire correctness contract of
//! the CRDT live in submodules so each can stay small and auditable:
//!
//! | Submodule  | Function(s)                          |
//! |------------|--------------------------------------|
//! | `cycle`    | `Tree::creates_cycle`                |
//! | `op`       | `Tree::do_op`, `Tree::undo_op`       |
//! | `apply`    | `Tree::apply_op` (orchestrator)      |
//!
//! They must each match the paper line-by-line (see `docs/crdt.md`)
//! and remain at 100 % coverage forever. Tests for the algorithm
//! live in `crates/outl-core/tests/tree_unit.rs` (integration tests)
//! so the source module stays focused on the algorithm itself.
//!
//! See `crates/outl-core/CLAUDE.md` for the five invariants.

use crate::fractional::Fractional;
use crate::id::NodeId;
use crate::property::PropValue;
use std::collections::{HashMap, HashSet};

mod apply;
mod cycle;
mod op;

/// Materialized outline tree.
///
/// Stores `(parent, position)` for every node and property triples
/// `(node, key) -> value`. Block text content (Yrs `Doc`s) lives in
/// `Workspace`, not here — the tree CRDT itself is purely structural.
///
/// Construct via [`Tree::new`]; mutate via [`Tree::apply_op`].
#[derive(Debug, Default, Clone)]
pub struct Tree {
    pub(super) nodes: HashMap<NodeId, (NodeId, Fractional)>,
    pub(super) properties: HashMap<(NodeId, String), PropValue>,
    /// Nodes whose [`crate::op::Op::SetCollapsed`] last resolved to `true`.
    /// Absence means expanded (the default for every node, including
    /// ones the op log has never set explicitly).
    ///
    /// Stored as a set rather than a `HashMap<_, bool>` so the "no
    /// entry" / "false" cases share representation and serialised
    /// projections (the sidecar, the wire JSON Mobile receives) don't
    /// distinguish "we know it's expanded" from "we never heard about
    /// this node".
    pub(super) collapsed: HashSet<NodeId>,
}

/// Borrowed view of the three interior maps, returned by
/// [`Tree::snapshot_parts`]. Used by the snapshot path so it can
/// serialize the materialized tree without reaching into private
/// fields.
pub(crate) type TreeParts<'a> = (
    &'a HashMap<NodeId, (NodeId, Fractional)>,
    &'a HashMap<(NodeId, String), PropValue>,
    &'a HashSet<NodeId>,
);

impl Tree {
    /// Build an empty tree.
    ///
    /// `ROOT` and `TRASH_ROOT` are implicit — they don't appear in `nodes`
    /// but are valid parents for any other node.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the tree currently contains a record for this node.
    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    /// Current parent of a node, or `None` if the node is not in the tree.
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node).map(|(p, _)| *p)
    }

    /// Current position of a node, or `None` if the node is not in the tree.
    pub fn position(&self, node: NodeId) -> Option<&Fractional> {
        self.nodes.get(&node).map(|(_, p)| p)
    }

    /// Current value of a property, or `None` if unset.
    pub fn property(&self, node: NodeId, key: &str) -> Option<&PropValue> {
        self.properties.get(&(node, key.to_string()))
    }

    /// Every property currently set on `node`, as `(key, value)` pairs
    /// in unspecified order.
    ///
    /// Used by projection layers (mobile / TUI outline DTO) to
    /// snapshot the block's properties in one pass without scanning
    /// the whole property map. Callers that need a stable order
    /// should sort the result themselves — the underlying `HashMap`
    /// makes no ordering guarantees.
    pub fn properties_of(&self, node: NodeId) -> impl Iterator<Item = (&str, &PropValue)> {
        self.properties
            .iter()
            .filter(move |((n, _), _)| *n == node)
            .map(|((_, k), v)| (k.as_str(), v))
    }

    /// Whether `node` is currently rendered collapsed (children
    /// hidden in the outline view). Defaults to `false` for any node
    /// the op log has never explicitly set.
    pub fn is_collapsed(&self, node: NodeId) -> bool {
        self.collapsed.contains(&node)
    }

    /// Iterator over every node currently flagged collapsed. Used by
    /// projection layers (sidecar render, mobile wire format) to
    /// snapshot the fold state in one pass.
    pub fn collapsed_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.collapsed.iter().copied()
    }

    /// Iterate every (node, parent, position) triple. Useful for tests.
    pub fn iter_nodes(&self) -> impl Iterator<Item = (NodeId, NodeId, &Fractional)> {
        self.nodes.iter().map(|(n, (p, pos))| (*n, *p, pos))
    }

    /// Total number of nodes in the tree.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Total number of property bindings.
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Borrow the three interior maps as a tuple, in the order
    /// `(nodes, properties, collapsed)`. Used by the snapshot path to
    /// serialize the materialized tree without exposing the private
    /// field names beyond this crate.
    pub(crate) fn snapshot_parts(&self) -> TreeParts<'_> {
        (&self.nodes, &self.properties, &self.collapsed)
    }

    /// Rebuild a `Tree` directly from its three interior maps. Used by
    /// the snapshot path to hydrate the materialized state without
    /// replaying the op log. The maps are trusted: callers must ensure
    /// they came from a valid (validated-content-hash) snapshot.
    pub(crate) fn from_parts(
        nodes: HashMap<NodeId, (NodeId, Fractional)>,
        properties: HashMap<(NodeId, String), PropValue>,
        collapsed: HashSet<NodeId>,
    ) -> Self {
        Self {
            nodes,
            properties,
            collapsed,
        }
    }
}
