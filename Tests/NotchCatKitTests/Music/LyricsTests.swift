import XCTest
@testable import NotchCatKit

final class LyricsTests: XCTestCase {
    private func synced(_ raw: String, file: StaticString = #filePath, line: UInt = #line) -> [LyricLine] {
        guard case .synced(let lines)? = LRCParser.parse(raw) else {
            XCTFail("expected synced lyrics for \(raw.prefix(60))", file: file, line: line)
            return []
        }
        return lines
    }

    // MARK: - Parsing

    func testParsesTimestampedLines() {
        let lines = synced("[00:12.34]first\n[01:05.00]second")
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines[0].time, 12.34, accuracy: 0.001)
        XCTAssertEqual(lines[0].text, "first")
        XCTAssertEqual(lines[1].time, 65.0, accuracy: 0.001)
        XCTAssertEqual(lines[1].text, "second")
    }

    func testUnsortedTimestampsAreSorted() {
        XCTAssertEqual(synced("[01:00.00]b\n[00:10.00]a").map(\.text), ["a", "b"])
    }

    func testTextWithoutAnyTimestampIsPlain() {
        guard case .plain(let text)? = LRCParser.parse("  just words\nmore words \n") else {
            return XCTFail("expected plain lyrics")
        }
        XCTAssertEqual(text, "just words\nmore words")
    }

    func testMetadataOnlyTagsDoNotCountAsSynced() {
        guard case .plain(let text)? = LRCParser.parse("[ar:Artist]\n[ti:Title]\n[offset:500]\nwords") else {
            return XCTFail("metadata tags are not timestamps")
        }
        XCTAssertEqual(text, "[ar:Artist]\n[ti:Title]\n[offset:500]\nwords")
    }

    func testMetadataLinesAlongsideTimestampsAreIgnored() {
        let lines = synced("[ar:Artist]\n[ti:Title]\n[length:03:20]\n[00:01.00]hello")
        XCTAssertEqual(lines, [LyricLine(time: 1, text: "hello")])
    }

    func testEmptyInputYieldsNil() {
        XCTAssertNil(LRCParser.parse(""))
        XCTAssertNil(LRCParser.parse("   \n  "))
    }

    func testBlankLyricLinesAreDropped() {
        XCTAssertEqual(synced("[00:01.00]\n[00:02.00]real").map(\.text), ["real"])
    }

    func testRepeatedTimestampsOnOneLine() {
        let lines = synced("[00:10.00][00:20.00]chorus")
        XCTAssertEqual(lines.map(\.time), [10, 20])
        XCTAssertEqual(lines.map(\.text), ["chorus", "chorus"])
    }

    func testDuplicateTimestampsKeepBothLinesInInputOrder() {
        let lines = synced("[00:10.00]one\n[00:05.00]early\n[00:10.00]two\n[00:10.00]three")
        XCTAssertEqual(lines.map(\.text), ["early", "one", "two", "three"])
    }

    func testTimeTagFormats() {
        let lines = synced("""
        [00:01.50]centis-one-digit
        [00:02.25]centis
        [00:03.125]millis
        [00:04]bare
        [00:05:50]colon-fraction
        [1:06.5]short-minutes
        """)
        XCTAssertEqual(lines.map(\.text), [
            "centis-one-digit", "centis", "millis", "bare", "colon-fraction", "short-minutes",
        ])
        let expected: [TimeInterval] = [1.5, 2.25, 3.125, 4, 5.5, 66.5]
        for (line, time) in zip(lines, expected) {
            XCTAssertEqual(line.time, time, accuracy: 0.0001, line.text)
        }
    }

    func testWhitespaceAroundTagsAndTextIsTrimmed() {
        let lines = synced("  [00:01.00]   spaced out   \r\n[00:02.00]\ttabbed\t")
        XCTAssertEqual(lines.map(\.text), ["spaced out", "tabbed"])
    }

    func testCRLFAndCRLineEndings() {
        XCTAssertEqual(synced("[00:01.00]a\r\n[00:02.00]b\r\n").map(\.text), ["a", "b"])
        XCTAssertEqual(synced("[00:01.00]a\r[00:02.00]b").map(\.text), ["a", "b"])
    }

    func testNonASCIITextIsPreservedIntact() {
        let lines = synced("[00:01.00]你好，世界 🎵\n[00:02.00]héllo")
        XCTAssertEqual(lines.map(\.text), ["你好，世界 🎵", "héllo"])
    }

    func testLeadingByteOrderMarkIsIgnored() {
        XCTAssertEqual(synced("\u{FEFF}[00:01.00]hi").map(\.text), ["hi"])
    }

    func testTimeTagInsideTextIsNotATimestamp() {
        let lines = synced("[00:01.00]see [00:30.00] later")
        XCTAssertEqual(lines, [LyricLine(time: 1, text: "see [00:30.00] later")])
    }

    // MARK: - Hostile input

    func testHugeMinutesValueDoesNotCrash() {
        let huge = "[99999999999999999999999999:00.00]x"
        guard case .plain(let text)? = LRCParser.parse(huge) else {
            return XCTFail("an unparseable tag is not a timestamp")
        }
        XCTAssertEqual(text, huge)

        let big = synced("[999:59.999]last")
        XCTAssertEqual(big[0].time, 999 * 60 + 59.999, accuracy: 0.0001)
    }

    func testHugeSecondsAndFractionDoNotCrash() {
        XCTAssertEqual(LRCParser.parse("[00:99999999999999999999.00]x"), .plain("[00:99999999999999999999.00]x"))
        XCTAssertEqual(LRCParser.parse("[00:01.99999999999999999999]x"), .plain("[00:01.99999999999999999999]x"))
    }

    func testNegativeLookingTagsAreNotTimestamps() {
        XCTAssertEqual(LRCParser.parse("[-00:05.00]x\n[00:-5.00]y\n[+00:05.00]z"),
                       .plain("[-00:05.00]x\n[00:-5.00]y\n[+00:05.00]z"))
    }

    func testMalformedTagsAreNotTimestamps() {
        for raw in ["[00:01.00", "00:01.00]x", "[:01.00]x", "[00:]x", "[00:01.]x", "[a0:01.00]x", "[00:01.00x]y", "[]x", "["] {
            XCTAssertEqual(LRCParser.parse(raw), .plain(raw), raw)
        }
    }

    func testTagsOnlyLinesProduceNoLines() {
        XCTAssertNil(LRCParser.parse("[00:00.00]"))
        XCTAssertNil(LRCParser.parse("[00:01.00][00:02.00]\n[00:03.00]   "))
    }

    func testEmptyTextAtZeroIsDroppedButRealLinesSurvive() {
        XCTAssertEqual(synced("[00:00.00]\n[00:05.00]go"), [LyricLine(time: 5, text: "go")])
    }

    func testOverlongLineDoesNotCrash() {
        let longText = String(repeating: "a", count: 2_000_000)
        let lines = synced("[00:01.00]\(longText)")
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(lines[0].text.count, 2_000_000)

        let noise = String(repeating: "[", count: 500_000)
        XCTAssertEqual(LRCParser.parse(noise), .plain(noise))
    }

    func testManyTagsOnOneLineDoNotCrash() {
        let tags = String(repeating: "[00:01.00]", count: 20_000)
        XCTAssertEqual(synced(tags + "x").count, 20_000)
    }

    // MARK: - lineIndex

    func testLineIndexAtPosition() {
        let lyrics = Lyrics.synced([
            LyricLine(time: 0, text: "a"),
            LyricLine(time: 10, text: "b"),
            LyricLine(time: 20, text: "c"),
        ])
        XCTAssertEqual(lyrics.lineIndex(at: 0), 0)
        XCTAssertEqual(lyrics.lineIndex(at: 9.9), 0)
        XCTAssertEqual(lyrics.lineIndex(at: 10), 1)
        XCTAssertEqual(lyrics.lineIndex(at: 999), 2)
    }

    func testLineIndexBeforeFirstLineIsNil() {
        let lyrics = Lyrics.synced([LyricLine(time: 5, text: "a")])
        XCTAssertNil(lyrics.lineIndex(at: 1))
        XCTAssertNil(lyrics.lineIndex(at: -3))
        XCTAssertNil(lyrics.lineIndex(at: .nan))
    }

    func testLineIndexOnPlainLyricsIsNil() {
        XCTAssertNil(Lyrics.plain("words").lineIndex(at: 10))
    }

    func testLineIndexWithDuplicateTimestampsPicksLast() {
        let lyrics = Lyrics.synced([
            LyricLine(time: 1, text: "a"),
            LyricLine(time: 5, text: "b"),
            LyricLine(time: 5, text: "c"),
            LyricLine(time: 9, text: "d"),
        ])
        XCTAssertEqual(lyrics.lineIndex(at: 5), 2)
        XCTAssertEqual(lyrics.lineIndex(at: 6), 2)
        XCTAssertEqual(lyrics.lineIndex(at: .infinity), 3)
    }

    func testIsSynced() {
        XCTAssertTrue(Lyrics.synced([LyricLine(time: 0, text: "a")]).isSynced)
        XCTAssertFalse(Lyrics.plain("a").isSynced)
    }
}
