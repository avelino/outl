import OutlKit
import ObjectiveC.runtime
import UIKit
import WebKit

/// Replaces the `inputAccessoryView` of WebKit's private `WKContentView`
/// with our `OutlToolbarView` via method swizzling.
///
/// **Why we have to swizzle:** the iOS form input accessory bar
/// (`↑ ↓ ✓`) is supplied by `WKContentView`'s default
/// `inputAccessoryView` getter, and there's no public API to swap it.
/// `WKContentView` is a private class in WebKit, so we look it up by
/// name and replace the method's IMP at runtime with a closure that
/// returns our toolbar singleton.
///
/// **Why this lives in Swift now:** the original Obj-C version called
/// `imp_implementationWithBlock` directly. Swift can do the same via
/// `ObjectiveC.runtime` — the only twist is the closure must be
/// `@convention(block)` so its ABI matches what `imp_implementationWithBlock`
/// expects.
@objc(OutlSwizzle)
public final class OutlSwizzle: NSObject {

    /// Toolbar singleton handed back to UIKit as the keyboard's
    /// `inputAccessoryView`. One per app lifetime so the same view
    /// instance animates with the keyboard across every focus change.
    private static var sharedToolbar: OutlToolbarView?

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    /// Schedules all the steps that have to run after `main()`
    /// finishes wiring UIKit.
    @objc public static func install() {
        DispatchQueue.main.async {
            installSwizzle()
        }
    }

    // MARK: - Swizzle install

    private static var swizzleRetries = 0

    private static func installSwizzle() {
        // `WKContentView` isn't registered with the Obj-C runtime
        // until at least one `WKWebView` exists. Retry up to ~1s
        // after launch so we catch it without busy-looping.
        guard let cls: AnyClass = NSClassFromString("WKContentView") else {
            if swizzleRetries >= 10 {
                NSLog("[outl] gave up looking for WKContentView")
                return
            }
            swizzleRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                installSwizzle()
            }
            return
        }

        let sel = #selector(getter: UIResponder.inputAccessoryView)
        guard let method = class_getInstanceMethod(cls, sel) else { return }

        // The block here MUST be `@convention(block)`. Without it, the
        // Swift closure ABI doesn't match what `imp_implementationWithBlock`
        // builds — you'd get a crash the first time UIKit dispatches
        // to the swizzled IMP.
        let block: @convention(block) (AnyObject) -> UIView? = { _ in
            if sharedToolbar == nil {
                sharedToolbar = OutlToolbarView()
            }
            return sharedToolbar
        }
        let imp = imp_implementationWithBlock(block)
        method_setImplementation(method, imp)
        NSLog("[outl] installed native toolbar (with embedded suggester)")

        disableInteractiveDismiss()
        bindWebView()
    }

    // MARK: - Disable interactive keyboard dismiss

    private static var dismissRetries = 0

    /// iOS's "interactive keyboard dismiss" gesture lets the user
    /// drag down on a `UIScrollView` to slide the keyboard out. Our
    /// outline is itself a scroll view, so without this fix any
    /// down-drag inside the editor partially dismisses the keyboard
    /// and drags the toolbar along — confusing UX.
    private static func disableInteractiveDismiss() {
        guard let win = keyWindow(),
              let web = OutlToolbarView.findWebView(in: win)
        else {
            if dismissRetries >= 10 { return }
            dismissRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                disableInteractiveDismiss()
            }
            return
        }
        web.scrollView.keyboardDismissMode = .none
        NSLog("[outl] disabled interactive keyboard dismiss")
    }

    // MARK: - Bind WebView into toolbar + suggester overlay

    private static var bindRetries = 0

    private static func bindWebView() {
        guard let win = keyWindow(),
              let web = OutlToolbarView.findWebView(in: win)
        else {
            if bindRetries >= 20 { return }
            bindRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                bindWebView()
            }
            return
        }
        if sharedToolbar == nil {
            sharedToolbar = OutlToolbarView()
        }
        sharedToolbar?.webView = web
        OutlSuggestOverlay.install(webView: web)
        NSLog("[outl] bound suggester webview (overlay-driven)")
    }

    // MARK: - Helpers

    private static func keyWindow() -> UIWindow? {
        for scene in UIApplication.shared.connectedScenes {
            guard let ws = scene as? UIWindowScene else { continue }
            if let w = ws.windows.first { return w }
        }
        return UIApplication.shared.windows.first
    }
}
