import Combine
import XCTest
@testable import NotchCatKit

private final class FakeScheduler {
    private(set) var delays: [TimeInterval] = []
    private var pending: [() -> Void] = []

    var schedule: (TimeInterval, @escaping () -> Void) -> Void {
        { [unowned self] delay, work in
            self.delays.append(delay)
            self.pending.append(work)
        }
    }

    func runPending() {
        let work = pending
        pending = []
        work.forEach { $0() }
    }
}

final class WindowTitleSourceTests: XCTestCase {
    private var windows: [WindowInfo] = []
    private var now = Date(timeIntervalSince1970: 1_000_000)
    private var scheduler: FakeScheduler!
    private var source: WindowTitleSource!
    private var published: [NowPlayingSnapshot?] = []
    private var cancellables: Set<AnyCancellable> = []

    override func setUp() {
        super.setUp()
        windows = []
        scheduler = FakeScheduler()
        published = []
        source = WindowTitleSource(
            windows: { [unowned self] in self.windows },
            now: { [unowned self] in self.now },
            schedule: scheduler.schedule)
        source.snapshots.sink { [unowned self] in self.published.append($0) }.store(in: &cancellables)
    }

    override func tearDown() {
        source.stop()
        cancellables = []
        super.tearDown()
    }

    private func ytm(_ title: String, playing: Bool = true) -> WindowInfo {
        WindowInfo(owner: "YouTube Music", title: "YouTube Music - \(title) | YouTube Music" + (playing ? " 🔊" : ""))
    }

    private func tick(advancing seconds: TimeInterval = 1) {
        now = now.addingTimeInterval(seconds)
        scheduler.runPending()
    }

    private var latest: NowPlayingSnapshot? { published.last ?? nil }

    func testPublishesSnapshotForMatchingWindow() {
        windows = [ytm("假日而已")]
        source.start()
        let snapshot = latest
        XCTAssertEqual(snapshot?.title, "假日而已")
        XCTAssertNil(snapshot?.artist)
        XCTAssertNil(snapshot?.album)
        XCTAssertNil(snapshot?.duration)
        XCTAssertNil(snapshot?.artwork)
        XCTAssertEqual(snapshot?.isPlaying, true)
        XCTAssertEqual(snapshot?.sourceID, source.id)
    }

    func testPublishesNilWhenNoWindowMatches() {
        windows = [WindowInfo(owner: "Finder", title: "Documents")]
        source.start()
        XCTAssertEqual(published.count, 1)
        XCTAssertNil(latest)
    }

    func testIgnoresNonMatchingOwner() {
        windows = [WindowInfo(owner: "Safari", title: "YouTube Music - 假日而已 | YouTube Music 🔊")]
        source.start()
        XCTAssertNil(latest)
    }

    func testIgnoresUnparsableTitleOfMatchingOwner() {
        windows = [WindowInfo(owner: "YouTube Music", title: "YouTube Music")]
        source.start()
        XCTAssertNil(latest)
    }

    func testPrefersPlayingWindowOverPausedOne() {
        windows = [ytm("A", playing: false), ytm("B", playing: true)]
        source.start()
        XCTAssertEqual(latest?.title, "B")
    }

    func testSnapshotCarriesAnchorOnlyAfterObservedChange() {
        windows = [ytm("A")]
        source.start()
        XCTAssertEqual(latest?.title, "A")
        XCTAssertNil(latest?.position)
        tick()
        XCTAssertNil(latest?.position)

        windows = [ytm("B")]
        tick()
        XCTAssertEqual(latest?.title, "B")
        XCTAssertEqual(latest?.position, PositionAnchor(elapsed: 0, timestamp: now, rate: 1))
    }

    func testPauseFreezesAnchorInSnapshot() {
        windows = [ytm("A")]
        source.start()
        windows = [ytm("B")]
        tick()
        windows = [ytm("B", playing: false)]
        tick(advancing: 4)
        XCTAssertEqual(latest?.isPlaying, false)
        XCTAssertEqual(latest?.position, PositionAnchor(elapsed: 4, timestamp: now, rate: 0))
    }

    func testPollsEverySecondWhilePlayingAndEveryFiveWhenNot() {
        windows = [ytm("A")]
        source.start()
        tick()
        windows = [ytm("A", playing: false)]
        tick()
        windows = []
        tick()
        XCTAssertEqual(scheduler.delays, [1, 1, 5, 5])
    }

    func testDoesNotRepublishUnchangedSnapshot() {
        windows = [ytm("A")]
        source.start()
        tick()
        tick()
        XCTAssertEqual(published.count, 2, "the initial nil, then the one snapshot")
    }

    func testStopPublishesNilAndCancelsPolling() {
        windows = [ytm("A")]
        source.start()
        source.stop()
        XCTAssertNil(latest)
        let count = published.count
        tick()
        XCTAssertEqual(published.count, count)
    }

    func testRestartAfterStopForgetsEstimatorState() {
        windows = [ytm("A")]
        source.start()
        windows = [ytm("B")]
        tick()
        XCTAssertNotNil(latest?.position)
        source.stop()
        source.start()
        XCTAssertEqual(latest?.title, "B")
        XCTAssertNil(latest?.position)
    }
}
