import XCTest
@testable import NotchCatKit

final class WindowTitleParserTests: XCTestCase {
    private func window(_ title: String, owner: String = "YouTube Music") -> WindowInfo {
        WindowInfo(owner: owner, title: title)
    }

    func testParsesPlayingTitle() {
        let parsed = WindowTitleParser.parse(window("YouTube Music - 假日而已 | YouTube Music 🔊"))
        XCTAssertEqual(parsed, ParsedTrack(title: "假日而已", isPlaying: true))
    }

    func testParsesPausedTitleWithoutSpeaker() {
        let parsed = WindowTitleParser.parse(window("YouTube Music - 假日而已 | YouTube Music"))
        XCTAssertEqual(parsed, ParsedTrack(title: "假日而已", isPlaying: false))
    }

    func testStripsExtraWhitespace() {
        let parsed = WindowTitleParser.parse(window("  YouTube Music -   假日而已  | YouTube Music  🔊  "))
        XCTAssertEqual(parsed, ParsedTrack(title: "假日而已", isPlaying: true))
    }

    func testUnknownOwnerReturnsNil() {
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music - 假日而已 | YouTube Music 🔊", owner: "Safari")))
    }

    func testEmptyTitleReturnsNil() {
        XCTAssertNil(WindowTitleParser.parse(window("")))
        XCTAssertNil(WindowTitleParser.parse(window("   ")))
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music -  | YouTube Music")))
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music - | YouTube Music 🔊")))
    }

    func testUnknownFormatReturnsNil() {
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music")))
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music - 假日而已")))
        XCTAssertNil(WindowTitleParser.parse(window("假日而已 | YouTube Music")))
        XCTAssertNil(WindowTitleParser.parse(window("Home - YouTube Music")))
        XCTAssertNil(WindowTitleParser.parse(window("YouTube Music - 假日而已 | Spotify")))
    }

    func testTitleContainingSeparators() {
        XCTAssertEqual(WindowTitleParser.parse(window("YouTube Music - A - B | YouTube Music 🔊")),
                       ParsedTrack(title: "A - B", isPlaying: true))
        XCTAssertEqual(WindowTitleParser.parse(window("YouTube Music - A | B | YouTube Music")),
                       ParsedTrack(title: "A | B", isPlaying: false))
    }
}
