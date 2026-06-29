//! Property model. Properties are key-value pairs attached to a node.
//!
//! Page-level properties live at the top of a `.md` file; block-level
//! properties are children of a block with `key:: value` syntax. Internally
//! both routes resolve to `SetProp` ops on the relevant node.

use serde::{Deserialize, Serialize};

/// Value types supported as property values.
///
/// The surface is intentionally narrow today; the query DSL may expand it later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropValue {
    /// Plain text value.
    Text(String),
    /// Reference to a page by name (e.g. `[[avelino]]`).
    PageRef(String),
    /// Tag reference (e.g. `#produto`).
    Tag(String),
    /// Multiple values (e.g. `tags:: #a #b`).
    List(Vec<PropValue>),
}

/// A property on a node — name + value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    /// Property key (e.g. `priority`).
    pub key: String,
    /// Property value.
    pub value: PropValue,
}
