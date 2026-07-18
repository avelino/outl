//! iroh-based P2P sync transport for outl.
//!
//! The main entry point is [`IrohSyncTransport`], which implements
//! [`outl_actions::SyncTransport`] using iroh QUIC + iroh-gossip.
//!
//! ## Quick start
//!
//! ```ignore
//! use outl_sync_iroh::{IrohSyncTransport, IrohIdentity, PeersStore};
//! use outl_actions::SyncEngine;
//! use std::sync::mpsc;
//!
//! // Identity is per-DEVICE (one node id per machine) → global `~/.outl/`.
//! let identity = IrohIdentity::load_or_generate(
//!     &dirs::home_dir().unwrap().join(".outl/identity.key")
//! ).unwrap();
//! // The peer list is per-GRAPH → `<workspace>/.outl/peers.json`.
//! outl_sync_iroh::migrate_global_peers_if_absent(&workspace_root);
//! let peers = PeersStore::load_or_default(
//!     &outl_sync_iroh::workspace_peers_path(&workspace_root)
//! ).unwrap();
//! // `relay_url`: `None` uses outl's default relay (`use1-1.relay.avelino.outl.iroh.link`); `Some(url)`
//! // (from `[sync] relay_url` in the user config) points the endpoint at a custom one.
//! let transport = IrohSyncTransport::new(identity, peers, None);
//! let engine = SyncEngine::with_transport(workspace_root, actor, Box::new(transport));
//! let (tx, rx) = mpsc::channel();
//! engine.start_transport(tx);
//! // Now rx fires whenever peer ops arrive and the workspace is ready to reload.
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bind;
mod engine;
mod engine_catchup;
mod engine_gossip;
mod engine_membership;
mod engine_pairing;
mod engine_snapshot;
mod engine_sync;
mod health;
mod identity;
mod pairing;
mod peers;
mod peers_lock;
mod progress;
mod protocol;
mod status;

#[doc(hidden)]
pub mod test_support;

pub use engine::IrohSyncTransport;
pub use identity::IrohIdentity;
pub use pairing::{host_pairing, join_pairing};
pub use peers::{migrate_global_peers_if_absent, workspace_peers_path, PeerEntry, PeersStore};
pub use protocol::{PAIRING_ALPN, SNAPSHOT_ALPN, SYNC_ALPN};
pub use status::{probe_peers, probe_peers_blocking, PeerStatus};
