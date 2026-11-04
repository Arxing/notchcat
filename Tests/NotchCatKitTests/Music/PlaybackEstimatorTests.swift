import XCTest
@testable import NotchCatKit

final class PlaybackEstimatorTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_000_000)

    func testFirstObservationHasNoAnchor() {
        var estimator = PlaybackEstimator()
        XCTAssertNil(estimator.anchor())
        estimator.observe(title: "A", isPlaying: true, at: t0)
        XCTAssertNil(estimator.anchor())
        estimator.observe(title: "A", isPlaying: true, at: t0.addingTimeInterval(30))
        XCTAssertNil(estimator.anchor())
    }

    func testTrackChangeWhileRunningYieldsAnchorAtZero() {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        let change = t0.addingTimeInterval(40)
        estimator.observe(title: "B", isPlaying: true, at: change)
        XCTAssertEqual(estimator.anchor(), PositionAnchor(elapsed: 0, timestamp: change, rate: 1))
        XCTAssertEqual(estimator.anchor()?.position(at: change.addingTimeInterval(7), cappedTo: nil), 7)
    }

    func testPauseFreezesPosition() throws {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        let change = t0.addingTimeInterval(10)
        estimator.observe(title: "B", isPlaying: true, at: change)
        let pause = change.addingTimeInterval(25)
        estimator.observe(title: "B", isPlaying: false, at: pause)
        let anchor = try XCTUnwrap(estimator.anchor())
        XCTAssertEqual(anchor, PositionAnchor(elapsed: 25, timestamp: pause, rate: 0))
        XCTAssertEqual(anchor.position(at: pause.addingTimeInterval(10), cappedTo: nil), 25)
    }

    func testResumeContinuesFromFrozenPosition() throws {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        let change = t0.addingTimeInterval(10)
        estimator.observe(title: "B", isPlaying: true, at: change)
        let pause = change.addingTimeInterval(25)
        estimator.observe(title: "B", isPlaying: false, at: pause)
        let resume = pause.addingTimeInterval(60)
        estimator.observe(title: "B", isPlaying: true, at: resume)
        let anchor = try XCTUnwrap(estimator.anchor())
        XCTAssertEqual(anchor, PositionAnchor(elapsed: 25, timestamp: resume, rate: 1))
        XCTAssertEqual(anchor.position(at: resume.addingTimeInterval(5), cappedTo: nil), 30)
    }

    func testSameTrackReobservedKeepsAnchor() {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        let change = t0.addingTimeInterval(10)
        estimator.observe(title: "B", isPlaying: true, at: change)
        let before = estimator.anchor()
        estimator.observe(title: "B", isPlaying: true, at: change.addingTimeInterval(1))
        estimator.observe(title: "B", isPlaying: true, at: change.addingTimeInterval(2))
        XCTAssertEqual(estimator.anchor(), before)
    }

    func testUnknownStartStaysUnknownAcrossPauseAndResume() {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        estimator.observe(title: "A", isPlaying: false, at: t0.addingTimeInterval(5))
        XCTAssertNil(estimator.anchor())
        estimator.observe(title: "A", isPlaying: true, at: t0.addingTimeInterval(9))
        XCTAssertNil(estimator.anchor())
    }

    func testNoTrackClearsAnchorAndNextTrackStartsUnknown() {
        var estimator = PlaybackEstimator()
        estimator.observe(title: "A", isPlaying: true, at: t0)
        estimator.observe(title: "B", isPlaying: true, at: t0.addingTimeInterval(10))
        XCTAssertNotNil(estimator.anchor())
        estimator.observe(title: nil, isPlaying: false, at: t0.addingTimeInterval(20))
        XCTAssertNil(estimator.anchor())
        estimator.observe(title: "C", isPlaying: true, at: t0.addingTimeInterval(30))
        XCTAssertNil(estimator.anchor())
    }
}
