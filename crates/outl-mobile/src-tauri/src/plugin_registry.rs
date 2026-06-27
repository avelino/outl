//! Marketplace backend: fetch the registry + install / manage official
//! plugins. Pure HTTP + lockfile + filesystem work — no `PluginHost`, so it
//! lives outside `plugin_service.rs` (which keeps the Boa thread). The plugin
//! thread calls these, then flips its `loaded` flag so the host reloads.

use std::path::Path;

use outl_core::hlc::HlcGenerator;
use serde::Serialize;

/// One marketplace row: a registry entry plus this workspace's local state
/// (installed / enabled). Mirrors the desktop DTO.
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
    pub installed: bool,
    pub enabled: bool,
}

/// Registry marketplace rows: fetch the official index, then mark each entry
/// installed/enabled from the lockfile.
pub(crate) fn registry_list(storage_root: &Path) -> Result<Vec<RegistryItemDto>, String> {
    let index = outl_plugins::registry::fetch(outl_plugins::DEFAULT_REGISTRY_URL)
        .map_err(|e| e.to_string())?;
    let pdir = outl_plugins::plugins_dir(storage_root);
    let lock = outl_plugins::InstalledPlugins::load(&outl_plugins::lockfile_path(&pdir))
        .unwrap_or_default();
    Ok(index
        .plugins
        .into_iter()
        .map(|e| {
            let entry = lock.plugins.get(&e.id);
            RegistryItemDto {
                installed: entry.is_some(),
                enabled: entry.map(|x| x.enabled).unwrap_or(false),
                id: e.id,
                name: e.name,
                description: e.description,
                author: e.author,
                category: e.category,
                capabilities: e.capabilities,
                permissions: e.permissions,
                latest: e.latest,
            }
        })
        .collect())
}

/// Download + install an official plugin; returns its display name.
pub(crate) fn install_official(
    storage_root: &Path,
    hlc: &HlcGenerator,
    id: &str,
) -> Result<String, String> {
    let pdir = outl_plugins::plugins_dir(storage_root);
    std::fs::create_dir_all(&pdir).map_err(|e| e.to_string())?;
    let manifest = outl_plugins::registry::install_official(
        &pdir,
        outl_plugins::DEFAULT_REGISTRY_BASE,
        id,
        Some(hlc.actor().to_string()),
    )
    .map_err(|e| e.to_string())?;
    Ok(manifest.name)
}

/// Flip `enabled` in the lockfile.
pub(crate) fn set_enabled(storage_root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    let lock_path = outl_plugins::lockfile_path(&outl_plugins::plugins_dir(storage_root));
    let mut lock = outl_plugins::InstalledPlugins::load(&lock_path).map_err(|e| e.to_string())?;
    let entry = lock
        .plugins
        .get_mut(id)
        .ok_or_else(|| format!("`{id}` is not installed"))?;
    entry.enabled = enabled;
    lock.save(&lock_path).map_err(|e| e.to_string())
}

/// Delete a plugin's directory + lockfile entry.
pub(crate) fn uninstall_plugin(storage_root: &Path, id: &str) -> Result<bool, String> {
    outl_plugins::uninstall(&outl_plugins::plugins_dir(storage_root), id).map_err(|e| e.to_string())
}
