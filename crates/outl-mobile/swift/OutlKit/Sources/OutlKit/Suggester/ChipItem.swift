import Foundation

/// One entry in the ref-autocomplete chip strip.
///
/// `title` is what the user sees; `slug` is the canonical id sent
/// back to JS via `window.__outlSuggesterPicked(slug, kind)`; `kind`
/// distinguishes pages from journals so the frontend can route
/// correctly.
public struct ChipItem: Equatable, Sendable {
    public let title: String
    public let slug: String
    public let kind: Kind

    /// Mirrors the JS side: `"page"` / `"journal"` / `"emoji"`.
    /// Anything else (or missing) falls back to `.page` to keep
    /// ingestion lenient — the parser is the boundary, so we don't
    /// want a typo to drop the whole chip.
    ///
    /// **Why `.emoji` must be its own case:** the chip's `rawValue`
    /// is sent back to JS verbatim via `window.__outlSuggesterPicked(slug, kind)`,
    /// and the JS side dispatches on it (`kind === "emoji"` →
    /// `applyEmojiSuggestion`). If we collapsed `"emoji"` into
    /// `.fallback = .page`, the tap would fire `…Picked(shortcode, "page")`
    /// and the JS branch would never match, leaving the user with a
    /// chip that renders but does nothing.
    public enum Kind: String, Sendable, Equatable {
        case page
        case journal
        case emoji

        public static let fallback: Kind = .page

        /// Resolve a Kind from a raw string (case-insensitive,
        /// trimmed). Returns `.fallback` for missing / unknown values.
        public static func from(_ raw: String?) -> Kind {
            guard let raw else { return .fallback }
            let normalized = raw.trimmingCharacters(in: .whitespaces).lowercased()
            return Kind(rawValue: normalized) ?? .fallback
        }
    }

    public init(title: String, slug: String, kind: Kind) {
        self.title = title
        self.slug = slug
        self.kind = kind
    }
}
