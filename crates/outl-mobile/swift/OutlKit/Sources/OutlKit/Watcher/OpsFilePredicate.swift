import Foundation

/// Predicates / matchers used by the iCloud peer-file watcher.
///
/// `OutlOpsWatcher` runs an `NSMetadataQuery` against the ubiquity
/// container and listens for changes on each peer's `ops-<actor>.jsonl`.
/// The same matcher logic also doubles as a sanity check on URLs
/// surfaced by `NSMetadataItem` results — a query that returns the
/// wrong file (extension casing, accidental temp files) would corrupt
/// the op log merge silently.
public enum OpsFilePredicate {

    /// Filename pattern shipped to `NSMetadataQuery`. Wildcard `*` is
    /// the actor id (an ULID), so the suffix has to start with `ops-`
    /// and end with `.jsonl`.
    public static let nsMetadataFormat = "%K LIKE 'ops-*.jsonl'"

    /// The `NSPredicate` instance the watcher passes to
    /// `NSMetadataQuery.predicate`. Built lazily because
    /// `NSPredicate(format:)` does string interpolation and we want a
    /// fresh predicate per query (safer with KVC keypaths).
    public static func nsMetadataPredicate() -> NSPredicate {
        NSPredicate(format: nsMetadataFormat, NSMetadataItemFSNameKey)
    }

    /// Lightweight check on a raw filename string. Used after a
    /// metadata-query result comes back so we can reject anything
    /// that snuck past the predicate (e.g. a hidden temp file with a
    /// matching prefix that the metadata index briefly surfaced).
    ///
    /// Rules:
    ///   - starts with `ops-`
    ///   - ends with `.jsonl`
    ///   - has at least one character between the prefix and suffix
    ///     (so a literal `ops-.jsonl` is rejected)
    ///   - no path separators inside (caller passes basename only)
    public static func isOpsJsonl(filename: String) -> Bool {
        guard filename.hasPrefix("ops-"),
              filename.hasSuffix(".jsonl"),
              !filename.contains("/"),
              !filename.contains("\\")
        else { return false }
        // Length check: "ops-" (4) + at least 1 actor char + ".jsonl" (6)
        return filename.count > 10
    }
}
