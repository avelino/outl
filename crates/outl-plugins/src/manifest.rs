//! `plugin.json` — the contract between a plugin and the host.
//!
//! A single JSON file at the plugin root. It is the *stable surface*: the host
//! can list a plugin's commands and required permissions **without executing
//! its JS**. Parsing is permissive on optional metadata and strict on the
//! load-bearing fields (id shape, capabilities, permissions, semver ranges).

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::capability::Capability;
use crate::error::{PluginError, Result};
use crate::permission::Permission;

/// The parsed `plugin.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Reverse-DNS identity, e.g. `app.outl.examples.todo-archiver`. This — not
    /// `name` — identifies the plugin on disk, in the lockfile, in the index.
    pub id: String,

    /// Human display name.
    pub name: String,

    /// Plugin's own semver version.
    pub version: Version,

    /// One-line description. Optional.
    #[serde(default)]
    pub description: Option<String>,

    /// Range of the **plugin API** this plugin targets, e.g. `^1.0`. Decoupled
    /// from the binary version so the API can be stable while the app moves.
    pub api: VersionReq,

    /// Minimum outl binary the plugin needs.
    #[serde(default)]
    pub engines: Engines,

    /// Bundled entry point, relative to the plugin root (e.g. `index.js`).
    pub main: String,

    /// Optional icon path, relative to the plugin root.
    #[serde(default)]
    pub icon: Option<String>,

    /// Capabilities the plugin registers. The loader intersects these with
    /// what the client implements.
    #[serde(default)]
    pub capabilities: Vec<Capability>,

    /// Permissions the plugin needs. Approved by the user on install.
    #[serde(default)]
    pub permissions: Vec<Permission>,

    /// Declarative contributions (commands, keybindings, config schema). The
    /// host can read these without running JS.
    #[serde(default)]
    pub contributes: Contributes,

    /// Free-form metadata, none of it load-bearing.
    #[serde(flatten)]
    pub meta: Meta,
}

/// Minimum-binary requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engines {
    /// Minimum outl version range, e.g. `>=0.7.0`.
    #[serde(default = "any_version")]
    pub outl: VersionReq,
}

impl Default for Engines {
    fn default() -> Self {
        Self {
            outl: any_version(),
        }
    }
}

fn any_version() -> VersionReq {
    VersionReq::STAR
}

/// Declarative contributions surfaced to the host without executing JS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contributes {
    /// Commands the plugin exposes (palette / slash menu).
    #[serde(default)]
    pub commands: Vec<Command>,
    /// Default keybindings for the plugin's commands.
    #[serde(default)]
    pub keybindings: Vec<Keybinding>,
    /// Path to a JSON Schema describing user-editable config.
    #[serde(default, rename = "configSchema")]
    pub config_schema: Option<String>,
    /// Toolbar buttons the plugin contributes to GUI clients' chrome.
    #[serde(default)]
    pub toolbar: Vec<ToolbarButton>,
    /// Content transformers: code-fence languages the plugin renders into a
    /// descriptor each client draws (capabilities `content-transformer:text` /
    /// `:rich`). Declaring the language here lets a client skip running JS for
    /// fences no plugin transforms.
    #[serde(default)]
    pub transformers: Vec<TransformerDecl>,
}

/// One content transformer a plugin declares. The plugin registers the matching
/// function in JS via `ctx.content.register(lang, fn)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformerDecl {
    /// Code-fence language this transformer handles (e.g. `"mermaid"`).
    pub lang: String,
    /// `"text"` (descriptor is rendered as text/markdown on every client) or
    /// `"rich"` (descriptor is HTML, run in a sandboxed iframe on GUI clients).
    #[serde(default = "default_transform_kind")]
    pub kind: String,
}

fn default_transform_kind() -> String {
    "text".to_string()
}

/// A toolbar button a plugin contributes (GUI clients render it in their
/// chrome; tapping it runs the referenced command).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolbarButton {
    /// The command id this button triggers.
    pub command: String,
    /// A short glyph/emoji shown on the button (e.g. `"📊"`).
    pub icon: String,
    /// Optional tooltip / accessible label.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional client scope (`desktop`, `mobile`).
    #[serde(default)]
    pub when: Option<String>,
}

/// A command a plugin contributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// `kebab-case` command id, unique within the plugin.
    pub id: String,
    /// Human title shown in the palette.
    pub title: String,
}

/// A default keybinding for a contributed command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keybinding {
    /// The command id this chord triggers.
    pub command: String,
    /// The chord, e.g. `Ctrl+T S`.
    pub key: String,
    /// Optional client scope (`tui`, `desktop`, `mobile`).
    #[serde(default)]
    pub when: Option<String>,
}

/// Optional, non-load-bearing manifest metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    /// Author block.
    #[serde(default)]
    pub author: Option<Author>,
    /// SPDX license id.
    #[serde(default)]
    pub license: Option<String>,
    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// Funding URL.
    #[serde(default)]
    pub funding: Option<String>,
    /// Locales the plugin ships strings for (BCP-47).
    #[serde(default)]
    pub locales: Vec<String>,
    /// Search keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Single category for the index.
    #[serde(default)]
    pub category: Option<String>,
}

/// Author block. Accepts both npm forms: a single string
/// (`"Avelino <avelinorun@gmail.com>"`) or a structured object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Author {
    /// npm-style single string.
    Text(String),
    /// Structured author block.
    Detailed {
        /// Author name.
        name: String,
        /// Optional email.
        #[serde(default)]
        email: Option<String>,
        /// Optional URL.
        #[serde(default)]
        url: Option<String>,
    },
}

impl Author {
    /// The author's display name, parsed out of either form.
    pub fn display_name(&self) -> &str {
        match self {
            // For the string form, the name is everything before `<` or `(`.
            Self::Text(s) => s
                .split(['<', '('])
                .next()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(s),
            Self::Detailed { name, .. } => name,
        }
    }
}

impl PluginManifest {
    /// Parse and validate from raw JSON bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the load-bearing fields beyond what serde already enforces.
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        if self.name.trim().is_empty() {
            return Err(PluginError::Manifest("name must not be empty".into()));
        }
        // `main` and any `config_schema` are read from disk relative to the
        // plugin's own directory (`.outl/plugins/<id>/`) on install AND on
        // every load, so they must stay inside it. A `..` or absolute path
        // would let a crafted manifest read/write outside the plugin dir
        // (path traversal) — defense in depth for the registry/marketplace,
        // where a published manifest a human reviewed reaches every installer.
        validate_resource_path("main", &self.main)?;
        if let Some(schema) = &self.contributes.config_schema {
            validate_resource_path("config_schema", schema)?;
        }
        // Every keybinding must reference a declared command — a binding to a
        // command that doesn't exist is a silent dead key otherwise.
        for kb in &self.contributes.keybindings {
            if !self.contributes.commands.iter().any(|c| c.id == kb.command) {
                return Err(PluginError::Manifest(format!(
                    "keybinding references unknown command `{}`",
                    kb.command
                )));
            }
        }
        // Same rule for toolbar buttons — a button to a missing command is dead.
        for tb in &self.contributes.toolbar {
            if !self.contributes.commands.iter().any(|c| c.id == tb.command) {
                return Err(PluginError::Manifest(format!(
                    "toolbar button references unknown command `{}`",
                    tb.command
                )));
            }
        }
        Ok(())
    }

    /// Whether this plugin is compatible with the host's plugin-API version.
    pub fn is_api_compatible(&self, host_api: &Version) -> bool {
        self.api.matches(host_api)
    }
}

/// Reject a manifest resource path (`main`, `config_schema`) that could
/// escape the plugin's directory when joined onto it. The path is read from
/// disk relative to `.outl/plugins/<id>/`, so an absolute path or a `..`
/// component would traverse out of it. Rejects: empty, absolute (`/…`,
/// `\…`, `~…`, a drive `C:…`), and any `..` segment (both `/` and `\`
/// separators, so a Windows-style `..\x` is caught too).
fn validate_resource_path(field: &str, value: &str) -> Result<()> {
    let v = value.trim();
    if v.is_empty() {
        return Err(PluginError::Manifest(format!("{field} must not be empty")));
    }
    let escapes = v.starts_with('/')
        || v.starts_with('\\')
        || v.starts_with('~')
        || v.contains(':')
        || v.split(['/', '\\']).any(|seg| seg == "..");
    if escapes {
        return Err(PluginError::Manifest(format!(
            "{field} must be a relative path inside the plugin, got `{value}`"
        )));
    }
    Ok(())
}

/// Validate a reverse-DNS plugin id: lowercase, dot-separated labels of
/// `[a-z0-9-]`, at least two labels, no leading/trailing dot or hyphen.
fn validate_id(id: &str) -> Result<()> {
    let bad = |why: &str| Err(PluginError::Manifest(format!("invalid id `{id}`: {why}")));
    if id.is_empty() {
        return bad("empty");
    }
    let labels: Vec<&str> = id.split('.').collect();
    if labels.len() < 2 {
        return bad("must be reverse-DNS (at least two dot-separated labels)");
    }
    for label in labels {
        if label.is_empty() {
            return bad("empty label (leading, trailing, or doubled dot)");
        }
        if label.starts_with('-') || label.ends_with('-') {
            return bad("label must not start or end with a hyphen");
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return bad("labels must be lowercase [a-z0-9-]");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "id": "app.outl.examples.todo-archiver",
        "name": "Todo Archiver",
        "version": "1.2.0",
        "api": "^1.0",
        "engines": { "outl": ">=0.7.0" },
        "main": "index.js",
        "capabilities": ["op-hook", "slash-command", "keybinding", "config-schema"],
        "permissions": ["read-page", "submit-op", "storage:local"],
        "contributes": {
            "commands": [{ "id": "todo-archive-done", "title": "Archive DONE blocks" }],
            "keybindings": [{ "command": "todo-archive-done", "key": "Ctrl+T A", "when": "tui" }],
            "configSchema": "config.schema.json"
        },
        "author": { "name": "Avelino", "url": "https://avelino.run" },
        "license": "MIT",
        "category": "productivity"
    }"#;

    #[test]
    fn parses_a_valid_manifest() {
        let m = PluginManifest::parse(VALID.as_bytes()).unwrap();
        assert_eq!(m.id, "app.outl.examples.todo-archiver");
        assert_eq!(m.version, Version::new(1, 2, 0));
        assert_eq!(m.capabilities.len(), 4);
        assert_eq!(
            m.contributes.config_schema.as_deref(),
            Some("config.schema.json")
        );
        assert_eq!(m.meta.author.unwrap().display_name(), "Avelino");
    }

    #[test]
    fn author_accepts_npm_string_form() {
        let json = VALID.replace(
            r#"{ "name": "Avelino", "url": "https://avelino.run" }"#,
            r#""Avelino <avelinorun@gmail.com>""#,
        );
        let m = PluginManifest::parse(json.as_bytes()).unwrap();
        assert_eq!(m.meta.author.unwrap().display_name(), "Avelino");
    }

    /// The shipped example plugin's manifest must always pass the validator —
    /// it is the canonical template authors copy.
    #[test]
    fn example_plugin_manifest_is_valid() {
        const EXAMPLE: &str = include_str!("../../../examples/todo-archiver/plugin.json");
        let m = PluginManifest::parse(EXAMPLE.as_bytes()).unwrap();
        assert_eq!(m.id, "app.outl.examples.todo-archiver");
        assert!(m.is_api_compatible(&semver::Version::new(1, 0, 0)));
    }

    #[test]
    fn api_compat_is_range_based() {
        let m = PluginManifest::parse(VALID.as_bytes()).unwrap();
        assert!(m.is_api_compatible(&Version::new(1, 4, 0)));
        assert!(!m.is_api_compatible(&Version::new(2, 0, 0)));
    }

    #[test]
    fn rejects_non_reverse_dns_id() {
        let json = VALID.replace("app.outl.examples.todo-archiver", "todoarchiver");
        assert!(PluginManifest::parse(json.as_bytes()).is_err());
    }

    #[test]
    fn rejects_uppercase_id() {
        let json = VALID.replace("app.outl.examples.todo-archiver", "run.Avelino.Todo");
        assert!(PluginManifest::parse(json.as_bytes()).is_err());
    }

    #[test]
    fn rejects_empty_main() {
        let json = VALID.replace("\"index.js\"", "\"\"");
        assert!(PluginManifest::parse(json.as_bytes()).is_err());
    }

    #[test]
    fn rejects_main_path_traversal() {
        // `main` is joined onto the plugin dir on install + every load, so a
        // traversal / absolute path must be rejected (it would read/write
        // outside `.outl/plugins/<id>/`).
        for evil in [
            "../../etc/passwd",
            "../index.js",
            "a/../../b.js",
            "/etc/passwd",
            "\\\\server\\share",
            "..\\\\windows\\\\x",
            "~/secret",
            "C:/x.js",
        ] {
            let json = VALID.replace("\"index.js\"", &format!("\"{evil}\""));
            assert!(
                PluginManifest::parse(json.as_bytes()).is_err(),
                "main `{evil}` must be rejected"
            );
        }
    }

    #[test]
    fn rejects_config_schema_traversal() {
        let json = VALID.replace(
            "\"configSchema\": \"config.schema.json\"",
            "\"configSchema\": \"../../steal.json\"",
        );
        // The VALID fixture may not declare configSchema; build one that does.
        let with_schema = json.replace(
            "\"contributes\": {",
            "\"contributes\": {\n    \"configSchema\": \"../../steal.json\",",
        );
        assert!(PluginManifest::parse(with_schema.as_bytes()).is_err());
    }

    #[test]
    fn accepts_subdir_main() {
        // A relative path into a subdirectory is fine — only escapes are rejected.
        let json = VALID.replace("\"index.js\"", "\"dist/index.js\"");
        assert!(PluginManifest::parse(json.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_keybinding_to_unknown_command() {
        let json = VALID.replace(
            "\"command\": \"todo-archive-done\"",
            "\"command\": \"ghost\"",
        );
        assert!(PluginManifest::parse(json.as_bytes()).is_err());
    }

    #[test]
    fn rejects_network_star_permission_via_serde() {
        let json = VALID.replace("\"storage:local\"", "\"network:*\"");
        assert!(PluginManifest::parse(json.as_bytes()).is_err());
    }

    #[test]
    fn missing_optional_sections_default() {
        let minimal = r#"{
            "id": "app.outl.examples.min",
            "name": "Min",
            "version": "0.1.0",
            "api": "^1.0",
            "main": "index.js"
        }"#;
        let m = PluginManifest::parse(minimal.as_bytes()).unwrap();
        assert!(m.capabilities.is_empty());
        assert!(m.permissions.is_empty());
        assert!(m.contributes.commands.is_empty());
        assert_eq!(m.engines.outl, VersionReq::STAR);
    }
}
