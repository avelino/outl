import XCTest
@testable import OutlKit

final class SuggesterStateTests: XCTestCase {

    func testParsesShowWithItems() {
        let json = """
        {
          "action": "show",
          "items": [
            {"title": "Today", "slug": "2026-06-06", "kind": "journal"},
            {"title": "outl", "slug": "outl"}
          ]
        }
        """
        let state = SuggesterState.parse(jsonString: json)
        XCTAssertEqual(state.action, .show)
        XCTAssertEqual(state.items.count, 2)
        XCTAssertEqual(state.items.first?.kind, .journal)
        XCTAssertEqual(state.items.last?.kind, .page)
    }

    func testParsesHide() {
        let json = #"{"action":"hide","items":[]}"#
        XCTAssertEqual(
            SuggesterState.parse(jsonString: json),
            SuggesterState.hidden
        )
    }

    func testShowWithEmptyItemsCollapsesToHidden() {
        // Rendering a "show" state with zero chips is just an empty
        // strip on screen — collapse to hidden so the overlay slides
        // out rather than showing an empty pill.
        let json = #"{"action":"show","items":[]}"#
        XCTAssertEqual(
            SuggesterState.parse(jsonString: json),
            SuggesterState.hidden
        )
    }

    func testMalformedJsonReturnsHidden() {
        XCTAssertEqual(
            SuggesterState.parse(jsonString: "{not json"),
            SuggesterState.hidden
        )
    }

    func testNilReturnsHidden() {
        XCTAssertEqual(SuggesterState.parse(jsonString: nil), .hidden)
    }

    func testEmptyStringReturnsHidden() {
        XCTAssertEqual(SuggesterState.parse(jsonString: ""), .hidden)
    }

    func testStringNullReturnsHidden() {
        // WKWebView.evaluateJavaScript returns the literal string
        // "null" when the JS expression evaluates to `null`. Treat it
        // the same as "no payload yet".
        XCTAssertEqual(SuggesterState.parse(jsonString: "null"), .hidden)
    }

    func testStringUndefinedReturnsHidden() {
        XCTAssertEqual(
            SuggesterState.parse(jsonString: "undefined"),
            .hidden
        )
    }

    func testUnknownActionDefaultsToHide() {
        // Anything that isn't literally "show" is treated as hide —
        // safer to under-render than to surface a strip the user
        // can't dismiss.
        let json = #"{"action":"flash","items":[{"slug":"x"}]}"#
        XCTAssertEqual(
            SuggesterState.parse(jsonString: json).action,
            .hide
        )
    }

    /// Regression: emoji chips used to fall back to `.page` here,
    /// which made the tap fire `__outlSuggesterPicked(shortcode, "page")`
    /// — the JS side dispatches on the second argument, so the emoji
    /// branch never ran and the chip rendered but did nothing.
    func testParsesEmojiKind() {
        let json = """
        {
          "action": "show",
          "items": [
            {"title": "🎉", "slug": "tada", "kind": "emoji"},
            {"title": "🚀", "slug": "rocket", "kind": "emoji"}
          ]
        }
        """
        let state = SuggesterState.parse(jsonString: json)
        XCTAssertEqual(state.action, .show)
        XCTAssertEqual(state.items.count, 2)
        XCTAssertEqual(state.items.first?.kind, .emoji)
        XCTAssertEqual(state.items.first?.slug, "tada")
        // `rawValue` is what gets shipped back to JS — must stay "emoji"
        // verbatim so `kind === "emoji"` matches in the picked callback.
        XCTAssertEqual(state.items.first?.kind.rawValue, "emoji")
    }
}
