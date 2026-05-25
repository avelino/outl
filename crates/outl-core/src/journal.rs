//! Journal model. A journal is a page keyed by date.
//!
//! Files live in `<workspace>/journals/YYYY-MM-DD.md`. The journal of
//! today opens by default in the TUI.

use crate::id::NodeId;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A daily journal page.
///
/// Journal pages have date-as-name. They are otherwise regular pages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// Stable identifier of the journal page (and of its root block).
    pub id: NodeId,
    /// Calendar date the journal corresponds to.
    pub date: NaiveDate,
}

impl Journal {
    /// Filename for this journal (without `.md`).
    pub fn filename(&self) -> String {
        self.date.format("%Y-%m-%d").to_string()
    }
}
