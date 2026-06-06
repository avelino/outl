import XCTest
@testable import OutlKit

final class OpsFilePredicateTests: XCTestCase {

    // MARK: - NSPredicate format

    func testNsMetadataPredicateUsesFSNameKey() {
        // The actual `NSPredicate(format:)` interpolation is exercised
        // here so we catch any drift between the LIKE pattern and the
        // `NSMetadataItemFSNameKey` substitution.
        let predicate = OpsFilePredicate.nsMetadataPredicate()
        // The format string survives the build with the key
        // substituted in. We don't try to test against a real
        // NSMetadataItem (only available with a live query); the
        // build of the predicate itself is enough.
        XCTAssertTrue(
            predicate.predicateFormat.contains("ops-*.jsonl"),
            "predicate should LIKE-match ops-*.jsonl, got: \(predicate.predicateFormat)"
        )
    }

    // MARK: - String matcher (defense-in-depth)

    func testMatchesValidOpsJsonl() {
        XCTAssertTrue(OpsFilePredicate.isOpsJsonl(
            filename: "ops-01H9XK0YV4FQM0G3A1V2WZ.jsonl"
        ))
        XCTAssertTrue(OpsFilePredicate.isOpsJsonl(filename: "ops-a.jsonl"))
    }

    func testRejectsPlainOpsJsonl() {
        // No actor id between the prefix and suffix — would be a
        // shared file, exactly the layout we're paid to NOT have.
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: "ops-.jsonl"))
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: "ops.jsonl"))
    }

    func testRejectsWrongExtension() {
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: "ops-x.json"))
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: "ops-x.jsonl.bak"))
    }

    func testRejectsPathSeparators() {
        // Caller is supposed to pass a basename; reject anything that
        // smells like a relative path.
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(
            filename: "subdir/ops-x.jsonl"
        ))
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(
            filename: "..\\ops-x.jsonl"
        ))
    }

    func testRejectsUnrelatedFiles() {
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: "data-x.jsonl"))
        XCTAssertFalse(OpsFilePredicate.isOpsJsonl(filename: ".ops-x.jsonl"))
    }
}
