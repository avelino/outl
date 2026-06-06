import XCTest
@testable import OutlKit

final class JSStringEscapeTests: XCTestCase {

    func testNoSpecialCharsPassesThrough() {
        XCTAssertEqual(JSStringEscape.singleQuoted("hello"), "hello")
        XCTAssertEqual(JSStringEscape.singleQuoted(""), "")
    }

    func testEscapesSingleQuote() {
        XCTAssertEqual(
            JSStringEscape.singleQuoted("it's"),
            "it\\'s"
        )
    }

    func testEscapesBackslash() {
        XCTAssertEqual(
            JSStringEscape.singleQuoted("a\\b"),
            "a\\\\b"
        )
    }

    func testEscapesBackslashBeforeQuote() {
        // Order-sensitive: the backslash that escapes the quote must
        // NOT itself get doubled. We do `\\` → `\\\\` first, then
        // `'` → `\'`, so the final output for `a\'b` is `a\\\\\'b`
        // (4 backslashes, escaped quote).
        XCTAssertEqual(
            JSStringEscape.singleQuoted("a\\'b"),
            "a\\\\\\'b"
        )
    }

    func testEscapesMultipleOccurrences() {
        XCTAssertEqual(
            JSStringEscape.singleQuoted("'a'b'c'"),
            "\\'a\\'b\\'c\\'"
        )
    }

    func testRealWorldAction() {
        // Sanity: a typical action string ships unchanged because it
        // never contains special chars.
        XCTAssertEqual(JSStringEscape.singleQuoted("indent"), "indent")
        XCTAssertEqual(JSStringEscape.singleQuoted("toggleTodo"), "toggleTodo")
    }
}
