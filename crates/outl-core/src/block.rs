//! Block model. A block is a node in the outline tree with optional text
//! content (stored as a Yrs `Doc` for collaborative editing).
//!
//! Step 1 ships the struct; Step 2 wires the Yrs doc lifecycle.

use crate::id::NodeId;
use crate::property::Property;
use serde::{Deserialize, Serialize};

/// A block in the outline.
///
/// Materialized projection over the op log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Stable identifier for the block.
    pub id: NodeId,
    /// Plain text view of the block's content.
    ///
    /// Step 2: real content is held in a Yrs `TextRef`. This field is
    /// the rendered snapshot at the moment of materialization.
    pub text: String,
    /// Block properties.
    pub properties: Vec<Property>,
}
