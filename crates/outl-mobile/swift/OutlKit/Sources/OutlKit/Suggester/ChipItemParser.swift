import Foundation

/// Parses the items array shipped by the JS suggester state.
///
/// The JS side puts the payload at `window.__outlSuggesterState`,
/// which `OutlSuggestOverlay` polls and decodes into `[[String: Any]]`.
/// This parser turns that loose dictionary array into typed
/// `ChipItem`s, **silently dropping entries without a slug** (a chip
/// without slug can't fire the pick callback, so rendering it would
/// be a dead element).
///
/// Missing `title` falls back to the slug — the chip strip stays
/// usable even if the JS side forgot to populate the title.
public enum ChipItemParser {
    public static func parse(_ raw: [[String: Any]]) -> [ChipItem] {
        var out: [ChipItem] = []
        out.reserveCapacity(raw.count)
        for entry in raw {
            guard let slug = entry["slug"] as? String, !slug.isEmpty else {
                continue
            }
            let title = (entry["title"] as? String) ?? slug
            let kind = ChipItem.Kind.from(entry["kind"] as? String)
            out.append(ChipItem(title: title, slug: slug, kind: kind))
        }
        return out
    }
}
