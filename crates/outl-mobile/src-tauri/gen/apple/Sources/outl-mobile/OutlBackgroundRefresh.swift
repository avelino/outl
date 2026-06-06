import BackgroundTasks
import Foundation

/// Background app refresh registration.
///
/// Registers a `BGAppRefreshTask` under `app.outl.mobile-app.refresh`
/// (must be listed in Info.plist's `BGTaskSchedulerPermittedIdentifiers`
/// — see CLAUDE.md). iOS calls the handler opportunistically a handful
/// of times per day; we mark it successful immediately because iCloud
/// Documents already syncs `ops-*.jsonl` in the background. This hook
/// exists so future versions can warm up the workspace or pre-render
/// `.md` projections before the user reopens the app.
///
/// On the simulator, `BGTaskScheduler.submit` always fails with error
/// 1 because there's no daemon. We swallow that error there so dev
/// builds stay quiet — registration itself still works, which is all
/// that matters for everything downstream.
@objc(OutlBackgroundRefresh)
public final class OutlBackgroundRefresh: NSObject {

    private static let taskIdentifier = "app.outl.mobile-app.refresh"

    /// 1 hour minimum interval. iOS takes this as a floor, not a
    /// commitment — the system schedules tasks when it can.
    private static let refreshEvery: TimeInterval = 60 * 60

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        guard #available(iOS 13.0, *) else { return }
        DispatchQueue.main.async {
            registerTask()
        }
    }

    @available(iOS 13.0, *)
    private static func registerTask() {
        let ok = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: taskIdentifier,
            using: nil
        ) { task in
            // Always schedule the next refresh first — even if our
            // handler somehow crashed, iOS would still call us next
            // window. Then mark success so the budget grows.
            scheduleNextRefresh()
            task.setTaskCompleted(success: true)
        }
        if ok {
            NSLog("[outl] registered background refresh")
            scheduleNextRefresh()
        } else {
            NSLog("[outl] failed to register background refresh")
        }
    }

    @available(iOS 13.0, *)
    private static func scheduleNextRefresh() {
        let req = BGAppRefreshTaskRequest(identifier: taskIdentifier)
        req.earliestBeginDate = Date(timeIntervalSinceNow: refreshEvery)
        do {
            try BGTaskScheduler.shared.submit(req)
        } catch {
            #if targetEnvironment(simulator)
            // No BGTaskScheduler daemon on the sim — submit always
            // fails. Ignore so dev builds don't spam the log.
            #else
            NSLog("[outl] schedule next refresh failed: \(error.localizedDescription)")
            #endif
        }
    }
}
