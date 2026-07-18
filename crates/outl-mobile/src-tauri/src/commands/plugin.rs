//! Plugin command surface — thin shims over the shared
//! [`PluginService`] (the Boa `Context` lives on a dedicated thread,
//! `!Send`, so these commands only talk to it through the request
//! channel) and the shared reply builders in
//! `outl_tauri_shared::commands::plugin`.
//!
//! Identical to the desktop's surface minus `plugin_keybindings`
//! (chords aren't a mobile surface, so the command is simply not
//! registered).

use tauri::State;

use crate::state::AppState;
use outl_plugins::{MarketplaceItem, SettingsField};
use outl_tauri_shared::commands::plugin::{self as shared, PluginRunReply, PluginSyncHooksReply};
use outl_tauri_shared::plugin_dto::{
    PluginCommandDto, ToolbarButtonDto, TransformResultDto, TransformerDto,
};
use outl_tauri_shared::PluginService;

/// List every command contributed by a loaded plugin whose
/// `slash-command` capability the mobile client honors. Empty until the
/// workspace opens and plugins load (best-effort).
#[tauri::command]
pub(crate) async fn plugin_list(
    plugins: State<'_, PluginService>,
) -> Result<Vec<PluginCommandDto>, String> {
    let plugins = plugins.inner().clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || plugins.list_commands())
            .await
            .unwrap_or_default(),
    )
}

/// List every toolbar button a loaded plugin contributes for the mobile
/// chrome (`host.toolbar_buttons("mobile")`). The header renders one glyph
/// per entry; tapping it fires `plugin_run(plugin_id, command_id)`. Empty
/// until the workspace opens and plugins load (best-effort).
#[tauri::command]
pub(crate) async fn plugin_toolbar(
    plugins: State<'_, PluginService>,
) -> Result<Vec<ToolbarButtonDto>, String> {
    let plugins = plugins.inner().clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || plugins.list_toolbar())
            .await
            .unwrap_or_default(),
    )
}

/// List every content transformer granted on the mobile client
/// (`host.transformers()`). The frontend loads this once when the workspace
/// opens and keys on `lang`: a code fence whose language matches fires
/// `plugin_transform(plugin_id, lang, body)`. Empty until the workspace
/// opens and plugins load (best-effort).
#[tauri::command]
pub(crate) async fn plugin_transformers(
    plugins: State<'_, PluginService>,
) -> Result<Vec<TransformerDto>, String> {
    let plugins = plugins.inner().clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || plugins.list_transformers())
            .await
            .unwrap_or_default(),
    )
}

/// Run a plugin's content transformer for `lang` against `input`, returning
/// the descriptor (`{kind, content}`) it produced — `None` when the
/// transformer declined. Pure render: no workspace mutation, no
/// re-projection, so (unlike `plugin_run`) it carries no refreshed page view.
#[tauri::command]
pub(crate) async fn plugin_transform(
    plugin_id: String,
    lang: String,
    input: String,
    plugins: State<'_, PluginService>,
) -> Result<Option<TransformResultDto>, String> {
    let plugins = plugins.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins.transform_block(plugin_id, lang, input))
        .await
        .map_err(|e| format!("plugin task join: {e}"))?
}

/// Run a plugin command. `page_id` is the page currently on screen; the
/// reply carries its refreshed page view so the frontend re-renders
/// without a second round-trip.
#[tauri::command]
pub(crate) fn plugin_run(
    plugin_id: String,
    command_id: String,
    page_id: Option<String>,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<PluginRunReply, String> {
    shared::run(
        state.inner(),
        plugins.inner(),
        plugin_id,
        command_id,
        page_id,
    )
}

/// Fire the plugins' `onOp` hook sweep after a user mutation — see the
/// shared body for the re-render / views contract (the confetti path).
#[tauri::command]
pub(crate) fn plugin_sync_hooks(
    page_id: Option<String>,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<PluginSyncHooksReply, String> {
    shared::sync_hooks(state.inner(), plugins.inner(), page_id)
}

/// Marketplace rows: the official registry (`plugins.outl.app`) crossed with
/// this workspace's lockfile (installed / enabled flags).
#[tauri::command]
pub(crate) async fn plugin_registry_list(
    plugins: State<'_, PluginService>,
) -> Result<Vec<MarketplaceItem>, String> {
    let plugins = plugins.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins.registry_list())
        .await
        .map_err(|e| format!("plugin task join: {e}"))?
}

/// Tap-to-install an official plugin by id; returns the installed name.
#[tauri::command]
pub(crate) async fn plugin_install_official(
    id: String,
    plugins: State<'_, PluginService>,
) -> Result<String, String> {
    let plugins = plugins.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins.install_official(id))
        .await
        .map_err(|e| format!("plugin task join: {e}"))?
}

/// Enable / disable an installed plugin, then reload.
#[tauri::command]
pub(crate) async fn plugin_set_enabled(
    id: String,
    enabled: bool,
    plugins: State<'_, PluginService>,
) -> Result<(), String> {
    let plugins = plugins.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins.set_enabled(id, enabled))
        .await
        .map_err(|e| format!("plugin task join: {e}"))?
}

/// Uninstall a plugin (delete its dir + lockfile entry), then reload.
#[tauri::command]
pub(crate) async fn plugin_uninstall(
    id: String,
    plugins: State<'_, PluginService>,
) -> Result<bool, String> {
    let plugins = plugins.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins.uninstall(id))
        .await
        .map_err(|e| format!("plugin task join: {e}"))?
}

/// Describe a plugin's settings form (config + secret fields).
#[tauri::command]
pub(crate) fn plugin_settings_describe(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SettingsField>, String> {
    shared::settings_describe(state.inner(), plugin_id)
}

/// Set a plaintext config field (coerced to its schema type), then reload.
#[tauri::command]
pub(crate) fn plugin_config_set(
    plugin_id: String,
    key: String,
    value: String,
    plugins: State<'_, PluginService>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    shared::config_set(state.inner(), plugins.inner(), plugin_id, key, value)
}

/// Store a secret field in the OS keychain.
#[tauri::command]
pub(crate) fn plugin_secret_set(
    plugin_id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    shared::secret_set(plugin_id, key, value)
}

/// Delete a secret field from the keychain.
#[tauri::command]
pub(crate) fn plugin_secret_remove(plugin_id: String, key: String) -> Result<(), String> {
    shared::secret_remove(plugin_id, key)
}
