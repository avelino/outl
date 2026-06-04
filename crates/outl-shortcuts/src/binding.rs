//! `Binding` and `Mode` — what mode is the user in, and what
//! action does a chord resolve to in that mode.

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::chord::ChordSequence;

/// Editor mode the chord catalog applies to.
///
/// `Global` matches everywhere — chrome shortcuts like the picker
/// or "open today" don't care whether the user is editing a block.
/// `Overlay` is when a modal popup (picker, command palette, help)
/// owns the keystrokes; the catalog has its own bindings for those.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Global,
    Normal,
    Insert,
    Visual,
    Overlay,
}

/// One row of the binding catalog.
///
/// `description` is what the help overlay renders next to the chord
/// (`"Open today's journal"`, `"New block below"`). It also makes
/// reading the wire format with `jq` actually informative.
///
/// `description` is `String` (not `&'static str`) because Serde
/// can't borrow a static slice out of a freshly-parsed JSON blob
/// — the catalog rides the Tauri wire so deserialise has to work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub chord: ChordSequence,
    pub mode: Mode,
    pub action: Action,
    pub description: String,
}

impl Binding {
    /// Construct a binding. `description` accepts anything that
    /// can be turned into a `String` so the default table reads
    /// compactly with string literals.
    pub fn new(
        chord: ChordSequence,
        mode: Mode,
        action: Action,
        description: impl Into<String>,
    ) -> Self {
        Self {
            chord,
            mode,
            action,
            description: description.into(),
        }
    }
}
