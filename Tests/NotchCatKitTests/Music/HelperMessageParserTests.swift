import XCTest
@testable import NotchCatKit

final class HelperMessageParserTests: XCTestCase {
    private func track(_ line: String, file: StaticString = #filePath, lineNumber: UInt = #line) -> NowPlayingSnapshot? {
        guard case .track(let snapshot)? = HelperMessageParser.parse(line: line) else {
            XCTFail("expected a track for: \(line)", file: file, line: lineNumber)
            return nil
        }
        return snapshot
    }

    func testParsesFullTrack() throws {
        let artwork = Data([0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02])
        let line = """
        {"title":"蘇裔非 - 抱我!  ft. 莊岱諺","artist":"蘇裔非","album":"","duration":213.081,\
        "elapsed":0,"timestamp":1793750400.5,"rate":1,\
        "artworkId":"7C1F2A52-4D41-4E0B-9B6B-0E2C3D4F5A6B","artworkData":"\(artwork.base64EncodedString())"}
        """
        let snapshot = try XCTUnwrap(track(line))
        XCTAssertEqual(snapshot.title, "蘇裔非 - 抱我!  ft. 莊岱諺")
        XCTAssertEqual(snapshot.artist, "蘇裔非")
        XCTAssertNil(snapshot.album)
        XCTAssertEqual(snapshot.duration, 213.081)
        XCTAssertTrue(snapshot.isPlaying)
        XCTAssertEqual(snapshot.sourceID, "mediaremote")
        XCTAssertEqual(snapshot.position,
                       PositionAnchor(elapsed: 0,
                                      timestamp: Date(timeIntervalSince1970: 1793750400.5),
                                      rate: 1))
        XCTAssertEqual(snapshot.artwork?.identifier, "7C1F2A52-4D41-4E0B-9B6B-0E2C3D4F5A6B")
        XCTAssertEqual(snapshot.artwork?.data, artwork)
    }

    func testParsesNone() {
        XCTAssertEqual(HelperMessageParser.parse(line: #"{"none":true}"#), HelperMessage.none)
    }

    func testArtworkDataAbsentKeepsIdentifier() throws {
        let snapshot = try XCTUnwrap(track(
            #"{"title":"T","elapsed":1,"timestamp":100,"rate":1,"artworkId":"abc"}"#))
        XCTAssertEqual(snapshot.artwork?.identifier, "abc")
        XCTAssertNil(snapshot.artwork?.data)
    }

    func testRateZeroIsPaused() throws {
        let snapshot = try XCTUnwrap(track(#"{"title":"T","elapsed":42,"timestamp":100,"rate":0}"#))
        XCTAssertFalse(snapshot.isPlaying)
        XCTAssertEqual(snapshot.position?.rate, 0)
        XCTAssertEqual(snapshot.position?.elapsed, 42)
    }

    func testMissingTitleYieldsNil() {
        XCTAssertNil(HelperMessageParser.parse(line: #"{"artist":"A","elapsed":1,"timestamp":100,"rate":1}"#))
    }

    func testEmptyTitleYieldsNil() {
        XCTAssertNil(HelperMessageParser.parse(line: #"{"title":"","elapsed":1,"timestamp":100,"rate":1}"#))
    }

    func testGarbageYieldsNil() {
        for line in ["hello", "{", "", "[]"] {
            XCTAssertNil(HelperMessageParser.parse(line: line), "line: \(line)")
        }
    }

    func testMissingOptionalFieldsAreFine() throws {
        let snapshot = try XCTUnwrap(track(#"{"title":"T","elapsed":5,"timestamp":200,"rate":1}"#))
        XCTAssertEqual(snapshot.title, "T")
        XCTAssertNil(snapshot.artist)
        XCTAssertNil(snapshot.album)
        XCTAssertNil(snapshot.duration)
        XCTAssertNil(snapshot.artwork)
        XCTAssertEqual(snapshot.position,
                       PositionAnchor(elapsed: 5, timestamp: Date(timeIntervalSince1970: 200), rate: 1))
    }

    func testEmptyArtistIsNil() throws {
        let snapshot = try XCTUnwrap(track(#"{"title":"T","artist":"","elapsed":5,"timestamp":200,"rate":1}"#))
        XCTAssertNil(snapshot.artist)
    }

    func testNonFiniteNumbersYieldNil() {
        // 1e999 overflows to infinity when decoded.
        for key in ["elapsed", "timestamp", "rate", "duration"] {
            var fields = ["elapsed": "1", "timestamp": "100", "rate": "1", "duration": "200"]
            fields[key] = "1e999"
            let body = fields.map { "\"\($0.key)\":\($0.value)" }.sorted().joined(separator: ",")
            XCTAssertNil(HelperMessageParser.parse(line: "{\"title\":\"T\",\(body)}"), "key: \(key)")
        }
    }

    func testNonPositiveDurationIsAbsent() throws {
        for duration in ["0", "-3"] {
            let snapshot = try XCTUnwrap(track(
                "{\"title\":\"T\",\"duration\":\(duration),\"elapsed\":1,\"timestamp\":100,\"rate\":1}"))
            XCTAssertNil(snapshot.duration, "duration: \(duration)")
            XCTAssertEqual(snapshot.title, "T")
        }
    }

    // MARK: HelperLineBuffer

    func testLineBufferSplitsAcrossReads() {
        var buffer = HelperLineBuffer()
        XCTAssertEqual(buffer.append(Data(#"{"none":"#.utf8)), [])
        XCTAssertEqual(buffer.append(Data("true}\n".utf8)), [#"{"none":true}"#])
    }

    func testLineBufferHandlesMultipleLinesInOneRead() {
        var buffer = HelperLineBuffer()
        let lines = buffer.append(Data("one\ntwo\nthr".utf8))
        XCTAssertEqual(lines, ["one", "two"])
        XCTAssertEqual(buffer.append(Data("ee\n".utf8)), ["three"])
    }

    func testLineBufferDropsInvalidUTF8Line() {
        var buffer = HelperLineBuffer()
        var data = Data("good\n".utf8)
        data.append(contentsOf: [0x62, 0xFF, 0xFE, 0x0A])
        data.append(Data(#"{"none":true}"#.utf8))
        data.append(0x0A)
        let lines = buffer.append(data)
        XCTAssertEqual(lines, ["good", #"{"none":true}"#])
        XCTAssertEqual(lines.last.flatMap { HelperMessageParser.parse(line: $0) }, HelperMessage.none)
    }

    func testLineBufferKeepsMultibyteCharacterSplitAcrossReads() {
        var buffer = HelperLineBuffer()
        let bytes = Array("蘇\n".utf8)
        XCTAssertEqual(buffer.append(Data(bytes[0..<2])), [])
        XCTAssertEqual(buffer.append(Data(bytes[2...])), ["蘇"])
    }
}
