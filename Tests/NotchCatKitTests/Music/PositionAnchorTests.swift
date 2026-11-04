import XCTest
@testable import NotchCatKit

final class PositionAnchorTests: XCTestCase {
    private let t = Date(timeIntervalSinceReferenceDate: 1_000)

    private func anchor(elapsed: TimeInterval, rate: Double = 1) -> PositionAnchor {
        PositionAnchor(elapsed: elapsed, timestamp: t, rate: rate)
    }

    func testAdvancesWithRate() {
        let p = anchor(elapsed: 10).position(at: t.addingTimeInterval(5), cappedTo: nil)
        XCTAssertEqual(p, 15, accuracy: 0.0001)
    }

    func testHonorsNonUnitRate() {
        let p = anchor(elapsed: 10, rate: 2).position(at: t.addingTimeInterval(5), cappedTo: nil)
        XCTAssertEqual(p, 20, accuracy: 0.0001)
    }

    func testPausedFreezes() {
        let p = anchor(elapsed: 10, rate: 0).position(at: t.addingTimeInterval(100), cappedTo: nil)
        XCTAssertEqual(p, 10, accuracy: 0.0001)
    }

    func testCapsAtDuration() {
        let p = anchor(elapsed: 230).position(at: t.addingTimeInterval(100), cappedTo: 233.9)
        XCTAssertEqual(p, 233.9, accuracy: 0.0001)
    }

    func testNoDurationMeansNoCap() {
        let p = anchor(elapsed: 230).position(at: t.addingTimeInterval(100), cappedTo: nil)
        XCTAssertEqual(p, 330, accuracy: 0.0001)
    }

    func testTimestampInFutureDoesNotRewind() {
        let p = anchor(elapsed: 10).position(at: t.addingTimeInterval(-50), cappedTo: nil)
        XCTAssertEqual(p, 10, accuracy: 0.0001)
        XCTAssertGreaterThanOrEqual(p, 0)
    }

    func testNegativeElapsedClampsToZero() {
        let p = anchor(elapsed: -5).position(at: t, cappedTo: nil)
        XCTAssertEqual(p, 0, accuracy: 0.0001)
    }
}
