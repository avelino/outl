//! Error types for the plugin system.

use thiserror::Error;

/// Anything that can go wrong loading, validating, or running a plugin.
#[derive(Debug, Error)]
pub enum PluginError {
    /// The `plugin.json` manifest is malformed or fails validation.
    #[error("invalid manifest: {0}")]
    Manifest(String),

    /// An unknown or malformed capability string.
    #[error("invalid capability: {0}")]
    Capability(String),

    /// An unknown or malformed permission string.
    #[error("invalid permission: {0}")]
    Permission(String),

    /// The installed bundle hash does not match the lockfile.
    #[error("bundle integrity check failed for {id}: expected {expected}, got {actual}")]
    BundleHashMismatch {
        /// Plugin id whose bundle failed verification.
        id: String,
        /// Hash recorded in the lockfile.
        expected: String,
        /// Hash computed from the bundle on disk.
        actual: String,
    },

    /// A host call was made without the required permission approved.
    #[error("permission denied: plugin {id} needs `{needed}`")]
    PermissionDenied {
        /// Plugin id that attempted the call.
        id: String,
        /// Permission string the call required.
        needed: String,
    },

    /// The plugin's required plugin-API range is incompatible with the host.
    #[error("api incompatible: plugin requires `{required}`, host implements {host}")]
    ApiIncompatible {
        /// `api` range declared in the manifest.
        required: String,
        /// Plugin-API version the host implements.
        host: String,
    },

    /// The JS engine failed to load or run plugin code.
    #[error("engine: {0}")]
    Engine(String),

    /// The OS keychain (secret store) refused a read/write.
    #[error("secret store: {0}")]
    Secret(String),

    /// No engine is available (built without the `js` feature).
    #[error("plugin engine unavailable: built without the `js` feature")]
    NoEngine,

    /// A plugin referenced a block id that is not a valid node id.
    #[error("invalid node id `{0}`")]
    BadNodeId(String),

    /// I/O error reading or writing plugin files / the lockfile.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// (De)serialization error on the lockfile or a JSON manifest.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Convenience alias for plugin operations.
pub type Result<T> = std::result::Result<T, PluginError>;
