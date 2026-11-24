import XCTest
@testable import NotchCatKit

final class MarqueeTextTests: XCTestCase {
    func testShortTextDoesNotScroll() {
        XCTAssertFalse(MarqueeLayout.needsScroll(textWidth: 80, containerWidth: 120))
    }

    func testLongTextScrolls() {
        XCTAssertTrue(MarqueeLayout.needsScroll(textWidth: 300, containerWidth: 120))
    }

    func testExactFitDoesNotScroll() {
        XCTAssertFalse(MarqueeLayout.needsScroll(textWidth: 120, containerWidth: 120))
    }

    func testZeroWidthContainerDoesNotCrash() {
        // Before layout the container has no width yet; scrolling then would
        // only churn, so it must stay still.
        XCTAssertFalse(MarqueeLayout.needsScroll(textWidth: 120, containerWidth: 0))
        XCTAssertFalse(MarqueeLayout.needsScroll(textWidth: 0, containerWidth: 0))
    }
}
