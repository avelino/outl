//! Background-sync FFI for the iOS `BGProcessingTask`.
//!
//! iOS suspends the app's sockets in the background, so the only way to
//! sync while the app is closed is an opportunistic `BGProcessingTask`
//! window (see `OutlBackgroundRefresh.swift`). When iOS grants one, the
//! Tauri `setup` hook has already brought `IrohSyncTransport` up; the Swift
//! handler then calls [`outl_ios_background_sync`] to **actively** drive a
//! sync pass instead of just waiting.
//!
//! The live transport registers itself here at boot via [`register`]; the
//! FFI reads that handle, fires `sync_now()` (a forced delta-sync against
//! every paired peer — the mobile side initiates, which is NAT-friendly),
//! and blocks briefly so the QUIC pull/push can land before the handler
//! reports completion to iOS.

use std::sync::OnceLock;
use std::time::Duration;

use outl_actions::SyncTransport;
use outl_sync_iroh::IrohSyncTransport;
use parking_lot::Mutex;

/// How long the FFI blocks after firing `sync_now`, giving the forced
/// delta-sync time to dial peers + exchange ops. Cross-network iroh
/// connects can take ~20s (multipath), and it stays under the
/// `BGProcessingTask` budget.
const SYNC_WINDOW: Duration = Duration::from_secs(20);

/// The live transport, refreshed every boot. A re-settable slot (not a bare
/// `OnceLock<IrohSyncTransport>`) so a relaunch / workspace reopen replaces a
/// stale handle instead of keeping the first one forever.
fn slot() -> &'static Mutex<Option<IrohSyncTransport>> {
    static SLOT: OnceLock<Mutex<Option<IrohSyncTransport>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Stash a clone of the live transport so the `BGProcessingTask` FFI can
/// reach it. Called from `iroh_sync::wire_iroh_transport` after the
/// transport starts. `IrohSyncTransport` is `Clone` (its handles are
/// `Arc`-backed), so the clone drives the same endpoint as `AppState.iroh`.
pub(crate) fn register(transport: &IrohSyncTransport) {
    *slot().lock() = Some(transport.clone());
}

/// Drive one forced sync pass from an iOS background task.
///
/// Returns `true` when a transport was wired and a sync was fired, `false`
/// when iroh is off (no transport) so the Swift side can mark the task
/// accordingly. Blocks for [`SYNC_WINDOW`] so the pass can complete before
/// the caller reports completion to `BGTaskScheduler`.
///
/// Plain C ABI, no arguments, no pointers — safe to call from Swift via
/// `@_silgen_name("outl_ios_background_sync")`.
#[no_mangle]
pub extern "C" fn outl_ios_background_sync() -> bool {
    // Clone the handle out so we don't hold the lock across the blocking
    // wait below.
    let Some(transport) = slot().lock().clone() else {
        return false;
    };
    transport.sync_now();
    std::thread::sleep(SYNC_WINDOW);
    true
}
