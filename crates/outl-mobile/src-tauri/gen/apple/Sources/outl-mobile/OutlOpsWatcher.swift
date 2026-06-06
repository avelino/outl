import Foundation
import OutlKit
import UIKit
import WebKit

/// iCloud real-time watcher.
///
/// `NSMetadataQuery` is the only public API for being told when iCloud
/// documents change. We scope it to `ops-*.jsonl` files anywhere in the
/// app's ubiquitous documents and notify the WebView via
/// `window.__outlOpsChanged()` so the Solid frontend can reload the
/// workspace without the user having to pull-to-refresh.
///
/// Updates are debounced via `notifyPending` so a burst of files
/// arriving from iCloud only fires one refresh through the JS bridge.
///
/// **Peer-file materialization is critical.** `NSMetadataItem` fires
/// when iCloud knows a file *exists*, but the file's bytes may not
/// be downloaded yet — opening it from Rust's `std::fs::open` would
/// see an empty placeholder. We sequence:
///   1. `startDownloadingUbiquitousItem` — request materialization
///   2. `NSFileCoordinator` — block until the file is fully on disk
/// before notifying JS. Skip either step and you race the iCloud
/// download daemon. See CLAUDE.md → "Peer-file materialisation".
@objc(OutlOpsWatcher)
public final class OutlOpsWatcher: NSObject {

    /// Singleton bound to the app lifetime.
    @objc public static let shared = OutlOpsWatcher()

    private var query: NSMetadataQuery?
    private var notifyPending = false

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    /// Delays start by 1s so the iCloud ubiquity container resolver
    /// has time to finish initializing.
    @objc public static func install() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            shared.start()
        }
    }

    /// Begin watching `ops-*.jsonl` under the ubiquitous documents
    /// scope. No-op if already started.
    @objc public func start() {
        guard query == nil else { return }
        let q = NSMetadataQuery()
        q.searchScopes = [NSMetadataQueryUbiquitousDocumentsScope]
        // v0 contract: `ops-<actor>.jsonl` is the wire format peers
        // append to. iCloud syncs each file independently and each
        // actor only ever writes its own jsonl, so concurrent edits
        // never produce a conflicting file — the CRDT does the merge
        // after we reload. The predicate comes from `OutlKit` so the
        // pattern is unit-tested and can't drift between this class
        // and the local matcher.
        q.predicate = OpsFilePredicate.nsMetadataPredicate()

        let nc = NotificationCenter.default
        nc.addObserver(
            self,
            selector: #selector(onUpdate(_:)),
            name: .NSMetadataQueryDidUpdate,
            object: q
        )
        nc.addObserver(
            self,
            selector: #selector(onUpdate(_:)),
            name: .NSMetadataQueryDidFinishGathering,
            object: q
        )

        self.query = q
        q.start()
    }

    @objc private func onUpdate(_ note: Notification) {
        guard !notifyPending, let q = query else { return }
        notifyPending = true

        // Snapshot the current results on the main thread. Disabling
        // updates around the iteration prevents NSMetadataQuery from
        // mutating the result set under us.
        q.disableUpdates()
        var urls: [URL] = []
        urls.reserveCapacity(q.resultCount)
        for i in 0..<q.resultCount {
            guard let item = q.result(at: i) as? NSMetadataItem,
                  let url = item.value(forAttribute: NSMetadataItemURLKey) as? URL
            else { continue }
            urls.append(url)
        }
        q.enableUpdates()

        DispatchQueue.global(qos: .utility).async { [weak self] in
            self?.materialize(urls: urls)
            DispatchQueue.main.async {
                self?.notifyJSAndClear()
            }
        }
    }

    private func materialize(urls: [URL]) {
        let fm = FileManager.default
        let coord = NSFileCoordinator(filePresenter: nil)
        for url in urls {
            do {
                try fm.startDownloadingUbiquitousItem(at: url)
            } catch {
                // Non-fatal — the coordinator below still waits on
                // any in-flight download iCloud started on its own.
            }
            var coordErr: NSError?
            coord.coordinate(
                readingItemAt: url,
                options: .forUploading,
                error: &coordErr
            ) { _ in
                // The act of coordinating is what blocks until the
                // file is on disk; the accessor body has no work.
            }
        }
    }

    private func notifyJSAndClear() {
        notifyPending = false
        guard
            let window = UIApplication.shared.windows.first,
            let web = OutlToolbarView.findWebView(in: window)
        else { return }
        web.evaluateJavaScript(
            "window.__outlOpsChanged && window.__outlOpsChanged()",
            completionHandler: nil
        )
    }
}
