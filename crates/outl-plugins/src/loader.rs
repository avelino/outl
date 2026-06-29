//! Disk loader + installer.
//!
//! Reads a workspace's installed plugins off disk and into a [`PluginHost`],
//! and installs a plugin from a local directory (the `github:` source resolves
//! to a clone the CLI hands us as a local path, so this stays transport-free).
//!
//! Layout under a workspace root:
//!
//! ```text
//! <root>/.outl/plugins/
//! ├── installed.json            ← lockfile (versions, hashes, approved perms)
//! ├── <id>/                     ← one installed plugin
//! │   ├── plugin.json
//! │   └── index.js              ← bundled, hash-checked on load
//! └── _dev/<name>/              ← dev-mode plugins (no hash, perms relaxed)
//! ```

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{PluginError, Result};
use crate::host::PluginHost;
use crate::lockfile::{bundle_hash, InstalledEntry, InstalledPlugins};
use crate::manifest::PluginManifest;
use crate::permission::PermissionSet;

/// Per-plugin load outcome, so one broken plugin never blocks the others.
#[derive(Debug)]
pub struct LoadReport {
    /// Ids that loaded and activated.
    pub loaded: Vec<String>,
    /// `(id, error)` for plugins that failed to load.
    pub failed: Vec<(String, PluginError)>,
}

/// The `.outl/plugins` directory under a workspace root.
pub fn plugins_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".outl").join("plugins")
}

/// Path to the lockfile.
pub fn lockfile_path(plugins_dir: &Path) -> PathBuf {
    plugins_dir.join("installed.json")
}

/// Load every enabled installed plugin (plus any `_dev/` plugins) into `host`.
///
/// Best-effort: a plugin whose manifest is invalid, whose bundle hash no longer
/// matches the lockfile, or whose JS fails to load is recorded in
/// [`LoadReport::failed`] and skipped, never fatal.
pub fn load_installed(host: &mut PluginHost, plugins_dir: &Path) -> LoadReport {
    let mut report = LoadReport {
        loaded: Vec::new(),
        failed: Vec::new(),
    };
    if !plugins_dir.exists() {
        return report;
    }
    // So `ctx.storage` persists under <plugins_dir>/<id>/storage.json.
    host.set_storage_dir(plugins_dir.to_path_buf());

    let lock = InstalledPlugins::load(&lockfile_path(plugins_dir)).unwrap_or_default();
    for (id, entry) in &lock.plugins {
        if !entry.enabled {
            continue;
        }
        let dir = plugins_dir.join(id);
        match load_one(host, &dir, Some(entry)) {
            Ok(()) => report.loaded.push(id.clone()),
            Err(e) => report.failed.push((id.clone(), e)),
        }
    }

    load_dev(host, plugins_dir, &mut report);
    report
}

/// Load `_dev/*` plugins: no hash check, every requested permission implicitly
/// granted, never recorded in the lockfile.
fn load_dev(host: &mut PluginHost, plugins_dir: &Path, report: &mut LoadReport) {
    let dev = plugins_dir.join("_dev");
    let Ok(entries) = std::fs::read_dir(&dev) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let label = format!("_dev/{}", entry.file_name().to_string_lossy());
        match load_one(host, &dir, None) {
            Ok(()) => report.loaded.push(label),
            Err(e) => report.failed.push((label, e)),
        }
    }
}

/// Load a single plugin directory. When `entry` is `Some`, the bundle hash is
/// verified and the approved permission set comes from the lockfile; when
/// `None` (dev mode), the hash is skipped and every declared permission is
/// granted.
fn load_one(host: &mut PluginHost, dir: &Path, entry: Option<&InstalledEntry>) -> Result<()> {
    let manifest = PluginManifest::parse(&std::fs::read(dir.join("plugin.json"))?)?;
    let bundle = std::fs::read(dir.join(&manifest.main))?;

    let (perms, config) = match entry {
        Some(e) => {
            e.verify_bundle(&manifest.id, &bundle)?;
            (
                PermissionSet::new(e.permissions_approved.clone()),
                e.config.clone(),
            )
        }
        // Dev mode: grant exactly what the manifest asks for, no lockfile.
        None => (
            PermissionSet::new(manifest.permissions.clone()),
            Value::Null,
        ),
    };

    let source = String::from_utf8(bundle)
        .map_err(|_| PluginError::Manifest("bundle is not valid UTF-8".into()))?;
    host.load_plugin(manifest, &source, perms, config)
}

/// Install a plugin from a local directory (the installed-shape: a `plugin.json`
/// plus its bundle and assets). Copies it under `.outl/plugins/<id>/`, hashes
/// the bundle, and writes a lockfile entry approving `permissions`.
///
/// The caller is responsible for having shown the user `manifest.permissions`
/// and gotten approval — `permissions` is what they approved.
pub fn install_from_dir(
    plugins_dir: &Path,
    source_dir: &Path,
    source_ref: &str,
    permissions: Vec<crate::permission::Permission>,
    installed_by: Option<String>,
) -> Result<PluginManifest> {
    let manifest = PluginManifest::parse(&std::fs::read(source_dir.join("plugin.json"))?)?;
    let bundle = std::fs::read(source_dir.join(&manifest.main))?;
    let hash = bundle_hash(&bundle);

    // Copy the installed shape: manifest + bundle (+ config schema if present).
    let dest = plugins_dir.join(&manifest.id);
    std::fs::create_dir_all(&dest)?;
    std::fs::copy(source_dir.join("plugin.json"), dest.join("plugin.json"))?;
    std::fs::write(dest.join(&manifest.main), &bundle)?;
    if let Some(schema) = &manifest.contributes.config_schema {
        let from = source_dir.join(schema);
        if from.exists() {
            std::fs::copy(from, dest.join(schema))?;
        }
    }

    // Record it in the lockfile.
    let lock_path = lockfile_path(plugins_dir);
    let mut lock = InstalledPlugins::load(&lock_path)?;
    lock.plugins.insert(
        manifest.id.clone(),
        InstalledEntry {
            version: manifest.version.to_string(),
            source: source_ref.to_string(),
            bundle_hash: hash,
            installed_at: None,
            installed_by,
            permissions_approved: permissions,
            enabled: true,
            config: Value::Null,
        },
    );
    lock.save(&lock_path)?;
    Ok(manifest)
}

/// Uninstall a plugin: drop its lockfile entry and delete its installed
/// directory. Returns `true` if anything was removed (entry and/or directory),
/// `false` if the id wasn't installed.
///
/// The id is validated to be a plain reverse-DNS-shaped name (no path
/// separators, no `..`) before it is joined onto `plugins_dir`, so a crafted
/// id can never delete outside the plugins directory.
pub fn uninstall(plugins_dir: &Path, id: &str) -> Result<bool> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.split('.').any(|seg| seg.is_empty() || seg == "..")
    {
        return Err(PluginError::Manifest(format!("invalid plugin id `{id}`")));
    }

    let lock_path = lockfile_path(plugins_dir);
    let mut lock = InstalledPlugins::load(&lock_path)?;
    let had_entry = lock.plugins.remove(id).is_some();

    let dir = plugins_dir.join(id);
    let had_dir = dir.is_dir();
    if had_dir {
        std::fs::remove_dir_all(&dir)?;
    }
    if had_entry {
        lock.save(&lock_path)?;
    }
    Ok(had_entry || had_dir)
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::permission::Permission;

    const BUNDLE: &str = r#"
        globalThis.__outl_register({ activate(ctx) {
            ctx.commands.register('hello', () => ctx.ui.notify('hi'));
        }});
    "#;

    fn write_plugin(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            br#"{
                "id": "app.outl.examples.hello",
                "name": "Hello",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command"],
                "permissions": ["read-page"],
                "contributes": { "commands": [{ "id": "hello", "title": "Say hi" }] }
            }"#,
        )
        .unwrap();
        std::fs::write(dir.join("index.js"), BUNDLE).unwrap();
    }

    fn host() -> PluginHost {
        PluginHost::new([Capability::SlashCommand].into_iter().collect())
    }

    #[test]
    fn install_then_load_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        write_plugin(&src);
        let pdir = tmp.path().join(".outl/plugins");
        std::fs::create_dir_all(&pdir).unwrap();

        let manifest = install_from_dir(
            &pdir,
            &src,
            "local:src",
            vec![Permission::ReadPage],
            Some("device:test".into()),
        )
        .unwrap();
        assert_eq!(manifest.id, "app.outl.examples.hello");

        // The lockfile and the copied bundle now exist.
        assert!(pdir.join("installed.json").exists());
        assert!(pdir.join("app.outl.examples.hello/index.js").exists());

        let mut h = host();
        let report = load_installed(&mut h, &pdir);
        assert_eq!(report.loaded, vec!["app.outl.examples.hello"]);
        assert!(report.failed.is_empty());
        assert_eq!(h.commands().len(), 1);
    }

    #[test]
    fn uninstall_removes_dir_and_lockfile_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        write_plugin(&src);
        let pdir = tmp.path().join(".outl/plugins");
        std::fs::create_dir_all(&pdir).unwrap();
        install_from_dir(&pdir, &src, "local:src", vec![Permission::ReadPage], None).unwrap();

        let id = "app.outl.examples.hello";
        assert!(pdir.join(id).is_dir());

        let removed = uninstall(&pdir, id).unwrap();
        assert!(removed);
        assert!(!pdir.join(id).exists(), "plugin dir deleted");
        assert!(InstalledPlugins::load(&lockfile_path(&pdir))
            .unwrap()
            .get(id)
            .is_none());

        // Removing again reports nothing was there.
        assert!(!uninstall(&pdir, id).unwrap());
    }

    #[test]
    fn uninstall_rejects_path_traversal_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let pdir = tmp.path().join(".outl/plugins");
        std::fs::create_dir_all(&pdir).unwrap();
        assert!(uninstall(&pdir, "../../etc").is_err());
        assert!(uninstall(&pdir, "a/b").is_err());
        assert!(uninstall(&pdir, "..").is_err());
    }

    #[test]
    fn tampered_bundle_fails_to_load() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        write_plugin(&src);
        let pdir = tmp.path().join(".outl/plugins");
        std::fs::create_dir_all(&pdir).unwrap();
        install_from_dir(&pdir, &src, "local:src", vec![Permission::ReadPage], None).unwrap();

        // Tamper with the installed bundle out-of-band.
        std::fs::write(
            pdir.join("app.outl.examples.hello/index.js"),
            "globalThis.evil = 1;",
        )
        .unwrap();

        let mut h = host();
        let report = load_installed(&mut h, &pdir);
        assert!(report.loaded.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(matches!(
            report.failed[0].1,
            PluginError::BundleHashMismatch { .. }
        ));
    }

    #[test]
    fn dev_mode_loads_without_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        let pdir = tmp.path().join(".outl/plugins");
        write_plugin(&pdir.join("_dev/wip"));

        let mut h = host();
        let report = load_installed(&mut h, &pdir);
        assert_eq!(report.loaded, vec!["_dev/wip"]);
    }
}
