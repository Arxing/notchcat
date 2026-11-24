import XCTest
@testable import NotchCatKit

final class TrackIdentityTests: XCTestCase {
    private func make(title: String, artist: String = "A", duration: TimeInterval? = 200) -> TrackIdentity {
        TrackIdentity(title: title, artist: artist, album: nil, duration: duration)
    }

    func testStripsFeatSuffix() {
        XCTAssertEqual(make(title: "善良是 (feat. CHS)").normalized.title, "善良是")
        XCTAssertEqual(make(title: "Song (ft. Someone)").normalized.title, "Song")
        XCTAssertEqual(make(title: "Song [Remastered 2011]").normalized.title, "Song")
    }

    func testStripsFullWidthBracketSuffixes() {
        XCTAssertEqual(make(title: "歌名（feat. X）").normalized.title, "歌名")
        XCTAssertEqual(make(title: "歌名【Remastered】").normalized.title, "歌名")
        XCTAssertEqual(make(title: "歌名（Live）").normalized.title, "歌名")
    }

    /// The keyword must be a whole word: "Live" must not eat "Liverpool".
    func testSuffixKeywordsNeedWordBoundary() {
        XCTAssertEqual(make(title: "Song (Liverpool Mix)").normalized.title, "Song (Liverpool Mix)")
        XCTAssertEqual(make(title: "Song (Feather)").normalized.title, "Song (Feather)")
        XCTAssertEqual(make(title: "Song (Explicitly Yours)").normalized.title, "Song (Explicitly Yours)")
        XCTAssertEqual(make(title: "Song (Ftw)").normalized.title, "Song (Ftw)")
    }

    func testLeavesCleanTitleAlone() {
        XCTAssertEqual(make(title: "後來有很多人去過那裡").normalized.title, "後來有很多人去過那裡")
    }

    func testTakesPrimaryArtist() {
        XCTAssertEqual(make(title: "T", artist: "someshiit 山姆/CHS").normalized.artist, "someshiit 山姆")
        XCTAssertEqual(make(title: "T", artist: "A & B").normalized.artist, "A")
        XCTAssertEqual(make(title: "T", artist: "A, B").normalized.artist, "A")
    }

    func testDurationToleranceIsFiveSeconds() {
        let track = make(title: "T", duration: 200)
        XCTAssertTrue(track.durationMatches(200))
        XCTAssertTrue(track.durationMatches(204.9))
        XCTAssertTrue(track.durationMatches(195.1))
        XCTAssertFalse(track.durationMatches(210))
        XCTAssertFalse(track.durationMatches(190))
    }

    /// Either side not knowing the length must not reject a candidate.
    func testUnknownDurationAlwaysMatches() {
        XCTAssertTrue(make(title: "T", duration: 200).durationMatches(nil))
        XCTAssertTrue(make(title: "T", duration: 200).durationMatches(0))
        XCTAssertTrue(make(title: "T", duration: nil).durationMatches(200))
        XCTAssertTrue(make(title: "T", duration: 0).durationMatches(200))
        XCTAssertTrue(make(title: "T", duration: nil).durationMatches(nil))
    }

    func testLooseMatchIgnoresSimplifiedTraditional() {
        XCTAssertTrue(TrackIdentity.loosely("後來有很多人去過那裡", matches: "后来有很多人去过那里"))
        XCTAssertTrue(TrackIdentity.loosely("Song", matches: "song"))
        XCTAssertTrue(TrackIdentity.loosely("ＡＢＣ", matches: "abc"))
        XCTAssertFalse(TrackIdentity.loosely("Song", matches: "Different"))
    }

    func testCacheKeyIgnoresDuration() {
        XCTAssertEqual(make(title: "T", duration: 200).cacheKey,
                       make(title: "T", duration: 260).cacheKey)
        XCTAssertEqual(make(title: "T", duration: 200).cacheKey,
                       make(title: "T", duration: nil).cacheKey)
    }

    func testCacheKeyCollapsesCaseAndFeatSuffix() {
        XCTAssertEqual(make(title: "Song (feat. X)", artist: "Artist").cacheKey,
                       make(title: "song", artist: "ARTIST").cacheKey)
    }
}
