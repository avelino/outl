//! Data that crosses the JS ↔ host boundary.
//!
//! The plugin runtime is **describe → apply**: the JS side reads from a
//! pre-computed [`ReadModel`] and emits [`HostIntent`]s into a buffer; the host
//! (which holds `&mut Workspace`) drains the buffer and applies each intent
//! through `outl-actions`. Nothing here borrows the workspace — every type is
//! owned and `serde`-serializable so it can be handed to Boa as plain JSON.

use serde::{Deserialize, Serialize};

use crate::permission::Permission;

/// What a content transformer returned for a block: how to render it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformResult {
    /// `"text"` (content is text/markdown the client renders normally) or
    /// `"rich"` (content is HTML the GUI client runs in a sandboxed iframe).
    pub kind: String,
    /// The rendered content (text or HTML, per `kind`).
    pub content: String,
}

/// A read-only snapshot the JS side queries during a turn. Built by the host
/// before invoking a command or dispatching an op.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadModel {
    /// Every block in the workspace with its text and todo state.
    pub blocks: Vec<BlockView>,
    /// Every page (and journal) by slug.
    pub pages: Vec<PageView>,
    /// Every template (pages with a non-empty `template::` property).
    pub templates: Vec<TemplateView>,
    /// The op currently being dispatched, when inside an `onOp` turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<LogOpView>,
}

/// A block as the JS side sees it. `id` is an opaque ULID string the plugin
/// never constructs — it only echoes ids back in intents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockView {
    /// Opaque block id (stringified `NodeId`).
    pub id: String,
    /// Body text **without** the TODO/DONE prefix.
    pub text: String,
    /// `"TODO"`, `"DONE"`, or absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<String>,
    /// Slug of the page this block belongs to.
    pub page: String,
}

/// A page as the JS side sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageView {
    /// Stable slug.
    pub slug: String,
    /// Human title.
    pub title: String,
    /// `"page"` or `"journal"`.
    pub kind: String,
}

/// A template page as the JS side sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateView {
    /// Invocation name (the value of `template::`).
    pub name: String,
    /// Page slug.
    pub slug: String,
    /// Declared parameter names (empty for structural templates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
}

/// The op being dispatched to `onOp`, projected to a stable JS shape. This is a
/// projection, not a mirror of the Rust `Op` enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogOpView {
    /// Coarse op kind: `"Create" | "Move" | "Edit" | "SetProp" | "SetCollapsed"`.
    pub kind: String,
    /// The node the op touched (stringified `NodeId`).
    pub node: String,
    /// New text body, present for `Edit` ops (TODO/DONE prefix stripped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Todo state after an `Edit`, when the body carried a TODO/DONE prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<String>,
}

/// A mutation the JS side asks the host to perform. The host applies each one
/// through `outl-actions`, after checking the plugin holds the right
/// permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum HostIntent {
    /// Replace a block's text verbatim.
    EditText {
        /// Target block id.
        node: String,
        /// New wire text (TODO/DONE prefix included if any).
        text: String,
    },
    /// Create a block as the last child of `parent`.
    CreateUnder {
        /// Parent block id.
        parent: String,
        /// Initial text.
        text: String,
    },
    /// Create a sibling right after `after`.
    CreateAfter {
        /// Anchor block id.
        after: String,
        /// Initial text.
        text: String,
    },
    /// Move a block under a page (by slug) or under another block.
    Move {
        /// Block to move.
        node: String,
        /// Where it lands.
        target: MoveTarget,
    },
    /// Cycle a block's TODO state.
    ToggleTodo {
        /// Target block id.
        node: String,
    },
    /// Delete a block (move to trash).
    Delete {
        /// Target block id.
        node: String,
    },
    /// Ensure a page exists (create it if missing). Idempotent on the slug.
    EnsurePage {
        /// Page slug to create.
        slug: String,
    },
    /// Instantiate a structural template under a target block.
    InstantiateTemplate {
        /// Template invocation name.
        name: String,
        /// Target block id to instantiate under.
        under: String,
    },
}

impl HostIntent {
    /// The permission a plugin must hold to have this intent applied.
    pub fn required_permission(&self) -> Permission {
        // Every intent writes, so all need `write-page`. `submit-op` is the
        // companion the loader requires alongside it; we gate on the narrower
        // `write-page` here and let the loader enforce both at install.
        Permission::WritePage
    }
}

/// Destination for a [`HostIntent::Move`]. Untagged so the JS side can emit a
/// bare `{ toPage }` / `{ toParent }` object (what the SDK produces) instead of
/// an externally-tagged wrapper. Fields are renamed per-field because
/// `rename_all` on an untagged enum only touches variant names, not the inner
/// fields the JSON actually carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MoveTarget {
    /// Move under a page identified by slug (created if missing).
    ToPage {
        /// Destination page slug.
        #[serde(rename = "toPage")]
        to_page: String,
    },
    /// Move under another block.
    ToParent {
        /// Destination parent block id.
        #[serde(rename = "toParent")]
        to_parent: String,
    },
}

/// What a single JS turn produced: the intents to apply plus any user-facing
/// output. Returned by the engine, consumed by the host.
#[derive(Debug, Clone, Default)]
pub struct TurnOutput {
    /// Mutations to apply, in order.
    pub intents: Vec<HostIntent>,
    /// `console.log` / `ctx.log` lines, for the host to surface in dev tools.
    pub logs: Vec<String>,
    /// `ctx.ui.notify` messages, for the host to show the user.
    pub notifications: Vec<String>,
    /// `ctx.ui.render` payloads — author-written HTML/JS the GUI client runs in
    /// a sandboxed iframe. The engine never interprets these; it only carries
    /// the string the plugin produced (so the author, not the host, owns what
    /// the effect is).
    pub views: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_serde_uses_tagged_kebab() {
        let intent = HostIntent::Move {
            node: "01ABC".into(),
            target: MoveTarget::ToPage {
                to_page: "archive".into(),
            },
        };
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(
            json,
            r#"{"op":"move","node":"01ABC","target":{"toPage":"archive"}}"#
        );
        let back: HostIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }

    #[test]
    fn edit_text_intent_roundtrips() {
        let intent = HostIntent::EditText {
            node: "n1".into(),
            text: "DONE x".into(),
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: HostIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }

    #[test]
    fn all_intents_require_write() {
        let i = HostIntent::Delete { node: "n".into() };
        assert_eq!(i.required_permission(), Permission::WritePage);
    }
}
