import XCTest
@testable import NotchCatKit

final class MusicPresentationTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_000_000)

    private let synced = Lyrics.synced([
        LyricLine(time: 10, text: "first"),
        LyricLine(time: 20, text: "second"),
        LyricLine(time: 30, text: "third"),
    ])

    private func snapshot(position: PositionAnchor?, duration: TimeInterval? = 200,
                          isPlaying: Bool = true) -> NowPlayingSnapshot {
        NowPlayingSnapshot(title: "Song", artist: "Artist", album: nil, duration: duration,
                           isPlaying: isPlaying, position: position, artwork: nil, sourceID: "test")
    }

    func testSyncedWithPositionShowsCurrentLine() {
        let anchor = PositionAnchor(elapsed: 12, timestamp: t0, rate: 1)

        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: anchor), lyrics: synced, at: t0),
            .line("first"))
        // 10 s later the position is 22 s: the second line is current.
        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: anchor), lyrics: synced,
                                           at: t0.addingTimeInterval(10)),
            .line("second"))
    }

    func testSyncedBeforeFirstLineShowsTitle() {
        let anchor = PositionAnchor(elapsed: 3, timestamp: t0, rate: 1)

        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: anchor), lyrics: synced, at: t0),
            .title("Song"))
    }

    func testSyncedWithoutAnchorShowsTitle() {
        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: nil), lyrics: synced, at: t0),
            .title("Song"))
    }

    func testPlainLyricsShowTitle() {
        let anchor = PositionAnchor(elapsed: 12, timestamp: t0, rate: 1)

        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: anchor),
                                           lyrics: .plain("la la la"), at: t0),
            .title("Song"))
    }

    func testNoLyricsShowsTitle() {
        let anchor = PositionAnchor(elapsed: 12, timestamp: t0, rate: 1)

        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: snapshot(position: anchor), lyrics: nil, at: t0),
            .title("Song"))
    }

    func testNoSnapshotIsNone() {
        XCTAssertEqual(MusicPresentation.stripContent(snapshot: nil, lyrics: synced, at: t0), .none)
    }

    func testPausedAnchorHoldsLine() {
        let anchor = PositionAnchor(elapsed: 22, timestamp: t0, rate: 0)
        let paused = snapshot(position: anchor, isPlaying: false)

        XCTAssertEqual(MusicPresentation.stripContent(snapshot: paused, lyrics: synced, at: t0),
                       .line("second"))
        XCTAssertEqual(
            MusicPresentation.stripContent(snapshot: paused, lyrics: synced,
                                           at: t0.addingTimeInterval(60)),
            .line("second"))
    }

    func testPlaybackPositionAdvancesAndCapsAtDuration() {
        let anchor = PositionAnchor(elapsed: 12, timestamp: t0, rate: 1)

        XCTAssertEqual(MusicPresentation.playbackPosition(snapshot: snapshot(position: anchor),
                                                          at: t0.addingTimeInterval(5)), 17)
        XCTAssertEqual(MusicPresentation.playbackPosition(snapshot: snapshot(position: anchor, duration: 20),
                                                          at: t0.addingTimeInterval(100)), 20)
    }

    func testPlaybackPositionIsNilWithoutAnchor() {
        XCTAssertNil(MusicPresentation.playbackPosition(snapshot: snapshot(position: nil), at: t0))
    }
}
