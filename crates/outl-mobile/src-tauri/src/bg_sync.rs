//! Background-sync FFI for the iOS `BGProcessingTask` / `BGAppRefreshTask`.
//!
//! iOS suspends the app's sockets in the background, so the only way to
//! sync while the app is closed is an opportunistic background window (see
//! `OutlBackgroundRefresh.swift`). When iOS grants one, the Tauri `setup`
//! hook has already brought `IrohSyncTransport` up; the Swift handler then
//! calls one of the FFIs below to **actively** drive a sync pass instead of
//! just waiting.
//!
//! The live transport (plus the workspace root) registers itself here at
//! boot via [`register`]. Three C-ABI symbols are exposed to Swift:
//!
//! - [`outl_ios_background_sync`] — forced sync pass, ≤ [`SYNC_WINDOW`]
//!   (the `BGProcessingTask`, minutes of budget).
//! - [`outl_ios_background_sync_short`] — same pass, ≤ [`REFRESH_WINDOW`]
//!   (the `BGAppRefreshTask`, whose whole window is ~30s).
//! - [`outl_ios_peer_count`] — paired-peer count, read fresh from
//!   `<workspace>/.outl/peers.json`; the Swift side gates task *submission*
//!   on it so a device with nothing to sync never burns background budget.
//!
//! Instead of sleeping a fixed worst-case window, the sync FFIs fire
//! `sync_now()` and **poll** [`IrohSyncTransport::completed_sync_passes`]
//! every [`POLL_INTERVAL`]: the moment a forced pass finishes its dial cycle
//! over every peer, the FFI returns and hands the unused window back to iOS
//! (which rewards short tasks with more frequent grants). The cap is only
//! the fallback for a pass that outlives the window.
//!
//! Everything here is deliberately panic-free (no `unwrap`/`expect`,
//! `parking_lot` locks don't poison), because a panic must never unwind
//! across the C ABI into Swift.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use outl_actions::SyncTransport;
use outl_sync_iroh::{workspace_peers_path, IrohSyncTransport, PeersStore};
use parking_lot::Mutex;
use tracing::{info, warn};

/// Upper bound for [`outl_ios_background_sync`] (the `BGProcessingTask`).
/// Cross-network iroh connects can take ~20s (multipath), and it stays under
/// the `BGProcessingTask` budget. With early-exit this is a *cap*, not the
/// typical duration — a same-LAN pass returns in a couple of seconds.
const SYNC_WINDOW: Duration = Duration::from_secs(20);

/// Upper bound for [`outl_ios_background_sync_short`] (the
/// `BGAppRefreshTask`). Its whole window is ~30s, so the cap leaves headroom
/// for the Swift handler to report completion before iOS expires the task.
const REFRESH_WINDOW: Duration = Duration::from_secs(12);

/// How often the sync FFIs re-check the completed-pass counter while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// What the BG-task FFIs need from the live app: the transport handle (to
/// fire + observe a forced sync) and the workspace root (to read the paired
/// peer list off `peers.json` — the on-disk file is the source of truth;
/// the transport's in-memory store is a boot-time snapshot that pairing
/// after boot does not refresh).
#[derive(Clone)]
struct Registration {
    transport: IrohSyncTransport,
    workspace_root: PathBuf,
}

/// The live registration, refreshed every boot. A re-settable slot (not a
/// bare `OnceLock<Registration>`) so a relaunch / workspace reopen replaces a
/// stale handle instead of keeping the first one forever.
fn slot() -> &'static Mutex<Option<Registration>> {
    static SLOT: OnceLock<Mutex<Option<Registration>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Stash a clone of the live transport + the workspace root so the BG-task
/// FFIs can reach them. Called from `iroh_sync::wire_iroh_transport` after
/// the transport starts. `IrohSyncTransport` is `Clone` (its handles are
/// `Arc`-backed), so the clone drives the same endpoint as `AppState.iroh`.
pub(crate) fn register(transport: &IrohSyncTransport, workspace_root: PathBuf) {
    *slot().lock() = Some(Registration {
        transport: transport.clone(),
        workspace_root,
    });
}

/// Drive one forced sync pass from an iOS background task, waiting at most
/// [`SYNC_WINDOW`] (early-exiting as soon as the pass completes).
///
/// Returns `true` when a transport was wired and a sync was fired, `false`
/// when iroh is off (no transport) so the Swift side can mark the task
/// accordingly.
///
/// Plain C ABI, no arguments, no pointers — safe to call from Swift via
/// `@_silgen_name("outl_ios_background_sync")`.
#[no_mangle]
pub extern "C" fn outl_ios_background_sync() -> bool {
    drive_sync(SYNC_WINDOW)
}

/// Like [`outl_ios_background_sync`], but capped at [`REFRESH_WINDOW`] so it
/// fits the `BGAppRefreshTask`'s ~30s window. A second symbol (instead of a
/// parameter) keeps the ABI trivially safe — no argument marshalling across
/// the FFI, and each call site documents its own budget.
///
/// Swift binding: `@_silgen_name("outl_ios_background_sync_short")`.
#[no_mangle]
pub extern "C" fn outl_ios_background_sync_short() -> bool {
    drive_sync(REFRESH_WINDOW)
}

/// Number of paired peers for the registered workspace, or `0` when nothing
/// is registered (transport off / boot not finished) or the peer list can't
/// be read.
///
/// Read fresh from `<workspace>/.outl/peers.json` on every call — pairing
/// and peer removal both write that file, so the count is current even for
/// peers added after the transport booted. The Swift side gates BG-task
/// *submission* on `> 0`: with zero peers a background wake boots the whole
/// app for nothing.
///
/// Plain C ABI, no arguments, no pointers — safe to call from Swift via
/// `@_silgen_name("outl_ios_peer_count")`.
#[no_mangle]
pub extern "C" fn outl_ios_peer_count() -> u32 {
    let root = match slot().lock().as_ref() {
        Some(reg) => reg.workspace_root.clone(),
        None => return 0,
    };
    peer_count_at(&root)
}

/// Fire a forced sync pass and wait for its completion (early-exit) or the
/// `cap` (fallback), whichever comes first. `false` iff no transport is
/// registered.
fn drive_sync(cap: Duration) -> bool {
    // Clone the handle out so we don't hold the lock across the wait below.
    let Some(reg) = slot().lock().clone() else {
        return false;
    };
    let transport = reg.transport;
    // Snapshot BEFORE firing: any pass completing after this point implies a
    // full dial cycle over the current peer set ran after our request (the
    // drain task increments once per drained request — see
    // `outl_sync_iroh::IrohSyncTransport::completed_sync_passes`).
    let baseline = transport.completed_sync_passes();
    transport.sync_now();
    let completed = wait_until(cap, POLL_INTERVAL, || {
        transport.completed_sync_passes() > baseline
    });
    if completed {
        info!("bg-sync: forced pass completed, returning window early");
    } else {
        info!("bg-sync: window elapsed before pass completion");
    }
    true
}

/// Poll `probe` every `poll` until it returns `true` or `cap` elapses.
/// Returns whether the probe fired (`false` = timed out).
///
/// Kept separate from [`drive_sync`] so the early-exit/timeout contract is
/// testable without a live transport.
fn wait_until(cap: Duration, poll: Duration, probe: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + cap;
    loop {
        if probe() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        // Never oversleep the deadline on the last iteration.
        std::thread::sleep(poll.min(deadline - now));
    }
}

/// Count the paired peers recorded in `<root>/.outl/peers.json`. `0` on a
/// missing or unreadable file — the conservative answer for the scheduling
/// gate (better to skip a window than to wake for a corrupt list).
fn peer_count_at(root: &Path) -> u32 {
    match PeersStore::load_or_default(&workspace_peers_path(root)) {
        Ok(store) => u32::try_from(store.list().len()).unwrap_or(u32::MAX),
        Err(e) => {
            warn!("bg-sync: peer count read failed: {e}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Early-exit contract: the wait returns `true` as soon as the probe
    /// fires, well before the cap.
    #[test]
    fn wait_until_returns_early_when_probe_fires() {
        let calls = AtomicU32::new(0);
        let started = Instant::now();
        let fired = wait_until(Duration::from_secs(10), Duration::from_millis(5), || {
            calls.fetch_add(1, Ordering::Relaxed) >= 3
        });
        assert!(fired);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "early-exit should not wait anywhere near the cap"
        );
    }

    /// Timeout contract: a probe that never fires bounds the wait at the cap.
    #[test]
    fn wait_until_times_out_at_the_cap() {
        let started = Instant::now();
        let fired = wait_until(Duration::from_millis(60), Duration::from_millis(10), || {
            false
        });
        assert!(!fired);
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    /// `peer_count_at` reads the on-disk peers.json fresh: absent file → 0,
    /// entries present → their count.
    #[test]
    fn peer_count_reads_peers_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(peer_count_at(tmp.path()), 0, "no peers.json yet");

        let path = workspace_peers_path(tmp.path());
        let mut store = PeersStore::load_or_default(&path).expect("empty store");
        for i in 0..2u8 {
            store
                .add(outl_sync_iroh::PeerEntry {
                    node_id: format!("test-node-{i}"),
                    alias: None,
                    relay_url: None,
                    endpoint_addr: None,
                    added_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .expect("add peer");
        }
        assert_eq!(peer_count_at(tmp.path()), 2);
    }

    /// The C-ABI surface follows the registration slot: everything reports
    /// "off" before `register`, and the peer count reflects the registered
    /// workspace afterwards. One sequential test because the slot is a
    /// process-wide global.
    #[test]
    fn ffi_surface_follows_registration() {
        // Before any registration: sync reports "no transport", count is 0.
        assert!(!outl_ios_background_sync());
        assert!(!outl_ios_background_sync_short());
        assert_eq!(outl_ios_peer_count(), 0);

        let tmp = tempfile::tempdir().expect("tempdir");
        let identity =
            outl_sync_iroh::IrohIdentity::load_or_generate(&tmp.path().join("identity.key"))
                .expect("identity");
        let peers_path = workspace_peers_path(tmp.path());
        let mut peers = PeersStore::load_or_default(&peers_path).expect("empty store");
        peers
            .add(outl_sync_iroh::PeerEntry {
                node_id: "test-node".to_string(),
                alias: Some("test".to_string()),
                relay_url: None,
                endpoint_addr: None,
                added_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .expect("add peer");
        // Unstarted transport is fine: peer count never touches the runtime.
        let transport = IrohSyncTransport::new(identity, peers, None);

        register(&transport, tmp.path().to_path_buf());
        assert_eq!(outl_ios_peer_count(), 1);

        // Leave the slot empty for any test run after this one.
        *slot().lock() = None;
        assert_eq!(outl_ios_peer_count(), 0);
    }
}
