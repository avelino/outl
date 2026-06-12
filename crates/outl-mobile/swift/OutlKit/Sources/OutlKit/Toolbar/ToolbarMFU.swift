import Foundation

/// Most-frequently-used ordering for the edit toolbar.
///
/// Persists per-device tap counts under `outl.toolbar.mfu.v1` in
/// `UserDefaults` and re-orders the middle of the row by count desc on
/// each read. The first and last slots stay pinned (`newLine` /
/// `done`) regardless of how often the user taps them.
///
/// `UserDefaults` is injectable so tests can pass an isolated suite
/// and avoid touching the device's real prefs.
public enum ToolbarMFU {

    /// Key used in `UserDefaults`. Versioned (`.v1`) so a future
    /// schema change doesn't try to read old shapes.
    public static let storageKey = "outl.toolbar.mfu.v1"

    /// Read counts from `defaults`. Tolerant of `NSNumber`-bridged
    /// values (UserDefaults brings everything back as `NSNumber`),
    /// and returns an empty dictionary if the key is missing or the
    /// stored value isn't a dictionary.
    public static func readCounts(
        defaults: UserDefaults = .standard
    ) -> [String: Int] {
        let raw = defaults.dictionary(forKey: storageKey) ?? [:]
        var counts: [String: Int] = [:]
        for (key, value) in raw {
            if let n = value as? Int {
                counts[key] = n
            } else if let n = value as? NSNumber {
                counts[key] = n.intValue
            }
        }
        return counts
    }

    /// Increment the count for `action` and persist. No-op for the
    /// pinned actions — their slot is fixed by position, so counting
    /// them just wastes storage.
    public static func record(
        _ action: ToolbarAction,
        defaults: UserDefaults = .standard
    ) {
        guard
            action != ToolbarAction.pinnedFirst,
            action != ToolbarAction.pinnedLast
        else { return }
        var counts = readCounts(defaults: defaults)
        counts[action.rawValue, default: 0] += 1
        defaults.set(counts, forKey: storageKey)
    }

    /// Wipe all counts. Exposed for tests + a future "reset toolbar"
    /// affordance in settings.
    public static func clearCounts(defaults: UserDefaults = .standard) {
        defaults.removeObject(forKey: storageKey)
    }

    /// Pure ordering function — takes an explicit `counts` dictionary
    /// so it stays deterministic and testable. Tests should call this
    /// overload; the app target uses the convenience overload below
    /// that reads from `UserDefaults`.
    public static func orderedActions(
        counts: [String: Int]
    ) -> [ToolbarAction] {
        [ToolbarAction.pinnedFirst]
            + orderedMiddleActions(counts: counts)
            + [ToolbarAction.pinnedLast]
    }

    /// Same MFU ordering as `orderedActions`, but **excludes the
    /// pinned slots** — only the middle (scrollable) range comes back.
    /// The view layer uses this to populate the `UIScrollView`'s stack
    /// while keeping `pinnedFirst` / `pinnedLast` as static buttons
    /// outside the scroll, so they stay visible regardless of how far
    /// the user scrolled the middle row.
    public static func orderedMiddleActions(
        counts: [String: Int]
    ) -> [ToolbarAction] {
        let middle = ToolbarAction.defaultOrder.filter {
            $0 != ToolbarAction.pinnedFirst && $0 != ToolbarAction.pinnedLast
        }
        return middle.sorted { a, b in
            let ca = counts[a.rawValue] ?? 0
            let cb = counts[b.rawValue] ?? 0
            if ca != cb { return ca > cb }
            // Stable tiebreak: original `defaultOrder` position.
            let ia = ToolbarAction.defaultOrder.firstIndex(of: a) ?? 0
            let ib = ToolbarAction.defaultOrder.firstIndex(of: b) ?? 0
            return ia < ib
        }
    }

    /// Convenience overload that reads counts from `defaults` and
    /// orders. The app target's `OutlToolbarView` calls this on every
    /// `rebuildButtons()`.
    public static func orderedActions(
        defaults: UserDefaults = .standard
    ) -> [ToolbarAction] {
        orderedActions(counts: readCounts(defaults: defaults))
    }

    /// Convenience overload of [`orderedMiddleActions(counts:)`] that
    /// reads counts from `defaults`.
    public static func orderedMiddleActions(
        defaults: UserDefaults = .standard
    ) -> [ToolbarAction] {
        orderedMiddleActions(counts: readCounts(defaults: defaults))
    }
}
