import Combine
import XCTest
@testable import NotchCatKit

private final class FakeSource: NowPlayingSource {
    let id = "fake"
    let subject = CurrentValueSubject<NowPlayingSnapshot?, Never>(nil)
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { subject.eraseToAnyPublisher() }
}

/// Records every lookup and holds each one open until the test answers it.
private final class GatedResolver: LyricsResolving, @unchecked Sendable {
    private let lock = NSLock()
    private var tracks: [TrackIdentity] = []
    private var continuations: [CheckedContinuation<Lyrics?, Never>] = []

    var requested: [TrackIdentity] {
        lock.lock(); defer { lock.unlock() }
        return tracks
    }

    func lyrics(for track: TrackIdentity) async -> Lyrics? {
        await withCheckedContinuation { continuation in
            lock.lock()
            tracks.append(track)
            continuations.append(continuation)
            lock.unlock()
        }
    }

    /// Answers the `index`-th lookup (in arrival order).
    func respond(_ index: Int, with lyrics: Lyrics?) {
        lock.lock()
        let continuation = continuations[index]
        lock.unlock()
        continuation.resume(returning: lyrics)
    }
}

private final class FakeControl: MediaRemoteControlling {
    var sent: [MediaRemoteCommand] = []
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool {
        sent.append(command)
        return true
    }
}

private func snapshot(_ title: String, artist: String? = "Artist", isPlaying: Bool = true,
                      position: PositionAnchor? = nil) -> NowPlayingSnapshot {
    NowPlayingSnapshot(title: title, artist: artist, album: nil, duration: 200,
                       isPlaying: isPlaying, position: position, artwork: nil, sourceID: "fake")
}

final class MusicViewModelTests: XCTestCase {
    private var source: FakeSource!
    private var resolver: GatedResolver!
    private var control: FakeControl!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private var monitor: NowPlayingMonitor!

    override func setUp() {
        super.setUp()
        source = FakeSource()
        resolver = GatedResolver()
        control = FakeControl()
        suiteName = "MusicViewModelTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
        monitor = NowPlayingMonitor(sources: [source])
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        super.tearDown()
    }

    private func makeViewModel(control: MediaRemoteControlling? = nil) -> MusicViewModel {
        MusicViewModel(monitor: monitor, lyrics: resolver, control: control,
                       preference: LyricStripPreference(defaults: defaults))
    }

    /// Lookups and their results hop through other executors, so spin the
    /// main run loop until the condition holds (bounded).
    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: TimeInterval = 2,
                           file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() && Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.005))
        }
        XCTAssertTrue(condition(), "condition not met in time", file: file, line: line)
    }

    func testFetchesLyricsOnTrackChange() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))

        waitUntil(resolver.requested.count == 1)
        XCTAssertEqual(resolver.requested.first?.title, "Song A")
        XCTAssertEqual(resolver.requested.first?.artist, "Artist")
        XCTAssertEqual(viewModel.snapshot?.title, "Song A")

        resolver.respond(0, with: .plain("words"))
        waitUntil(viewModel.lyrics == .plain("words"))
    }

    func testDoesNotRefetchForSameTrack() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))
        waitUntil(resolver.requested.count == 1)
        resolver.respond(0, with: .plain("words"))
        waitUntil(viewModel.lyrics != nil)

        // Only the playing state and the position anchor change.
        source.subject.send(snapshot("Song A", isPlaying: false))
        source.subject.send(snapshot("Song A", isPlaying: true,
                                     position: PositionAnchor(elapsed: 30, timestamp: Date(), rate: 1)))
        XCTAssertEqual(viewModel.lyrics, .plain("words"))
        XCTAssertEqual(viewModel.snapshot?.isPlaying, true)

        // A later track change is the sentinel: had Song A been refetched, it
        // would have been recorded before Song B.
        source.subject.send(snapshot("Song B"))
        waitUntil(resolver.requested.count >= 2)
        XCTAssertEqual(resolver.requested.map(\.title), ["Song A", "Song B"])
    }

    func testStaleResponseIsDiscarded() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))
        waitUntil(resolver.requested.count == 1)
        source.subject.send(snapshot("Song B"))
        waitUntil(resolver.requested.count == 2)
        resolver.respond(1, with: .plain("lyrics of B"))
        waitUntil(viewModel.lyrics == .plain("lyrics of B"))

        // A's request finally comes back, after the track has moved on. There
        // is no event to wait for when a reply is (correctly) ignored, so give
        // the main queue a bounded window to deliver it, then check.
        resolver.respond(0, with: .plain("lyrics of A"))
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))

        XCTAssertEqual(viewModel.lyrics, .plain("lyrics of B"))
    }

    func testStaleResponseIsDiscardedWhileCurrentTrackIsStillLoading() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))
        waitUntil(resolver.requested.count == 1)
        source.subject.send(snapshot("Song B"))
        waitUntil(resolver.requested.count == 2)

        resolver.respond(0, with: .plain("lyrics of A"))
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))

        XCTAssertNil(viewModel.lyrics)
    }

    func testTrackChangeClearsPreviousLyrics() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))
        waitUntil(resolver.requested.count == 1)
        resolver.respond(0, with: .plain("lyrics of A"))
        waitUntil(viewModel.lyrics != nil)

        source.subject.send(snapshot("Song B"))

        XCTAssertNil(viewModel.lyrics)
    }

    func testMissingArtistSkipsLookup() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("No Artist Song", artist: nil))
        XCTAssertEqual(viewModel.snapshot?.title, "No Artist Song")
        XCTAssertNil(viewModel.lyrics)

        // Sentinel: the only lookup that reaches the resolver is the next one.
        source.subject.send(snapshot("With Artist"))
        waitUntil(resolver.requested.count >= 1)
        XCTAssertEqual(resolver.requested.map(\.title), ["With Artist"])
    }

    func testNoSnapshotClearsLyrics() {
        let viewModel = makeViewModel()
        source.subject.send(snapshot("Song A"))
        waitUntil(resolver.requested.count == 1)
        resolver.respond(0, with: .plain("lyrics of A"))
        waitUntil(viewModel.lyrics != nil)

        source.subject.send(nil)

        XCTAssertNil(viewModel.snapshot)
        XCTAssertNil(viewModel.lyrics)
    }

    func testSendForwardsToControl() {
        let viewModel = makeViewModel(control: control)

        viewModel.send(.next)
        viewModel.send(.togglePlayPause)

        XCTAssertEqual(control.sent, [.next, .togglePlayPause])
    }

    func testSendWithoutControlIsNoOp() {
        let viewModel = makeViewModel(control: nil)

        viewModel.send(.next)  // must not crash

        XCTAssertTrue(control.sent.isEmpty)
    }

    func testHasControlReflectsAvailability() {
        XCTAssertFalse(makeViewModel(control: nil).hasControl)
        XCTAssertTrue(makeViewModel(control: control).hasControl)
    }
}
