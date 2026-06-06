import XCTest
@testable import OutlKit

final class ToolbarMFUTests: XCTestCase {

    /// Each test runs against a fresh, named `UserDefaults` suite so
    /// we never touch the developer's real device prefs and tests
    /// don't bleed into one another.
    private var defaults: UserDefaults!
    private var suiteName: String!

    override func setUp() {
        super.setUp()
        suiteName = "OutlKitTests.ToolbarMFU.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        suiteName = nil
        super.tearDown()
    }

    // MARK: - Pure ordering

    func testReturnsDefaultOrderWithEmptyCounts() {
        XCTAssertEqual(
            ToolbarMFU.orderedActions(counts: [:]),
            ToolbarAction.defaultOrder
        )
    }

    func testPinnedFirstStaysAtIndexZero() {
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 9999, "code": 9999]
        )
        XCTAssertEqual(order.first, ToolbarAction.pinnedFirst)
    }

    func testPinnedLastStaysAtFinalIndex() {
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 9999, "code": 9999]
        )
        XCTAssertEqual(order.last, ToolbarAction.pinnedLast)
    }

    func testMostUsedHoistsToPositionRightAfterPinnedFirst() {
        let order = ToolbarMFU.orderedActions(
            counts: ["code": 10, "italic": 1]
        )
        XCTAssertEqual(order[0], .newLine)
        XCTAssertEqual(order[1], .code)
    }

    func testStableTiebreakUsesDefaultOrderIndex() {
        // bold + italic both at 5; defaultOrder has bold ahead of
        // italic, so the tie has to resolve bold-first.
        let order = ToolbarMFU.orderedActions(
            counts: ["italic": 5, "bold": 5]
        )
        let boldIdx = order.firstIndex(of: .bold)!
        let italicIdx = order.firstIndex(of: .italic)!
        XCTAssertLessThan(boldIdx, italicIdx)
    }

    func testIgnoresCountsAgainstPinnedActions() {
        // Even if storage somehow contains counts for the pinned
        // slots, the slot position wins by virtue of the algorithm.
        let order = ToolbarMFU.orderedActions(counts: [
            "newLine": 9999,
            "done": 9999,
            "code": 1,
        ])
        XCTAssertEqual(order.first, .newLine)
        XCTAssertEqual(order.last, .done)
    }

    func testCardinalityPreserved() {
        let order = ToolbarMFU.orderedActions(counts: ["code": 10])
        XCTAssertEqual(order.count, ToolbarAction.defaultOrder.count)
        XCTAssertEqual(Set(order).count, ToolbarAction.defaultOrder.count)
    }

    func testEveryCaseAppearsInDefaultOrder() {
        // Catches the "added a new case to ToolbarAction but forgot
        // defaultOrder" footgun.
        let inDefault = Set(ToolbarAction.defaultOrder)
        for action in ToolbarAction.allCases {
            XCTAssertTrue(
                inDefault.contains(action),
                "ToolbarAction.defaultOrder is missing \(action.rawValue)"
            )
        }
    }

    // MARK: - Persistence

    func testRecordIncrementsCount() {
        ToolbarMFU.record(.code, defaults: defaults)
        ToolbarMFU.record(.code, defaults: defaults)
        let counts = ToolbarMFU.readCounts(defaults: defaults)
        XCTAssertEqual(counts["code"], 2)
    }

    func testRecordIsNoOpForPinnedFirstAndLast() {
        ToolbarMFU.record(.newLine, defaults: defaults)
        ToolbarMFU.record(.done, defaults: defaults)
        let counts = ToolbarMFU.readCounts(defaults: defaults)
        XCTAssertNil(counts["newLine"])
        XCTAssertNil(counts["done"])
    }

    func testClearCountsRemovesEntry() {
        ToolbarMFU.record(.bold, defaults: defaults)
        XCTAssertEqual(ToolbarMFU.readCounts(defaults: defaults)["bold"], 1)
        ToolbarMFU.clearCounts(defaults: defaults)
        XCTAssertTrue(ToolbarMFU.readCounts(defaults: defaults).isEmpty)
    }

    func testOrderedActionsConvenienceReadsFromDefaults() {
        ToolbarMFU.record(.italic, defaults: defaults)
        ToolbarMFU.record(.italic, defaults: defaults)
        ToolbarMFU.record(.code, defaults: defaults)
        let order = ToolbarMFU.orderedActions(defaults: defaults)
        // italic (2) > code (1) > everything else (0), so italic must
        // land at index 1 (right after the pinned newLine).
        XCTAssertEqual(order[0], .newLine)
        XCTAssertEqual(order[1], .italic)
        XCTAssertEqual(order.last, .done)
    }
}
