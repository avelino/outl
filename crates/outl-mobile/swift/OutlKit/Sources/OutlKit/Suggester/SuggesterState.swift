import Foundation

/// Decoded form of the payload at `window.__outlSuggesterState`.
///
/// The JS side writes a JSON object shaped like:
///
///     { "action": "show" | "hide", "items": [{ "title", "slug", "kind" }] }
///
/// `OutlSuggestOverlay` polls that string every ~150ms; the parse step
/// has to be **forgiving** — a partial write, a missing field, or
/// stale `"undefined"` should evaluate to "hide", not to a crash.
public struct SuggesterState: Equatable, Sendable {
    public enum Action: String, Sendable {
        case show
        case hide
    }

    public let action: Action
    public let items: [ChipItem]

    public init(action: Action, items: [ChipItem]) {
        self.action = action
        self.items = items
    }

    /// The "nothing to show" sentinel — overlay treats this as the
    /// signal to invalidate its content size and slide off-screen.
    public static let hidden = SuggesterState(action: .hide, items: [])

    /// Parse the JSON string the WebView returned. Returns
    /// `.hidden` (not nil) for any malformed / empty / non-dict
    /// payload so callers don't have to branch on optionals.
    public static func parse(jsonString raw: String?) -> SuggesterState {
        guard let raw, !raw.isEmpty, raw != "null", raw != "undefined" else {
            return .hidden
        }
        guard
            let data = raw.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data),
            let dict = object as? [String: Any]
        else {
            return .hidden
        }
        let action = (dict["action"] as? String) == "show" ? Action.show : Action.hide
        let rawItems = (dict["items"] as? [[String: Any]]) ?? []
        let items = ChipItemParser.parse(rawItems)
        // A "show" with zero items is effectively "hide" — collapse so
        // the overlay doesn't render an empty bar.
        if action == .show, items.isEmpty {
            return .hidden
        }
        return SuggesterState(action: action, items: items)
    }
}
