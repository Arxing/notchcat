import Combine
import XCTest
@testable import NotchCatKit

private final class FakeSource: NowPlayingSource {
    let id = "fake"
    let subject = CurrentValueSubject<NowPlayingSnapshot?, Never>(nil)
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { subject.eraseToAnyPublisher() }
}

private struct NoLyrics: LyricsResolving {
    func lyrics(for track: TrackIdentity) async -> Lyrics? { nil }
}

final class MusicModuleTests: XCTestCase {
    private var source: FakeSource!
    private var preference: LyricStripPreference!
    private var module: MusicModule!
    private var defaults: UserDefaults!
    private var suiteName: String!
    private var heights: [CGFloat] = []
    private var cancellables: Set<AnyCancellable> = []

    override func setUp() {
        super.setUp()
        suiteName = "MusicModuleTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
        source = FakeSource()
        preference = LyricStripPreference(defaults: defaults)
        module = MusicModule(
            monitor: NowPlayingMonitor(sources: [source]), lyrics: NoLyrics(),
            control: nil, preference: preference, notchWidth: 200)
        heights = []
        cancellables = []
        module.previewExtraHeightPublisher
            .sink { [unowned self] in heights.append($0) }
            .store(in: &cancellables)
    }

    override func tearDown() {
        cancellables = []
        defaults.removePersistentDomain(forName: suiteName)
        super.tearDown()
    }

    private func play(_ isPlaying: Bool = true) {
        source.subject.send(NowPlayingSnapshot(
            title: "Song", artist: nil, album: nil, duration: nil, isPlaying: isPlaying,
            position: nil, artwork: nil, sourceID: "fake"))
    }

    func testConstants() {
        XCTAssertEqual(MusicModule.lyricStripHeight, 24)
        XCTAssertEqual(MusicModule.lyricStripOpacity, 0.6)
        XCTAssertEqual(module.id, "com.notchcat.music")
        XCTAssertEqual(module.priority, 100)
    }

    func testExtraHeightIsZeroWhenInactive() {
        XCTAssertEqual(heights.last, 0)
        play(false)
        XCTAssertEqual(heights.last, 0)
    }

    func testExtraHeightIs24WhenActiveAndVisible() {
        play()
        XCTAssertEqual(heights.last, 24)
    }

    func testExtraHeightIsZeroWhenFolded() {
        preference.isVisible = false
        play()
        XCTAssertEqual(heights.last, 0)
    }

    func testFoldingUpdatesHeightImmediately() {
        play()
        XCTAssertEqual(heights.last, 24)

        preference.isVisible = false
        XCTAssertEqual(heights.last, 0)

        preference.isVisible = true
        XCTAssertEqual(heights.last, 24)
    }

    func testExtraHeightDoesNotRepeatValues() {
        play()
        play(true)
        preference.isVisible = true
        XCTAssertEqual(heights, [0, 24])
    }

    func testIsActiveFollowsPlayback() {
        var values: [Bool] = []
        module.isActivePublisher.sink { values.append($0) }.store(in: &cancellables)
        play()
        play(false)
        XCTAssertEqual(values, [false, true, false])
    }
}
