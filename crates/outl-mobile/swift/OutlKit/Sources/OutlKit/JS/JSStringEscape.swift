import Foundation

/// Escape a string for safe interpolation inside a JS single-quoted
/// literal that we pass to `WKWebView.evaluateJavaScript`.
///
/// We build strings like `window.__outlToolbar('\(action)')` from the
/// Swift side and ship them to the WebView. Anything the user typed
/// (block content, slug, kind) that ends up inside those single
/// quotes can break the JS syntax (or worse, inject a call) if not
/// escaped first.
///
/// Scope: this is the minimum set required for our specific call
/// sites (single-quote literals only, no template strings, no JSON
/// embedding). For arbitrary JS interop we'd want a full encoder; for
/// the action / slug / kind values shipped here, escaping `\` and `'`
/// is enough.
public enum JSStringEscape {

    /// Escape `\` and `'` for embedding inside `'…'` in JS source.
    ///
    /// Order matters: backslashes are doubled FIRST so the
    /// just-introduced backslashes from quote escaping don't get
    /// doubled themselves.
    public static func singleQuoted(_ s: String) -> String {
        var out = s.replacingOccurrences(of: "\\", with: "\\\\")
        out = out.replacingOccurrences(of: "'", with: "\\'")
        return out
    }
}
