import BackgroundTasks
import Foundation
import UIKit

/// Rust FFI, defined in `src/bg_sync.rs` and linked into the app's static
/// library. Fires a forced iroh sync pass (`sync_now`) against every paired
/// peer and blocks until the pass completes or a bounded window (~20s)
/// elapses — early-exiting the moment the pass lands; returns `false` when
/// iroh isn't wired. Plain C ABI — no args, no pointers.
@_silgen_name("outl_ios_background_sync")
private func outlIosBackgroundSync() -> Bool

/// Short-cap variant of the same forced pass (~12s ceiling), sized for the
/// `BGAppRefreshTask`'s ~30s window so the handler can still report
/// completion before iOS expires the task.
@_silgen_name("outl_ios_background_sync_short")
private func outlIosBackgroundSyncShort() -> Bool

/// Number of paired peers, read fresh from `<workspace>/.outl/peers.json`.
/// `0` when the Rust side hasn't registered the transport yet (early in
/// launch, or iroh disabled). Gates BG-task submission below.
@_silgen_name("outl_ios_peer_count")
private func outlIosPeerCount() -> UInt32

/// Background work for P2P sync.
///
/// iOS gives apps two kinds of opportunistic background windows; we use
/// both. Each identifier MUST be listed in Info.plist's
/// `BGTaskSchedulerPermittedIdentifiers`, and the matching `UIBackgroundModes`
/// (`fetch` / `processing`) must be declared, or `register`/`submit` fail.
///
/// - `app.outl.mobile-app.refresh` — a short `BGAppRefreshTask`. A handful
///   of windows a day, ~30s each. Drives a short-capped forced sync pass
///   (`outl_ios_background_sync_short`, ~12s ceiling) so even the cheap
///   windows pull fresh ops instead of being wasted on a bare reschedule.
/// - `app.outl.mobile-app.sync` — a longer `BGProcessingTask` that requires
///   network connectivity. iOS grants it when the device is on Wi-Fi (often
///   charging) and it can run for minutes — enough for an iroh pull/push to
///   complete.
///
/// There is **no continuous background P2P on iOS**: the system suspends the
/// app's sockets the moment it leaves the foreground. These tasks are the
/// only sanctioned way to sync while the app is closed, and the OS — not us —
/// decides when they fire.
///
/// How the sync actually happens: when iOS launches/resumes the app for one
/// of these tasks, the Tauri `setup` hook brings `IrohSyncTransport` up (the
/// same path as a normal launch), and its catch-up loop runs a delta sync
/// against every paired peer. The handlers below keep the task alive only
/// until the forced pass reports completion (or a bounded ceiling), then
/// report completion so iOS keeps granting future windows — returning unused
/// window early is what keeps the grants coming.
///
/// Scheduling is gated on having at least one paired peer
/// (`outl_ios_peer_count() > 0`): with zero peers a background wake boots
/// the whole app for nothing. The handlers are ALWAYS registered (mandatory
/// before the end of launch); only the `submit` is conditional. Because the
/// launch-time submit usually runs before the Rust side has registered the
/// transport (peer count reads 0), the schedule is re-armed on every
/// `didEnterBackground` — which also arms it right after the user pairs
/// their first peer with the app open, no Rust→Swift bridge needed.
@objc(OutlBackgroundRefresh)
public final class OutlBackgroundRefresh: NSObject {

    private static let refreshIdentifier = "app.outl.mobile-app.refresh"
    private static let syncIdentifier = "app.outl.mobile-app.sync"

    /// Floor, not a guarantee — iOS schedules when it can. Kept modest (15 min,
    /// not 1 h) so the scheduler has more latitude to grant a window; a larger
    /// floor is a self-imposed ceiling on how soon a background sync can run.
    private static let interval: TimeInterval = 15 * 60

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        guard #available(iOS 13.0, *) else { return }
        // Register the launch handlers SYNCHRONOUSLY, right here. Apple requires
        // every `BGTaskScheduler` handler to be registered before the app
        // finishes launching — otherwise a COLD background launch for that task
        // can't find its handler and iOS silently drops the window (the likely
        // cause of "doesn't sync while closed"). `register` needs no UIKit state
        // (only `submit`/scheduling touch app state), and `install()` is already
        // hopped onto the main queue by `main.mm`'s `+load`, so the previous
        // extra `DispatchQueue.main.async` here only pushed registration a
        // runloop turn too late. Keep only the observer registration inline too.
        registerTasks()
        // Re-arm on every backgrounding: at launch `registerTasks` runs BEFORE
        // the Rust side registers the transport (peer count reads 0, so nothing
        // is submitted), and pairing the first peer happens with the app open.
        // Both cases arm here, the moment the app actually goes to background.
        // Re-submitting an already-pending identifier just replaces the request,
        // so this is idempotent.
        NotificationCenter.default.addObserver(
            forName: UIApplication.didEnterBackgroundNotification,
            object: nil,
            queue: .main
        ) { _ in
            scheduleRefresh()
            scheduleSync()
        }
    }

    @available(iOS 13.0, *)
    private static func registerTasks() {
        let refreshOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: refreshIdentifier,
            using: nil
        ) { task in
            // The refresh window is ~30s: run the short-capped pass so the
            // sync finishes (or bails) with headroom to report completion.
            handleTask(task, reschedule: scheduleRefresh) {
                outlIosBackgroundSyncShort()
            }
        }

        let syncOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: syncIdentifier,
            using: nil
        ) { task in
            handleTask(task, reschedule: scheduleSync) {
                outlIosBackgroundSync()
            }
        }

        if refreshOk { scheduleRefresh() }
        if syncOk { scheduleSync() }
        NSLog("[outl] background tasks registered (refresh=\(refreshOk) sync=\(syncOk))")
    }

    /// Shared BG-task driver: reschedule, run the sync FFI off-thread,
    /// report completion exactly once.
    @available(iOS 13.0, *)
    private static func handleTask(
        _ task: BGTask,
        reschedule: () -> Void,
        sync: @escaping () -> Bool
    ) {
        // Reschedule first so a crash still leaves a future window armed.
        // (The submit inside is peer-gated, so an unpaired device stops
        // rescheduling itself here and re-arms on the next backgrounding
        // after a pair.)
        reschedule()

        // Report completion exactly once — the sync work and the OS
        // expiration handler race, and BGTaskScheduler rejects a double
        // `setTaskCompleted`.
        let lock = NSLock()
        var reported = false
        func complete(_ success: Bool) {
            lock.lock()
            defer { lock.unlock() }
            guard !reported else { return }
            reported = true
            task.setTaskCompleted(success: success)
        }

        // Drive the actual pull/push on a background queue: the FFI blocks
        // while iroh dials every peer and exchanges ops, returning as soon
        // as the forced pass completes (bounded by its cap). The mobile side
        // initiating is NAT-friendly, so this works even when the Mac can't
        // reach the phone directly.
        DispatchQueue.global(qos: .background).async {
            let ok = sync()
            complete(ok)
        }
        task.expirationHandler = {
            // iOS pulled the window — report now; the FFI thread unwinds on
            // its own and its later `complete(_:)` is a no-op.
            complete(false)
        }
    }

    /// Gate: with zero paired peers a background wake boots the whole app
    /// for nothing, so submission is skipped. `outlIosPeerCount()` reads
    /// `<workspace>/.outl/peers.json` through the transport registered by
    /// the Rust side — it returns 0 until that registration happens (early
    /// in launch); the `didEnterBackground` re-arm in `install()` covers
    /// that window and the "first peer paired with the app open" case.
    private static func peersArePaired() -> Bool {
        let count = outlIosPeerCount()
        if count == 0 {
            NSLog("[outl] bg schedule skipped: no paired peers")
        }
        return count > 0
    }

    @available(iOS 13.0, *)
    private static func scheduleRefresh() {
        guard peersArePaired() else { return }
        let req = BGAppRefreshTaskRequest(identifier: refreshIdentifier)
        req.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        submit(req)
    }

    @available(iOS 13.0, *)
    private static func scheduleSync() {
        guard peersArePaired() else { return }
        let req = BGProcessingTaskRequest(identifier: syncIdentifier)
        req.requiresNetworkConnectivity = true
        req.requiresExternalPower = false
        req.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        submit(req)
    }

    @available(iOS 13.0, *)
    private static func submit(_ req: BGTaskRequest) {
        do {
            try BGTaskScheduler.shared.submit(req)
        } catch {
            #if targetEnvironment(simulator)
            // No BGTaskScheduler daemon on the sim — submit always fails;
            // swallow so dev builds stay quiet. Registration still works.
            #else
            NSLog("[outl] schedule \(req.identifier) failed: \(error.localizedDescription)")
            #endif
        }
    }
}
