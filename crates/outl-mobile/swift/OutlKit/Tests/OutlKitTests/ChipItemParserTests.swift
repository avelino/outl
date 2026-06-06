import XCTest
@testable import OutlKit

final class ChipItemParserTests: XCTestCase {

    func testParsesNormalShape() {
        let raw: [[String: Any]] = [
            ["title": "avelino", "slug": "avelino", "kind": "page"],
            ["title": "Today", "slug": "2026-06-06", "kind": "journal"],
        ]
        let items = ChipItemParser.parse(raw)
        XCTAssertEqual(items, [
            ChipItem(title: "avelino", slug: "avelino", kind: .page),
            ChipItem(title: "Today", slug: "2026-06-06", kind: .journal),
        ])
    }

    func testMissingKindFallsBackToPage() {
        let items = ChipItemParser.parse([["title": "x", "slug": "x"]])
        XCTAssertEqual(items.first?.kind, .page)
    }

    func testUnknownKindFallsBackToPage() {
        let items = ChipItemParser.parse([
            ["title": "x", "slug": "x", "kind": "wat"],
        ])
        XCTAssertEqual(items.first?.kind, .page)
    }

    func testMissingSlugIsDropped() {
        // A chip without slug can't fire the pick callback, so the
        // parser refuses to surface it.
        let items = ChipItemParser.parse([
            ["title": "no-slug"],
            ["title": "valid", "slug": "valid"],
        ])
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.slug, "valid")
    }

    func testEmptySlugIsDropped() {
        let items = ChipItemParser.parse([
            ["title": "x", "slug": ""],
        ])
        XCTAssertTrue(items.isEmpty)
    }

    func testMissingTitleFallsBackToSlug() {
        let items = ChipItemParser.parse([["slug": "lonely"]])
        XCTAssertEqual(items.first?.title, "lonely")
        XCTAssertEqual(items.first?.slug, "lonely")
    }

    func testEmptyArrayReturnsEmpty() {
        XCTAssertTrue(ChipItemParser.parse([]).isEmpty)
    }

    func testKindIsCaseInsensitiveAndTrimmed() {
        let items = ChipItemParser.parse([
            ["slug": "a", "kind": "  JOURNAL  "],
        ])
        XCTAssertEqual(items.first?.kind, .journal)
    }
}
