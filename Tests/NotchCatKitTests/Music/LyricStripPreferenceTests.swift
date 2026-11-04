import XCTest
@testable import NotchCatKit

final class LyricStripPreferenceTests: XCTestCase {
    private var suiteName: String!
    private var defaults: UserDefaults!

    override func setUp() {
        super.setUp()
        suiteName = "LyricStripPreferenceTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        super.tearDown()
    }

    func testDefaultsToVisible() {
        XCTAssertTrue(LyricStripPreference(defaults: defaults).isVisible)
    }

    func testPersistsAcrossInstances() {
        let first = LyricStripPreference(defaults: defaults)
        first.isVisible = false

        XCTAssertFalse(LyricStripPreference(defaults: defaults).isVisible)

        first.isVisible = true
        XCTAssertTrue(LyricStripPreference(defaults: defaults).isVisible)
    }
}
