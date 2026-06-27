//! `installed.json` — per-workspace lockfile.
//!
//! Records exactly which plugin version is installed, where it came from, and
//! what permissions the user approved. The bundle hash is revalidated on every
//! load so a plugin never changes silently (an out-of-band edit via Finder /
//! iCloud / a sync conflict is caught, not trusted).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{PluginError, Result};
use crate::permission::Permission;

/// Current lockfile schema version.
pub const LOCKFILE_VERSION: u32 = 1;

/// The whole `installed.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugins {
    /// Schema version for forward-compat.
    pub version: u32,
    /// Installed plugins keyed by reverse-DNS id.
    #[serde(default)]
    pub plugins: BTreeMap<String, InstalledEntry>,
}

impl Default for InstalledPlugins {
    fn default() -> Self {
        Self {
            version: LOCKFILE_VERSION,
            plugins: BTreeMap::new(),
        }
    }
}

/// One installed plugin's locked state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledEntry {
    /// Installed semver version.
    pub version: String,
    /// Immutable source ref, e.g. `github:avelino/outl-todo-archiver#v1.2.0`.
    pub source: String,
    /// `sha256:<hex>` of the bundle (`index.js`). Revalidated on load.
    #[serde(rename = "bundleHash")]
    pub bundle_hash: String,
    /// ISO-8601 install timestamp.
    #[serde(rename = "installedAt", default)]
    pub installed_at: Option<String>,
    /// Device id that performed the install (so a new device asks to confirm
    /// rather than auto-trusting synced workspace contents).
    #[serde(rename = "installedBy", default)]
    pub installed_by: Option<String>,
    /// Permissions frozen at install. An update asking for more requires
    /// re-approval before it loads.
    #[serde(rename = "permissionsApproved", default)]
    pub permissions_approved: Vec<Permission>,
    /// Whether the plugin is currently enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// User config, validated against the plugin's config schema. Lives outside
    /// the bundle so updates preserve it.
    #[serde(default)]
    pub config: serde_json::Value,
}

fn default_true() -> bool {
    true
}

/// Compute the canonical bundle hash (`sha256:<hex>`) of raw bundle bytes.
pub fn bundle_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

impl InstalledEntry {
    /// Verify the on-disk bundle matches the locked hash. Returns
    /// [`PluginError::BundleHashMismatch`] on divergence — the caller must
    /// refuse to load.
    pub fn verify_bundle(&self, id: &str, bundle: &[u8]) -> Result<()> {
        let actual = bundle_hash(bundle);
        if actual == self.bundle_hash {
            Ok(())
        } else {
            Err(PluginError::BundleHashMismatch {
                id: id.to_string(),
                expected: self.bundle_hash.clone(),
                actual,
            })
        }
    }
}

impl InstalledPlugins {
    /// Load the lockfile at `path`. A missing file is an empty lockfile, not an
    /// error — a workspace with no plugins yet is valid.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(PluginError::Io(e)),
        }
    }

    /// Write the lockfile to `path` (pretty-printed for diffability).
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Look up an installed entry by id.
    pub fn get(&self, id: &str) -> Option<&InstalledEntry> {
        self.plugins.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::Permission;

    fn sample_entry() -> InstalledEntry {
        InstalledEntry {
            version: "1.2.0".into(),
            source: "github:avelino/outl-todo-archiver#v1.2.0".into(),
            bundle_hash: bundle_hash(b"console.log('hi')"),
            installed_at: Some("2026-06-26T12:00:00Z".into()),
            installed_by: Some("device:01HZ".into()),
            permissions_approved: vec![Permission::ReadPage, Permission::SubmitOp],
            enabled: true,
            config: serde_json::json!({ "archivePage": "archive" }),
        }
    }

    #[test]
    fn bundle_hash_is_stable_and_prefixed() {
        let h = bundle_hash(b"abc");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h, bundle_hash(b"abc"));
        assert_ne!(h, bundle_hash(b"abd"));
    }

    #[test]
    fn verify_bundle_accepts_matching_and_rejects_tampered() {
        let entry = sample_entry();
        assert!(entry
            .verify_bundle("app.outl.examples.todo-archiver", b"console.log('hi')")
            .is_ok());
        let err = entry.verify_bundle("app.outl.examples.todo-archiver", b"console.log('EVIL')");
        assert!(matches!(err, Err(PluginError::BundleHashMismatch { .. })));
    }

    #[test]
    fn missing_lockfile_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let lock = InstalledPlugins::load(&path).unwrap();
        assert_eq!(lock.version, LOCKFILE_VERSION);
        assert!(lock.plugins.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut lock = InstalledPlugins::default();
        lock.plugins
            .insert("app.outl.examples.todo-archiver".into(), sample_entry());
        lock.save(&path).unwrap();

        let back = InstalledPlugins::load(&path).unwrap();
        let entry = back.get("app.outl.examples.todo-archiver").unwrap();
        assert_eq!(entry.version, "1.2.0");
        assert_eq!(
            entry.permissions_approved,
            vec![Permission::ReadPage, Permission::SubmitOp]
        );
        assert!(entry.enabled);
    }
}
