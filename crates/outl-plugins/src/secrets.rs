//! Per-plugin secrets, backed by the OS keychain.
//!
//! Plugin **config** (`ctx.config`) lives in the lockfile in plaintext — fine
//! for a page prefix or a day count, wrong for an API token. **Secrets** are
//! the sensitive half: they never touch the workspace on disk, they live in the
//! OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret
//! Service), and a plugin reads its own with `ctx.secrets.get(key)` — gated by
//! the `secrets` permission.
//!
//! Isolation is by keychain **service**: each plugin's secrets sit under
//! `outl-plugin:<id>` ([`plugin_service`]), so one plugin can never address
//! another's. The host only ever builds the service from the *calling* plugin's
//! id, so a plugin can't spoof a different namespace.
//!
//! The store is a trait so the engine and CLI run against the real keychain in
//! production and an in-memory map in tests — no keychain access, no OS prompts
//! in CI.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{PluginError, Result};

/// The keychain service namespace for a plugin's secrets. Namespaced by plugin
/// id so secrets can never leak across plugins, and prefixed so outl's entries
/// are recognizable in the OS keychain UI.
pub fn plugin_service(plugin_id: &str) -> String {
    format!("outl-plugin:{plugin_id}")
}

/// A backing store for plugin secrets. Abstracted so the same code paths run
/// against the OS keychain ([`KeyringStore`]) in production and an in-memory
/// map ([`MemorySecretStore`]) in tests.
///
/// `service` is always [`plugin_service`] of the owning plugin; `key` is the
/// secret name the plugin and the user agree on (e.g. `token`).
pub trait SecretStore: Send + Sync {
    /// Read a secret, or `None` when it was never set.
    fn get(&self, service: &str, key: &str) -> Result<Option<String>>;
    /// Store (or overwrite) a secret.
    fn set(&self, service: &str, key: &str, value: &str) -> Result<()>;
    /// Delete a secret. Deleting a missing key is not an error (idempotent).
    fn delete(&self, service: &str, key: &str) -> Result<()>;
}

/// The production store: the OS keychain via the `keyring` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    /// A new keychain-backed store.
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, service: &str, key: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(service, key).map_err(secret_err)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            // Absent is a normal outcome, not a failure.
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_err(e)),
        }
    }

    fn set(&self, service: &str, key: &str, value: &str) -> Result<()> {
        let entry = keyring::Entry::new(service, key).map_err(secret_err)?;
        entry.set_password(value).map_err(secret_err)
    }

    fn delete(&self, service: &str, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(service, key).map_err(secret_err)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // Deleting what isn't there is a no-op success (idempotent).
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_err(e)),
        }
    }
}

fn secret_err(e: keyring::Error) -> PluginError {
    PluginError::Secret(e.to_string())
}

/// An in-memory [`SecretStore`] for tests and headless runs — never touches the
/// OS keychain. Keyed by `(service, key)`.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    map: Mutex<HashMap<(String, String), String>>,
}

impl MemorySecretStore {
    /// An empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, service: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(&(service.to_string(), key.to_string()))
            .cloned())
    }

    fn set(&self, service: &str, key: &str, value: &str) -> Result<()> {
        self.map
            .lock()
            .unwrap()
            .insert((service.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete(&self, service: &str, key: &str) -> Result<()> {
        self.map
            .lock()
            .unwrap()
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_is_namespaced_per_plugin() {
        assert_eq!(
            plugin_service("run.avelino.ouraring"),
            "outl-plugin:run.avelino.ouraring"
        );
        // Different plugins never collide on the same service string.
        assert_ne!(plugin_service("a.b.c"), plugin_service("a.b.d"));
    }

    #[test]
    fn memory_store_roundtrips() {
        let s = MemorySecretStore::new();
        let svc = plugin_service("run.avelino.ouraring");
        assert_eq!(s.get(&svc, "token").unwrap(), None);
        s.set(&svc, "token", "abc123").unwrap();
        assert_eq!(s.get(&svc, "token").unwrap(), Some("abc123".into()));
        s.set(&svc, "token", "def456").unwrap();
        assert_eq!(s.get(&svc, "token").unwrap(), Some("def456".into()));
        s.delete(&svc, "token").unwrap();
        assert_eq!(s.get(&svc, "token").unwrap(), None);
        // Deleting a missing key is a no-op, not an error.
        assert!(s.delete(&svc, "token").is_ok());
    }

    #[test]
    fn memory_store_isolates_by_service() {
        // A secret set for one plugin is invisible under another's service —
        // the isolation the permission model promises.
        let s = MemorySecretStore::new();
        let a = plugin_service("plugin.a");
        let b = plugin_service("plugin.b");
        s.set(&a, "token", "secret-a").unwrap();
        assert_eq!(s.get(&b, "token").unwrap(), None);
        assert_eq!(s.get(&a, "token").unwrap(), Some("secret-a".into()));
    }
}
