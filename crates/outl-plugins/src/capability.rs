//! Capability model.
//!
//! A capability is something a plugin *registers* and a client *implements*.
//! The loader intersects the two: a capability the plugin declares but the
//! current client cannot honor loads partially with a user-visible warning,
//! never a silent crash (see [`intersect`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// What a plugin can plug into. Serialized as the kebab/colon strings from the
/// RFC (`op-hook`, `content-transformer:text`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// React to ops as they are applied to the log.
    #[serde(rename = "op-hook")]
    OpHook,
    /// Register a command invokable from a slash menu / palette.
    #[serde(rename = "slash-command")]
    SlashCommand,
    /// Bind a chord to one of the plugin's commands.
    #[serde(rename = "keybinding")]
    Keybinding,
    /// Expose a user-editable config schema.
    #[serde(rename = "config-schema")]
    ConfigSchema,
    /// Render ephemeral HTML/JS the plugin author writes, in a sandboxed iframe
    /// (GUI clients only — the engine stays agnostic, it only transports the
    /// markup; the client executes it isolated from the app DOM).
    #[serde(rename = "ui-render")]
    UiRender,
    /// Turn a block into a textual descriptor each client renders its own way.
    #[serde(rename = "content-transformer:text")]
    ContentTransformerText,
    /// Turn a block into a rich descriptor (GUI clients only).
    #[serde(rename = "content-transformer:rich")]
    ContentTransformerRich,
    /// Provide query results (not yet wired).
    #[serde(rename = "query-provider")]
    QueryProvider,
    /// Provide a sync transport (not yet wired).
    #[serde(rename = "sync-transport")]
    SyncTransport,
    /// Contribute a toolbar button (GUI clients only).
    #[serde(rename = "toolbar-button")]
    ToolbarButton,
}

/// The set of capabilities a given client implements. Each client builds this
/// once at startup and hands it to the loader.
pub type ClientCapabilities = BTreeSet<Capability>;

/// Outcome of matching a plugin's declared capabilities against what the
/// current client implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMatch {
    /// Capabilities both declared and implemented — these activate.
    pub granted: BTreeSet<Capability>,
    /// Capabilities the plugin declared but this client cannot honor — these
    /// surface a warning, the plugin still loads for the rest.
    pub missing: BTreeSet<Capability>,
}

impl CapabilityMatch {
    /// True when every declared capability is implemented by the client.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Intersect what a plugin declares with what the client implements.
///
/// Capability missing on this client is **not** an error: it lands in
/// [`CapabilityMatch::missing`] so the host can warn and load the rest.
pub fn intersect(declared: &[Capability], client: &ClientCapabilities) -> CapabilityMatch {
    let mut granted = BTreeSet::new();
    let mut missing = BTreeSet::new();
    for cap in declared {
        if client.contains(cap) {
            granted.insert(*cap);
        } else {
            missing.insert(*cap);
        }
    }
    CapabilityMatch { granted, missing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(caps: &[Capability]) -> ClientCapabilities {
        caps.iter().copied().collect()
    }

    #[test]
    fn serde_roundtrip_uses_rfc_strings() {
        let json = serde_json::to_string(&Capability::ContentTransformerText).unwrap();
        assert_eq!(json, "\"content-transformer:text\"");
        let back: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Capability::ContentTransformerText);
    }

    #[test]
    fn unknown_capability_string_fails() {
        let err = serde_json::from_str::<Capability>("\"not-a-cap\"");
        assert!(err.is_err());
    }

    #[test]
    fn intersect_splits_granted_and_missing() {
        let declared = [
            Capability::OpHook,
            Capability::SlashCommand,
            Capability::ContentTransformerRich,
        ];
        let m = intersect(
            &declared,
            &client(&[Capability::OpHook, Capability::SlashCommand]),
        );
        assert!(m.granted.contains(&Capability::OpHook));
        assert!(m.granted.contains(&Capability::SlashCommand));
        assert!(m.missing.contains(&Capability::ContentTransformerRich));
        assert!(!m.is_complete());
    }

    #[test]
    fn fully_supported_plugin_is_complete() {
        let declared = [Capability::OpHook];
        let m = intersect(
            &declared,
            &client(&[Capability::OpHook, Capability::Keybinding]),
        );
        assert!(m.is_complete());
        assert!(m.missing.is_empty());
    }
}
