//! The plugin registry — the day-zero "store".
//!
//! A single static JSON index served at [`DEFAULT_REGISTRY_URL`] listing
//! installable plugins (id, `github:` source, capabilities, permissions,
//! versions). Parsing and [`RegistryIndex::search`] are always available;
//! the network `fetch` is gated behind the `registry` feature so a build
//! without HTTP can still read a local index.
//!
//! Discovery never installs on its own: a hit gives the user a `github:`
//! source string they pass to `outl plugin install`, which is where the
//! manifest validation, permission prompt, and bundle-hash freeze happen.

use serde::Deserialize;

/// Canonical registry URL (served statically via Netlify, CORS-enabled so
/// the GUI webviews can fetch it too).
pub const DEFAULT_REGISTRY_URL: &str = "https://plugins.outl.app/registry.json";

/// The parsed registry index.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    /// Schema version (always `1` today).
    #[serde(default)]
    pub version: u32,
    /// ISO date of the last edit (informational).
    #[serde(default)]
    pub updated: Option<String>,
    /// Every listed plugin.
    #[serde(default)]
    pub plugins: Vec<RegistryEntry>,
}

/// One plugin entry in the index. Mirrors `registry-v1.json`; unknown
/// fields are ignored so the index can grow without breaking old clients.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryEntry {
    /// Reverse-DNS id, identical to the plugin's `plugin.json` `id`.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// One-line summary.
    #[serde(default)]
    pub description: String,
    /// Author handle/name.
    #[serde(default)]
    pub author: Option<String>,
    /// Install source — `github:owner/repo[/subdir]`, fed to
    /// `outl plugin install`.
    pub repository: String,
    /// Coarse category (`productivity`, `fun`, …).
    #[serde(default)]
    pub category: Option<String>,
    /// Free-form search keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Declared capabilities, mirrored from the manifest.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Declared permissions, mirrored from the manifest (so the user sees
    /// the ask before install).
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Newest published tag (without a leading `v`).
    #[serde(default)]
    pub latest: Option<String>,
    /// All published tags, newest last.
    #[serde(default)]
    pub versions: Vec<String>,
}

impl RegistryIndex {
    /// Parse an index from raw JSON bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, RegistryError> {
        serde_json::from_slice(bytes).map_err(|e| RegistryError::Parse(e.to_string()))
    }

    /// Case-insensitive substring search over each entry's id, name,
    /// description, category, and keywords. An empty query returns every
    /// entry (the "browse all" case). Results keep index order.
    pub fn search(&self, query: &str) -> Vec<&RegistryEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.plugins.iter().collect();
        }
        self.plugins.iter().filter(|e| e.matches(&q)).collect()
    }
}

impl RegistryEntry {
    /// Does this entry match an already-lowercased query?
    fn matches(&self, q: &str) -> bool {
        self.id.to_lowercase().contains(q)
            || self.name.to_lowercase().contains(q)
            || self.description.to_lowercase().contains(q)
            || self
                .category
                .as_deref()
                .is_some_and(|c| c.to_lowercase().contains(q))
            || self.keywords.iter().any(|k| k.to_lowercase().contains(q))
    }
}

/// Failure reading the registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The index JSON didn't parse.
    #[error("registry parse error: {0}")]
    Parse(String),
    /// The network fetch failed (only on the `registry` feature).
    #[error("registry fetch error: {0}")]
    Fetch(String),
    /// Staging or installing the downloaded bundle failed.
    #[error("registry install error: {0}")]
    Install(String),
}

/// Default base URL the per-plugin bundles are served under (the index URL
/// with `registry.json` stripped). `install_official` joins `/p/<id>/…`.
pub const DEFAULT_REGISTRY_BASE: &str = "https://plugins.outl.app";

/// Fetch and parse the registry index over HTTP.
///
/// Blocking (a short, one-shot request), with a hard timeout. Pass
/// [`DEFAULT_REGISTRY_URL`] for the official index.
#[cfg(feature = "registry")]
pub fn fetch(url: &str) -> Result<RegistryIndex, RegistryError> {
    let client = http_client()?;
    let bytes = get_bytes(&client, url)?;
    RegistryIndex::parse(&bytes)
}

/// Install an **official** plugin straight from the registry's bundle host
/// (`<base>/p/<id>/plugin.json` + the bundle). Downloads into a temp dir,
/// then hands off to [`crate::loader::install_from_dir`], so the manifest
/// validation, bundle-hash freeze, permission recording, and lockfile write
/// are the exact same path a local install takes.
///
/// `base` is the bundle host (e.g. [`DEFAULT_REGISTRY_BASE`]). The approved
/// permission set is the manifest's **declared** permissions — installing
/// an official (registry-listed, human-reviewed) plugin via "tap to install"
/// grants what it asks for, which the UI shows the user before the tap. The
/// `source` recorded in the lockfile is `registry:<id>`.
#[cfg(feature = "registry")]
pub fn install_official(
    plugins_dir: &std::path::Path,
    base: &str,
    id: &str,
    installed_by: Option<String>,
) -> Result<crate::manifest::PluginManifest, RegistryError> {
    let prefix = format!("{}/p/{id}", base.trim_end_matches('/'));
    let client = http_client()?;

    // Manifest first — it names the bundle file and any config schema.
    let manifest_bytes = get_bytes(&client, &format!("{prefix}/plugin.json"))?;
    let manifest = crate::manifest::PluginManifest::parse(&manifest_bytes)
        .map_err(|e| RegistryError::Parse(e.to_string()))?;
    let bundle = get_bytes(&client, &format!("{prefix}/{}", manifest.main))?;

    // Stage the downloaded shape in a temp dir, then reuse the local
    // installer (one code path for hashing + lockfile + copy).
    let tmp = tempfile::tempdir().map_err(|e| RegistryError::Install(e.to_string()))?;
    write_file(tmp.path().join("plugin.json"), &manifest_bytes)?;
    write_file(tmp.path().join(&manifest.main), &bundle)?;
    if let Some(schema) = &manifest.contributes.config_schema {
        // Best-effort: a missing schema file shouldn't block the install.
        if let Ok(bytes) = get_bytes(&client, &format!("{prefix}/{schema}")) {
            write_file(tmp.path().join(schema), &bytes)?;
        }
    }

    let source_ref = format!("registry:{id}");
    crate::loader::install_from_dir(
        plugins_dir,
        tmp.path(),
        &source_ref,
        manifest.permissions.clone(),
        installed_by,
    )
    .map_err(|e| RegistryError::Install(e.to_string()))
}

#[cfg(feature = "registry")]
fn http_client() -> Result<reqwest::blocking::Client, RegistryError> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("outl/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| RegistryError::Fetch(e.to_string()))
}

#[cfg(feature = "registry")]
fn get_bytes(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, RegistryError> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| RegistryError::Fetch(e.to_string()))?
        .error_for_status()
        .map_err(|e| RegistryError::Fetch(e.to_string()))?;
    Ok(resp
        .bytes()
        .map_err(|e| RegistryError::Fetch(e.to_string()))?
        .to_vec())
}

#[cfg(feature = "registry")]
fn write_file(path: std::path::PathBuf, bytes: &[u8]) -> Result<(), RegistryError> {
    std::fs::write(&path, bytes)
        .map_err(|e| RegistryError::Install(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1,
        "plugins": [
            { "id": "app.outl.examples.todo-archiver", "name": "TODO Archiver",
              "description": "Archive DONE blocks.", "repository": "github:avelino/outl/examples/todo-archiver",
              "category": "productivity", "keywords": ["todo", "archive"],
              "capabilities": ["op-hook"], "permissions": ["read-page"], "latest": "1.0.0" },
            { "id": "app.outl.examples.confetti", "name": "Confetti on Done",
              "description": "Throws confetti.", "repository": "github:avelino/outl/examples/confetti",
              "category": "fun", "keywords": ["fun"], "capabilities": ["ui-render"], "permissions": [] }
        ]
    }"#;

    #[test]
    fn parses_and_lists() {
        let idx = RegistryIndex::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(idx.plugins.len(), 2);
        assert_eq!(
            idx.plugins[0].repository,
            "github:avelino/outl/examples/todo-archiver"
        );
    }

    #[test]
    fn empty_query_returns_all() {
        let idx = RegistryIndex::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(idx.search("").len(), 2);
    }

    #[test]
    fn matches_name_id_description_category_keywords() {
        let idx = RegistryIndex::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(idx.search("archive").len(), 1); // keyword + description
        assert_eq!(idx.search("confetti").len(), 1); // id + name
        assert_eq!(idx.search("FUN").len(), 1); // category, case-insensitive
        assert_eq!(idx.search("todo").len(), 1); // keyword
        assert!(idx.search("nonsense").is_empty());
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_optionals() {
        let json = r#"{ "version": 1, "extra": "ignored", "plugins": [
            { "id": "x.y", "name": "Y", "repository": "github:a/b", "future_field": 42 }
        ] }"#;
        let idx = RegistryIndex::parse(json.as_bytes()).unwrap();
        assert_eq!(idx.plugins[0].id, "x.y");
        assert!(idx.plugins[0].permissions.is_empty());
        assert_eq!(idx.plugins[0].latest, None);
    }
}
