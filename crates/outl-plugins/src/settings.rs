//! Plugin settings — the one config surface every client renders.
//!
//! A plugin's `config.schema.json` (JSON Schema draft-07) describes its
//! user-editable fields. This module turns that schema plus the current values
//! into a flat [`SettingsField`] list a client can render as a form, and
//! applies edits back. It is deliberately client-agnostic: the TUI, desktop,
//! mobile, and CLI all call [`describe`] to draw the form and [`set_config`] /
//! the secret helpers to persist a change, so "configure a plugin" behaves the
//! same everywhere.
//!
//! Two backings, one form:
//! - **config** fields live in the lockfile (`installed.json`) in plaintext —
//!   right for a page prefix or a day count.
//! - **secret** fields (a property flagged `"x-outl-secret": true`) live in the
//!   OS keychain via [`crate::secrets`] — right for an API token. The form
//!   never carries a secret's value, only whether it is set.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{PluginError, Result};
use crate::loader::{lockfile_path, plugins_dir};
use crate::lockfile::InstalledPlugins;
use crate::secrets::{plugin_service, SecretStore};

/// The JSON Schema extension key that flags a property as a secret (keychain-
/// backed) rather than plaintext lockfile config.
pub const SECRET_MARKER: &str = "x-outl-secret";

/// The value type of a settings field, from the schema `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    /// A free-text string.
    String,
    /// A whole number.
    Integer,
    /// A real number.
    Number,
    /// A true/false toggle.
    Boolean,
    /// A type the form renders as raw JSON (arrays/objects/unknown).
    Json,
}

impl FieldKind {
    fn from_schema(ty: Option<&str>) -> Self {
        match ty {
            Some("string") => Self::String,
            Some("integer") => Self::Integer,
            Some("number") => Self::Number,
            Some("boolean") => Self::Boolean,
            _ => Self::Json,
        }
    }
}

/// One configurable field, ready for a client to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsField {
    /// Property key (what `ctx.config.get()[key]` / `ctx.secrets.get(key)` reads).
    pub key: String,
    /// Human label (schema `title`, falling back to the key).
    pub title: String,
    /// Help text (schema `description`), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The value type.
    pub kind: FieldKind,
    /// Whether this field is keychain-backed (flagged `x-outl-secret`).
    pub secret: bool,
    /// Schema default, when declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Current config value (config fields only; `None` when unset). A secret's
    /// value is never included — see `is_set`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// For secret fields: whether a value is stored in the keychain. Always
    /// `false` for config fields.
    pub is_set: bool,
}

/// Read the installed plugin's config schema, or `None` when it declares no
/// `contributes.configSchema`. The path is validated at install, so this only
/// reads a file already vetted to sit inside the plugin directory.
fn read_schema(pdir: &Path, id: &str) -> Result<Option<Value>> {
    let manifest_path = pdir.join(id).join("plugin.json");
    let bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PluginError::Manifest(format!(
                "plugin `{id}` is not installed"
            )))
        }
        Err(e) => return Err(PluginError::Io(e)),
    };
    let manifest = crate::manifest::PluginManifest::parse(&bytes)?;
    let Some(rel) = manifest.contributes.config_schema else {
        return Ok(None);
    };
    let schema_path = pdir.join(id).join(rel);
    let schema_bytes = std::fs::read(&schema_path)?;
    let schema: Value = serde_json::from_slice(&schema_bytes)?;
    Ok(Some(schema))
}

/// The current config object for a plugin (from the lockfile), or an empty
/// object when the plugin has no stored config yet.
fn read_config_object(pdir: &Path, id: &str) -> Result<Map<String, Value>> {
    let lock = InstalledPlugins::load(&lockfile_path(pdir))?;
    let entry = lock
        .get(id)
        .ok_or_else(|| PluginError::Manifest(format!("plugin `{id}` is not installed")))?;
    Ok(match &entry.config {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    })
}

/// Describe a plugin's settings as a flat field list a client can render.
///
/// Combines the config schema (types, titles, defaults, secret flags), the
/// current lockfile config values, and — for secret fields — whether a value is
/// present in `store`. Returns an empty list when the plugin declares no schema.
pub fn describe(
    workspace_root: &Path,
    id: &str,
    store: &dyn SecretStore,
) -> Result<Vec<SettingsField>> {
    let pdir = plugins_dir(workspace_root);
    let Some(schema) = read_schema(&pdir, id)? else {
        return Ok(Vec::new());
    };
    let config = read_config_object(&pdir, id)?;
    let service = plugin_service(id);

    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut fields = Vec::with_capacity(props.len());
    for (key, spec) in &props {
        let secret = spec
            .get(SECRET_MARKER)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let kind = FieldKind::from_schema(spec.get("type").and_then(Value::as_str));
        let title = spec
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let description = spec
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let default = spec.get("default").cloned();

        // A secret's value never leaves the keychain; report only presence.
        let is_set = if secret {
            store.get(&service, key)?.is_some()
        } else {
            false
        };
        let value = if secret {
            None
        } else {
            config.get(key).cloned()
        };

        fields.push(SettingsField {
            key: key.clone(),
            title,
            description,
            kind,
            secret,
            default,
            value,
            is_set,
        });
    }

    // Stable order for a deterministic form (schemas are unordered maps).
    fields.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(fields)
}

/// Set a plaintext config field in the lockfile. `value` is stored as-is; the
/// caller coerces it to the field's type first (see [`coerce`]). Rejects a key
/// flagged as a secret — those go through [`set_secret`], never the lockfile.
pub fn set_config(workspace_root: &Path, id: &str, key: &str, value: Value) -> Result<()> {
    let pdir = plugins_dir(workspace_root);
    if is_secret_key(&pdir, id, key)? {
        return Err(PluginError::Manifest(format!(
            "`{key}` is a secret — set it with `secret set`, it never goes in the lockfile"
        )));
    }
    let lock_path = lockfile_path(&pdir);
    let mut lock = InstalledPlugins::load(&lock_path)?;
    let entry = lock
        .plugins
        .get_mut(id)
        .ok_or_else(|| PluginError::Manifest(format!("plugin `{id}` is not installed")))?;
    let obj = match &mut entry.config {
        Value::Object(m) => m,
        slot => {
            *slot = Value::Object(Map::new());
            slot.as_object_mut().unwrap()
        }
    };
    obj.insert(key.to_string(), value);
    lock.save(&lock_path)?;
    Ok(())
}

/// Store a secret field's value in the keychain (namespaced to the plugin).
pub fn set_secret(id: &str, key: &str, value: &str, store: &dyn SecretStore) -> Result<()> {
    store.set(&plugin_service(id), key, value)
}

/// Delete a secret field's value from the keychain (idempotent).
pub fn delete_secret(id: &str, key: &str, store: &dyn SecretStore) -> Result<()> {
    store.delete(&plugin_service(id), key)
}

/// Whether `key` is declared as a secret in the plugin's schema.
fn is_secret_key(pdir: &Path, id: &str, key: &str) -> Result<bool> {
    let Some(schema) = read_schema(pdir, id)? else {
        return Ok(false);
    };
    Ok(schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|p| p.get(key))
        .and_then(|spec| spec.get(SECRET_MARKER))
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// Coerce a raw string (e.g. a CLI argument) to a JSON value matching `kind`.
/// Strings stay strings; numbers/booleans parse; JSON fields parse as JSON.
pub fn coerce(kind: FieldKind, raw: &str) -> Result<Value> {
    let bad = |what: &str| PluginError::Manifest(format!("`{raw}` is not a valid {what}"));
    match kind {
        FieldKind::String => Ok(Value::String(raw.to_string())),
        FieldKind::Boolean => match raw {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(bad("boolean (expected true/false)")),
        },
        FieldKind::Integer => raw
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|_| bad("integer")),
        FieldKind::Number => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| bad("number")),
        FieldKind::Json => serde_json::from_str(raw).map_err(|_| bad("JSON value")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecretStore;
    use std::fs;
    use tempfile::tempdir;

    /// Lay down an installed plugin (manifest + schema + lockfile) under a
    /// temp workspace root, returning the root.
    fn install_fixture(config: Value) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let id = "run.avelino.ouraring";
        let pdir = plugins_dir(root).join(id);
        fs::create_dir_all(&pdir).unwrap();
        fs::write(
            pdir.join("plugin.json"),
            br#"{
                "id": "run.avelino.ouraring",
                "name": "Oura",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command", "config-schema"],
                "permissions": ["secrets"],
                "contributes": { "configSchema": "config.schema.json" }
            }"#,
        )
        .unwrap();
        fs::write(
            pdir.join("config.schema.json"),
            br#"{
                "type": "object",
                "properties": {
                    "token": { "type": "string", "title": "Oura token", "x-outl-secret": true },
                    "pagePrefix": { "type": "string", "title": "Page prefix", "default": "ouraring" },
                    "daysToSync": { "type": "integer", "title": "Days", "default": 7 }
                }
            }"#,
        )
        .unwrap();

        let mut lock = InstalledPlugins::default();
        lock.plugins.insert(
            id.to_string(),
            crate::lockfile::InstalledEntry {
                version: "1.0.0".into(),
                source: "local".into(),
                bundle_hash: "sha256:0".into(),
                installed_at: None,
                installed_by: None,
                permissions_approved: vec![],
                enabled: true,
                config,
            },
        );
        lock.save(&lockfile_path(&plugins_dir(root))).unwrap();
        dir
    }

    #[test]
    fn describe_merges_schema_config_and_secret_status() {
        let dir = install_fixture(serde_json::json!({ "pagePrefix": "health" }));
        let root = dir.path();
        let store = MemorySecretStore::new();
        store
            .set(&plugin_service("run.avelino.ouraring"), "token", "pat-x")
            .unwrap();

        let form = describe(root, "run.avelino.ouraring", &store).unwrap();
        // Sorted by key: daysToSync, pagePrefix, token.
        assert_eq!(form.len(), 3);

        let token = form.iter().find(|f| f.key == "token").unwrap();
        assert!(token.secret);
        assert!(token.is_set, "keychain has the token");
        assert!(
            token.value.is_none(),
            "a secret's value never appears in the form"
        );

        let prefix = form.iter().find(|f| f.key == "pagePrefix").unwrap();
        assert!(!prefix.secret);
        assert_eq!(prefix.value, Some(Value::String("health".into())));
        assert_eq!(prefix.default, Some(Value::String("ouraring".into())));

        let days = form.iter().find(|f| f.key == "daysToSync").unwrap();
        assert_eq!(days.kind, FieldKind::Integer);
        assert_eq!(days.value, None, "unset config field has no value");
    }

    #[test]
    fn set_config_writes_lockfile_and_rejects_secret_keys() {
        let dir = install_fixture(serde_json::json!({}));
        let root = dir.path();

        set_config(root, "run.avelino.ouraring", "daysToSync", Value::from(14)).unwrap();
        let store = MemorySecretStore::new();
        let days = describe(root, "run.avelino.ouraring", &store)
            .unwrap()
            .into_iter()
            .find(|f| f.key == "daysToSync")
            .unwrap();
        assert_eq!(days.value, Some(Value::from(14)));

        // A secret key must never be routed into the plaintext lockfile.
        let err = set_config(
            root,
            "run.avelino.ouraring",
            "token",
            Value::String("leak".into()),
        );
        assert!(err.is_err(), "setting a secret via config must be rejected");
    }

    #[test]
    fn coerce_respects_field_type() {
        assert_eq!(coerce(FieldKind::Integer, "14").unwrap(), Value::from(14));
        assert_eq!(
            coerce(FieldKind::Boolean, "true").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            coerce(FieldKind::String, "14").unwrap(),
            Value::String("14".into())
        );
        assert!(coerce(FieldKind::Integer, "not-a-number").is_err());
        assert!(coerce(FieldKind::Boolean, "yes").is_err());
    }

    #[test]
    fn set_secret_roundtrips_through_store() {
        let store = MemorySecretStore::new();
        set_secret("run.avelino.ouraring", "token", "abc", &store).unwrap();
        assert_eq!(
            store
                .get(&plugin_service("run.avelino.ouraring"), "token")
                .unwrap(),
            Some("abc".into())
        );
        delete_secret("run.avelino.ouraring", "token", &store).unwrap();
        assert_eq!(
            store
                .get(&plugin_service("run.avelino.ouraring"), "token")
                .unwrap(),
            None
        );
    }
}
