//! Wire shapes for the plugin surface.
//!
//! These are the `Serialize` DTOs the frontend receives from the
//! [`crate::plugin_service::PluginService`] requests. They are kept in a
//! sibling module so `plugin_service.rs` stays focused on the thread /
//! channel machinery; each DTO carries the `From<…>` projection off the
//! corresponding `outl-plugins` type.

use outl_plugins::{
    CommandEntry, PluginBinding, PluginRun, ToolbarButtonEntry, TransformResult, TransformerEntry,
};
use outl_shortcuts::{ChordSequence, Mode};
use serde::Serialize;

/// One plugin command, projected to the wire shape the frontend lists.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginCommandDto {
    pub plugin_id: String,
    pub command_id: String,
    pub title: String,
}

impl From<CommandEntry> for PluginCommandDto {
    fn from(c: CommandEntry) -> Self {
        Self {
            plugin_id: c.plugin_id,
            command_id: c.command_id,
            title: c.title,
        }
    }
}

/// One plugin keybinding, projected to the wire shape the frontend's
/// chord dispatcher compares against.
///
/// `chord` and `mode` serialize identically to the `outl-shortcuts`
/// catalog the frontend already parses (`ChordSequence` is
/// `#[serde(transparent)]` over `Vec<Chord>`, `Mode` is lowercase), so the
/// frontend reuses its existing `Chord` / `ShortcutMode` types and `seqEq`
/// comparison — no parallel parser. Plugin chords are always `global`, but
/// the field is carried explicitly so the frontend never has to assume it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginKeybindingDto {
    pub chord: ChordSequence,
    pub mode: Mode,
    pub plugin_id: String,
    pub command_id: String,
    pub description: String,
}

impl From<PluginBinding> for PluginKeybindingDto {
    fn from(b: PluginBinding) -> Self {
        Self {
            chord: b.chord,
            mode: b.mode,
            plugin_id: b.plugin_id,
            command_id: b.command_id,
            description: b.description,
        }
    }
}

/// One plugin toolbar button, projected to the wire shape the chrome
/// renders (one `<ChromeToggle>`-style button per entry).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolbarButtonDto {
    pub plugin_id: String,
    pub command_id: String,
    pub icon: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl From<ToolbarButtonEntry> for ToolbarButtonDto {
    fn from(e: ToolbarButtonEntry) -> Self {
        Self {
            plugin_id: e.plugin_id,
            command_id: e.command_id,
            icon: e.icon,
            title: e.title,
        }
    }
}

/// One content transformer a plugin declares for a code-fence language,
/// projected to the wire shape the frontend matches fences against.
///
/// The frontend loads the list once per workspace open and, when a fence's
/// language matches a `lang` here, calls `plugin_transform` to render it —
/// `text` content goes through the normal markdown renderer, `rich` content
/// runs in a sandboxed iframe (same isolation as `ui-render`).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformerDto {
    pub plugin_id: String,
    pub lang: String,
    /// `"text"` or `"rich"`.
    pub kind: String,
}

impl From<TransformerEntry> for TransformerDto {
    fn from(t: TransformerEntry) -> Self {
        Self {
            plugin_id: t.plugin_id,
            lang: t.lang,
            kind: t.kind,
        }
    }
}

/// The descriptor a content transformer produced for a fence body.
///
/// `kind` is `"text"` (content is markdown/text the client renders inline)
/// or `"rich"` (content is HTML the client runs in a sandboxed iframe —
/// untrusted plugin output, never injected into the app DOM).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransformResultDto {
    pub kind: String,
    pub content: String,
}

impl From<TransformResult> for TransformResultDto {
    fn from(r: TransformResult) -> Self {
        Self {
            kind: r.kind,
            content: r.content,
        }
    }
}

/// Outcome of running a plugin command, surfaced to the frontend.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginRunDto {
    /// Number of intents the plugin applied to the workspace.
    pub applied: usize,
    /// `ctx.ui.notify` messages — shown as info status lines.
    pub notifications: Vec<String>,
    /// Non-fatal plugin errors — shown as error status lines.
    pub errors: Vec<String>,
    /// HTML/JS documents a plugin emitted via `ctx.ui.render(html)`
    /// (gated by the `ui-render` capability upstream). The frontend
    /// runs each in an ephemeral sandboxed `<iframe>` overlay — these
    /// are untrusted plugin strings, never trusted app markup.
    pub views: Vec<String>,
}

impl From<PluginRun> for PluginRunDto {
    fn from(r: PluginRun) -> Self {
        Self {
            applied: r.applied,
            notifications: r.notifications,
            errors: r.errors,
            views: r.views,
        }
    }
}

/// One marketplace row: a registry entry plus this workspace's local state
/// (whether it's installed, and if so whether it's enabled). The frontend
/// renders the browse list + the install / enable / disable affordances off
/// this.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RegistryItemDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub latest: Option<String>,
    /// Already present in this workspace's lockfile.
    pub installed: bool,
    /// Installed *and* enabled (false when installed-but-disabled, or not
    /// installed at all).
    pub enabled: bool,
}
