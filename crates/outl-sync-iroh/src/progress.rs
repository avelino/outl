//! Sync-progress reporting sink.
//!
//! A cheap, cloneable handle the engine's initiator-side sync paths (boot
//! connect, catch-up, gossip-triggered, immediate post-pair) use to push
//! [`SyncProgress`] to the UI bridge. Wrapping an `Option<Sender>` keeps every
//! emit a no-op when no sink is registered (tests, CLI, a transport started
//! before `set_progress_sink`), and a closed channel is swallowed — progress is
//! cosmetic and **must never fail or slow a sync**. It is deliberately NOT the
//! `peer_ready` reload trigger, which stays the load-bearing "ops landed"
//! signal.

use std::sync::mpsc::Sender;

use outl_actions::SyncProgress;

/// Clone-cheap sink the sync paths emit [`SyncProgress`] through.
///
/// `Default` is the "no sink" state (every emit a no-op), so a path that
/// hasn't been wired for progress — or a test harness — carries a
/// `ProgressSink::default()` and reports nothing.
#[derive(Clone, Default)]
pub(crate) struct ProgressSink(Option<Sender<SyncProgress>>);

impl ProgressSink {
    /// Wrap a live sender.
    pub(crate) fn new(tx: Sender<SyncProgress>) -> Self {
        Self(Some(tx))
    }

    /// Push one update. Silently drops it when no sink is set or the channel
    /// closed (the UI bridge went away) — never blocks, never errors into the
    /// caller's sync.
    pub(crate) fn emit(&self, progress: SyncProgress) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(progress);
        }
    }
}
