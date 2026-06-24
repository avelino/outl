//! Device identity — ed25519 keypair stored at `~/.outl/identity.key`.

use anyhow::{Context, Result};
use std::path::Path;

/// Device identity backed by an ed25519 keypair.
///
/// The public key (node id) is the device's permanent P2P address.
/// Never synced — one per device, not per workspace.
pub struct IrohIdentity {
    secret_key: iroh::SecretKey,
}

impl IrohIdentity {
    /// Load an existing identity from `path`, or generate and persist a fresh one.
    pub fn load_or_generate(path: &Path) -> Result<Self> {
        if path.exists() {
            let bytes = std::fs::read(path)
                .with_context(|| format!("read identity key from {}", path.display()))?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("identity.key must be exactly 32 bytes"))?;
            let secret_key = iroh::SecretKey::from_bytes(&arr);
            Ok(Self { secret_key })
        } else {
            let secret_key = iroh::SecretKey::generate();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dir {}", parent.display()))?;
            }
            std::fs::write(path, secret_key.to_bytes())
                .with_context(|| format!("write identity key to {}", path.display()))?;
            tracing::info!(
                node_id = %secret_key.public().fmt_short(),
                "generated new iroh identity"
            );
            Ok(Self { secret_key })
        }
    }

    /// The secret key for building an iroh `Endpoint`.
    pub fn secret_key(&self) -> &iroh::SecretKey {
        &self.secret_key
    }

    /// The public node id (device address).
    pub fn node_id(&self) -> iroh::EndpointId {
        self.secret_key.public()
    }
}
