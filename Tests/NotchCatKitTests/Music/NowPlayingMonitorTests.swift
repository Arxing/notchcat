import Combine
import XCTest
@testable import NotchCatKit

private final class FakeSource: NowPlayingSource {
    let id: String
    let subject = CurrentValueSubject<NowPlayingSnapshot?, Never>(nil)
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { subject.eraseToAnyPublisher() }
    init(id: String) { self.id = id }
}

/// A source that has never emitted, not even an initial nil.
private final class SilentSource: NowPlayingSource {
    let id = "silent"
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { Empty().eraseToAnyPublisher() }
}

private func snapshot(_ title: String, source: String, isPlaying: Bool = true,
                      artwork: ArtworkRef? = nil) -> NowPlayingSnapshot {
    NowPlayingSnapshot(title: title, artist: nil, album: nil, duration: nil,
                       isPlaying: isPlaying, position: nil, artwork: artwork, sourceID: source)
}

final class NowPlayingMonitorTests: XCTestCase {
    private var primary: FakeSource!
    private var fallback: FakeSource!
    private var cancellables: Set<AnyCancellable> = []

    override func setUp() {
        super.setUp()
        primary = FakeSource(id: "primary")
        fallback = FakeSource(id: "fallback")
        cancellables = []
    }

    func testFirstNonNilSourceWins() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        fallback.subject.send(snapshot("Fallback", source: "fallback"))
        primary.subject.send(snapshot("Primary", source: "primary"))

        XCTAssertEqual(monitor.current?.title, "Primary")
        XCTAssertEqual(monitor.current?.sourceID, "primary")
    }

    func testFallsThroughToLaterSourceWhenFirstIsNil() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        primary.subject.send(snapshot("Primary", source: "primary"))
        fallback.subject.send(snapshot("Fallback", source: "fallback"))
        XCTAssertEqual(monitor.current?.title, "Primary")

        primary.subject.send(nil)

        XCTAssertEqual(monitor.current?.title, "Fallback")
    }

    func testFirstSourceRegainingDataTakesBackOver() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        fallback.subject.send(snapshot("Fallback", source: "fallback"))
        XCTAssertEqual(monitor.current?.title, "Fallback")

        primary.subject.send(snapshot("Primary", source: "primary"))

        XCTAssertEqual(monitor.current?.title, "Primary")
    }

    func testNeverEmittedSourceCountsAsNil() {
        let monitor = NowPlayingMonitor(sources: [SilentSource(), fallback])
        XCTAssertNil(monitor.current)

        fallback.subject.send(snapshot("Fallback", source: "fallback"))

        XCTAssertEqual(monitor.current?.title, "Fallback")
    }

    func testStartsWithInitialValuesOfSources() {
        primary.subject.send(snapshot("Already", source: "primary"))

        let monitor = NowPlayingMonitor(sources: [primary, fallback])

        XCTAssertEqual(monitor.current?.title, "Already")
    }

    func testNoSourcesYieldsNil() {
        XCTAssertNil(NowPlayingMonitor(sources: []).current)
    }

    func testPausedSnapshotStillWinsOverLaterSource() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        fallback.subject.send(snapshot("Fallback", source: "fallback", isPlaying: true))
        primary.subject.send(snapshot("Paused", source: "primary", isPlaying: false))

        XCTAssertEqual(monitor.current?.title, "Paused")
        XCTAssertEqual(monitor.current?.isPlaying, false)
    }

    func testIsPlayingPublisherDeduplicates() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        var values: [Bool] = []
        monitor.isPlayingPublisher.sink { values.append($0) }.store(in: &cancellables)

        primary.subject.send(snapshot("One", source: "primary", isPlaying: true))
        primary.subject.send(snapshot("Two", source: "primary", isPlaying: true))
        primary.subject.send(snapshot("Two", source: "primary", isPlaying: false))
        primary.subject.send(snapshot("Three", source: "primary", isPlaying: false))
        primary.subject.send(nil)
        primary.subject.send(snapshot("Four", source: "primary", isPlaying: true))

        XCTAssertEqual(values, [false, true, false, true])
    }

    func testArtworkBytesAreFilledInFromEarlierSnapshotWithSameIdentifier() {
        let bytes = Data([1, 2, 3])
        let monitor = NowPlayingMonitor(sources: [primary])

        primary.subject.send(snapshot("Song", source: "primary",
                                      artwork: ArtworkRef(identifier: "art", data: bytes)))
        XCTAssertEqual(monitor.current?.artwork, ArtworkRef(identifier: "art", data: bytes))

        primary.subject.send(snapshot("Song", source: "primary", isPlaying: false,
                                      artwork: ArtworkRef(identifier: "art", data: nil)))
        XCTAssertEqual(monitor.current?.artwork, ArtworkRef(identifier: "art", data: bytes))
    }

    func testArtworkWithUnknownIdentifierStaysWithoutData() {
        let monitor = NowPlayingMonitor(sources: [primary])

        primary.subject.send(snapshot("Song", source: "primary",
                                      artwork: ArtworkRef(identifier: "never-seen", data: nil)))

        XCTAssertEqual(monitor.current?.artwork, ArtworkRef(identifier: "never-seen", data: nil))
    }

    func testUsesInjectedArtworkCache() {
        let cache = ArtworkCache()
        _ = cache.resolve(ArtworkRef(identifier: "art", data: Data([9])))
        let monitor = NowPlayingMonitor(sources: [primary], artwork: cache)

        primary.subject.send(snapshot("Song", source: "primary",
                                      artwork: ArtworkRef(identifier: "art", data: nil)))

        XCTAssertEqual(monitor.current?.artwork?.data, Data([9]))
    }

    func testCurrentIsNotRepublishedForAnEqualSnapshot() {
        let monitor = NowPlayingMonitor(sources: [primary, fallback])
        var changes = 0
        monitor.objectWillChange.sink { changes += 1 }.store(in: &cancellables)

        primary.subject.send(snapshot("Song", source: "primary"))
        primary.subject.send(snapshot("Song", source: "primary"))
        fallback.subject.send(snapshot("Other", source: "fallback"))

        XCTAssertEqual(changes, 1)
    }
}
