import BackgroundTasks
import Foundation

/// Rust FFI, defined in `src/bg_sync.rs` and linked into the app's static
/// library. Fires a forced iroh sync pass (`sync_now`) against every paired
/// peer and blocks until it lands or a bounded window elapses; returns
/// `false` when iroh isn't wired. Plain C ABI — no args, no pointers.
@_silgen_name("outl_ios_background_sync")
private func outlIosBackgroundSync() -> Bool

/// Background work for P2P sync.
///
/// iOS gives apps two kinds of opportunistic background windows; we use
/// both. Each identifier MUST be listed in Info.plist's
/// `BGTaskSchedulerPermittedIdentifiers`, and the matching `UIBackgroundModes`
/// (`fetch` / `processing`) must be declared, or `register`/`submit` fail.
///
/// - `app.outl.mobile-app.refresh` — a short `BGAppRefreshTask`. A handful
///   of windows a day, ~30s each. A cheap "wake up so the iroh transport can
///   catch up" nudge.
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
/// against every paired peer. The processing handler below keeps the task
/// alive for a bounded window so that pass can finish, then reports
/// completion so iOS keeps granting future windows.
///
/// (Was previously a no-op that relied on iCloud Documents syncing the
/// `ops-*.jsonl` files — that path is gone; iroh is the only sync now.)
@objc(OutlBackgroundRefresh)
public final class OutlBackgroundRefresh: NSObject {

    private static let refreshIdentifier = "app.outl.mobile-app.refresh"
    private static let syncIdentifier = "app.outl.mobile-app.sync"

    /// Floor, not a guarantee — iOS schedules when it can.
    private static let interval: TimeInterval = 60 * 60

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        guard #available(iOS 13.0, *) else { return }
        DispatchQueue.main.async {
            registerTasks()
        }
    }

    @available(iOS 13.0, *)
    private static func registerTasks() {
        let refreshOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: refreshIdentifier,
            using: nil
        ) { task in
            // Reschedule first so a crash still leaves a window armed.
            scheduleRefresh()
            task.setTaskCompleted(success: true)
        }

        let syncOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: syncIdentifier,
            using: nil
        ) { task in
            handleSync(task as! BGProcessingTask)
        }

        if refreshOk { scheduleRefresh() }
        if syncOk { scheduleSync() }
        NSLog("[outl] background tasks registered (refresh=\(refreshOk) sync=\(syncOk))")
    }

    @available(iOS 13.0, *)
    private static func handleSync(_ task: BGProcessingTask) {
        // Reschedule first so a crash still leaves a future window armed.
        scheduleSync()

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
        // (~20s) while iroh dials every peer and exchanges ops. The mobile
        // side initiating is NAT-friendly, so this works even when the Mac
        // can't reach the phone directly.
        DispatchQueue.global(qos: .background).async {
            let ok = outlIosBackgroundSync()
            complete(ok)
        }
        task.expirationHandler = {
            // iOS pulled the window — report now; the FFI thread unwinds on
            // its own and its later `complete(_:)` is a no-op.
            complete(false)
        }
    }

    @available(iOS 13.0, *)
    private static func scheduleRefresh() {
        let req = BGAppRefreshTaskRequest(identifier: refreshIdentifier)
        req.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        submit(req)
    }

    @available(iOS 13.0, *)
    private static func scheduleSync() {
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
