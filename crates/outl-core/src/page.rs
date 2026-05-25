//! Page model. A page is a named container for an outline.
//!
//! Pages map 1:1 to `.md` files in `<workspace>/pages/`. The page's root
//! block is identified by `NodeId`; its tree of children is what the user
//! sees in the file.

use crate::id::NodeId;
use crate::property::Property;
use serde::{Deserialize, Serialize};

/// A page in the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    /// Stable identifier of the page (and of its root block).
    pub id: NodeId,
    /// Human-readable name (filename without `.md`).
    pub name: String,
    /// Page-level properties (rendered at the top of the `.md`).
    pub properties: Vec<Property>,
}
