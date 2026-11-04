import XCTest
@testable import NotchCatKit

final class TrackNormalizerTests: XCTestCase {
    private func snapshot(title: String, artist: String?, album: String? = nil,
                          duration: TimeInterval? = 200) -> NowPlayingSnapshot {
        NowPlayingSnapshot(title: title, artist: artist, album: album, duration: duration,
                           isPlaying: true, position: nil, artwork: nil, sourceID: "test")
    }

    private func title(_ title: String, artist: String?) -> String? {
        TrackNormalizer.identity(from: snapshot(title: title, artist: artist))?.title
    }

    /// Real sample from YouTube in Chrome (note the two spaces before "ft.").
    func testYouTubeTitleSplitsOffArtist() {
        XCTAssertEqual(title("蘇裔非 - 抱我!  ft. 莊岱諺", artist: "蘇裔非"), "抱我!")
    }

    func testCollapsesExtraWhitespace() {
        XCTAssertEqual(title("Some   Song    Name", artist: "X"), "Some Song Name")
        XCTAssertEqual(title("X  -   Some   Song", artist: "X"), "Some Song")
    }

    func testStripsMvBrackets() {
        XCTAssertEqual(title("X - Song (MV)", artist: "X"), "Song")
        XCTAssertEqual(title("X - Song [Official Video]", artist: "X"), "Song")
        XCTAssertEqual(title("X - Song (Lyrics)", artist: "X"), "Song")
        XCTAssertEqual(title("Song (Official Music Video)", artist: "X"), "Song")
        XCTAssertEqual(title("X - Song feat. Y [Official Video]", artist: "X"), "Song")
    }

    func testStripsFeatSegment() {
        XCTAssertEqual(title("Song feat. Someone Else", artist: "X"), "Song")
        XCTAssertEqual(title("Song Ft. Someone", artist: "X"), "Song")
    }

    func testNoDashKeepsOriginalTitle() {
        XCTAssertEqual(title("抱我!", artist: "蘇裔非"), "抱我!")
        XCTAssertEqual(title("Hyphen-ated Title", artist: "X"), "Hyphen-ated Title")
    }

    func testDashButArtistDoesNotMatchKeepsOriginal() {
        XCTAssertEqual(title("Live - Tonight Show", artist: "Someone Else"), "Live - Tonight Show")
    }

    func testArtistMatchIgnoresCaseAndSpacing() {
        XCTAssertEqual(title("some artist - Song", artist: "Some Artist"), "Song")
        XCTAssertEqual(title("A - Song", artist: "A & B"), "Song")
    }

    func testMissingArtistReturnsNil() {
        XCTAssertNil(TrackNormalizer.identity(from: snapshot(title: "Song", artist: nil)))
        XCTAssertNil(TrackNormalizer.identity(from: snapshot(title: "Song", artist: "")))
        XCTAssertNil(TrackNormalizer.identity(from: snapshot(title: "Song", artist: "   ")))
    }

    func testMissingTitleReturnsNil() {
        XCTAssertNil(TrackNormalizer.identity(from: snapshot(title: "", artist: "X")))
        XCTAssertNil(TrackNormalizer.identity(from: snapshot(title: "  ", artist: "X")))
    }

    func testCleanMusicAppTitleUntouched() {
        XCTAssertEqual(title("後來有很多人去過那裡", artist: "someshiit 山姆"), "後來有很多人去過那裡")
        XCTAssertEqual(title("Bohemian Rhapsody", artist: "Queen"), "Bohemian Rhapsody")
    }

    func testCarriesArtistAlbumAndDuration() {
        let id = TrackNormalizer.identity(from: snapshot(title: "T", artist: "Artist", album: "Album", duration: 123))
        XCTAssertEqual(id, TrackIdentity(title: "T", artist: "Artist", album: "Album", duration: 123))
    }
}
