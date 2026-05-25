//! Tag model. A tag is a page-reference with classification semantics.
//!
//! `#produto` and `[[produto]]` resolve to the same underlying page, but
//! the UI treats them differently: tags appear in filter sidebars, queries,
//! and grouping; page references appear in backlinks and the graph view.

use serde::{Deserialize, Serialize};

/// A tag — a thin wrapper around a page name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    /// The page name this tag refers to (no `#` prefix).
    pub name: String,
}

impl Tag {
    /// Build a `Tag` from a `#tag` token, stripping the leading `#`.
    pub fn parse(token: &str) -> Option<Self> {
        token.strip_prefix('#').map(|name| Self {
            name: name.to_string(),
        })
    }
}
