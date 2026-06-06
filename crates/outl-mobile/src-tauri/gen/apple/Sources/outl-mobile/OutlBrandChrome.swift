import OutlKit
import UIKit
import WebKit

/// Paints the UIWindow + WKWebView in `BrandColor.dark` (`#0c0814`)
/// as early as possible during launch.
///
/// **Why this exists:** iOS hands off from `LaunchScreen.storyboard`
/// (brand-dark) to the Tauri WebView. `WKWebView` defaults to
/// `opaque = true` + white background — that white flashed before the
/// first HTML frame rendered, breaking the visual continuity from
/// LaunchScreen → app.
///
/// We fix it by walking the window tree as early as possible:
///   - tint the UIWindow itself, covering any gap between view
///     transitions where the window is briefly visible
///   - mark the WKWebView non-opaque with a brand-coloured background
///     so even its very first frame matches LaunchScreen
///   - apply the same colour to `WKWebView.scrollView` so
///     bounce / overscroll doesn't reveal a white seam
///
/// The 0…1-second window between launch and "WebView mounted" is
/// covered by a retry loop with a hard cap so we never spin forever
/// if something genuinely broke. Brand source-of-truth lives in
/// `OutlKit.BrandColor.dark`; freezing that value is what the unit
/// test in `BrandColorTests.testDarkIsHex0c0814` guards.
@objc(OutlBrandChrome)
public final class OutlBrandChrome: NSObject {

    private static var windowRetries = 0
    private static var webRetries = 0

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        DispatchQueue.main.async {
            apply()
        }
    }

    private static func apply() {
        guard let win = keyWindow() else {
            if windowRetries >= 20 {
                return
            }
            windowRetries += 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                apply()
            }
            return
        }

        let brand = uiColor(from: BrandColor.dark)
        win.backgroundColor = brand
        win.rootViewController?.view.backgroundColor = brand

        if let web = OutlToolbarView.findWebView(in: win) {
            web.isOpaque = false
            web.backgroundColor = brand
            web.scrollView.backgroundColor = brand
            NSLog("[outl] brand chrome applied (window + webview)")
            return
        }

        // WebView not mounted yet — keep polling so we catch its
        // first frame. The window is already tinted so the user
        // sees brand colour throughout this window.
        if webRetries >= 30 {
            NSLog("[outl] brand chrome: webview never mounted, window-only")
            return
        }
        webRetries += 1
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            apply()
        }
    }

    private static func keyWindow() -> UIWindow? {
        for scene in UIApplication.shared.connectedScenes {
            guard let ws = scene as? UIWindowScene else { continue }
            if #available(iOS 15.0, *) {
                if let w = ws.keyWindow { return w }
            }
            if let w = ws.windows.first { return w }
        }
        return UIApplication.shared.windows.first
    }

    private static func uiColor(from c: BrandColor) -> UIColor {
        let comps = c.components
        return UIColor(
            red: CGFloat(comps.red),
            green: CGFloat(comps.green),
            blue: CGFloat(comps.blue),
            alpha: 1.0
        )
    }
}
