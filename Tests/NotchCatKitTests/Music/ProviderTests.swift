import XCTest
@testable import NotchCatKit

/// Answers every request from a closure, so providers run end to end without a network.
private final class StubURLProtocol: URLProtocol {
    typealias Handler = (URLRequest) throws -> (status: Int, body: Data)

    private static let lock = NSLock()
    private static var handler: Handler?
    private static var recorded: [URLRequest] = []

    static func install(_ handler: @escaping Handler) {
        lock.lock()
        self.handler = handler
        recorded = []
        lock.unlock()
    }

    static var requests: [URLRequest] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    static func makeSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}

    override func startLoading() {
        Self.lock.lock()
        Self.recorded.append(request)
        let handler = Self.handler
        Self.lock.unlock()

        do {
            guard let handler else { throw URLError(.unsupportedURL) }
            let (status, body) = try handler(request)
            let response = HTTPURLResponse(
                url: request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: nil
            )!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: body)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }
}

private func data(_ json: String) -> Data { Data(json.utf8) }

private func queryValues(_ request: URLRequest) -> [String: String] {
    let items = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems ?? []
    return Dictionary(uniqueKeysWithValues: items.map { ($0.name, $0.value ?? "") })
}

final class ProviderTests: XCTestCase {
    private let track = TrackIdentity(title: "T", artist: "A", album: "Al", duration: 200)

    // MARK: LRCLIB decoding

    func testLRCLIBPrefersSyncedFieldOverPlain() throws {
        let json = data("""
        {"trackName":"T","artistName":"A","duration":200,
         "plainLyrics":"words","syncedLyrics":"[00:01.00]line"}
        """)
        let result = try XCTUnwrap(LRCLIBProvider.decode(json, for: track))
        XCTAssertEqual(result, .synced([LyricLine(time: 1, text: "line")]))
    }

    func testLRCLIBFallsBackToPlainWhenSyncedAbsent() throws {
        let json = data(#"{"trackName":"T","artistName":"A","duration":200,"plainLyrics":"words"}"#)
        XCTAssertEqual(try XCTUnwrap(LRCLIBProvider.decode(json, for: track)), .plain("words"))
    }

    func testLRCLIBRejectsMismatchedDuration() {
        let json = data(#"{"trackName":"T","artistName":"A","duration":400,"syncedLyrics":"[00:01.00]line"}"#)
        XCTAssertNil(LRCLIBProvider.decode(json, for: track))
    }

    func testLRCLIBAcceptsDurationWithinTolerance() {
        let json = data(#"{"trackName":"T","artistName":"A","duration":203,"syncedLyrics":"[00:01.00]line"}"#)
        XCTAssertNotNil(LRCLIBProvider.decode(json, for: track))
    }

    func testLRCLIBAcceptsResultWhenTrackHasNoDuration() {
        let json = data(#"{"trackName":"T","artistName":"A","duration":400,"syncedLyrics":"[00:01.00]line"}"#)
        let unknownLength = TrackIdentity(title: "T", artist: "A", album: nil, duration: nil)
        XCTAssertNotNil(LRCLIBProvider.decode(json, for: unknownLength))
    }

    func testLRCLIBSearchPicksFirstValidCandidate() throws {
        let json = data("""
        [{"trackName":"T","duration":400,"syncedLyrics":"[00:01.00]wrong length"},
         {"trackName":"T","duration":200,"syncedLyrics":"","plainLyrics":"  "},
         {"trackName":"T","duration":201,"syncedLyrics":"[00:01.00]second"},
         {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]third"}]
        """)
        XCTAssertEqual(
            LRCLIBProvider.decodeSearch(json, for: track),
            .synced([LyricLine(time: 1, text: "second")])
        )
    }

    func testLRCLIBSearchRejectsCandidateWithWrongTitle() {
        let json = data(#"[{"trackName":"Another Song","duration":200,"syncedLyrics":"[00:01.00]wrong"}]"#)
        XCTAssertNil(LRCLIBProvider.decodeSearch(json, for: track))
    }

    func testLRCLIBSearchRejectsWrongTitleWhenTrackHasNoDuration() {
        let json = data(#"[{"trackName":"Another Song","duration":200,"syncedLyrics":"[00:01.00]wrong"}]"#)
        let unknownLength = TrackIdentity(title: "T", artist: "A", album: nil, duration: nil)
        XCTAssertNil(LRCLIBProvider.decodeSearch(json, for: unknownLength))
    }

    func testLRCLIBSearchRejectsCandidateWithoutTrackName() {
        let json = data("""
        [{"duration":200,"syncedLyrics":"[00:01.00]no name"},
         {"trackName":"","duration":200,"syncedLyrics":"[00:01.00]empty name"}]
        """)
        XCTAssertNil(LRCLIBProvider.decodeSearch(json, for: track))
    }

    func testLRCLIBSearchAcceptsSimplifiedVariantOfTraditionalTitle() throws {
        let query = TrackIdentity(title: "後來", artist: "劉若英", album: nil, duration: nil)
        let json = data(#"[{"trackName":"后来","duration":250,"syncedLyrics":"[00:01.00]line"}]"#)
        XCTAssertEqual(
            try XCTUnwrap(LRCLIBProvider.decodeSearch(json, for: query)),
            .synced([LyricLine(time: 1, text: "line")])
        )
    }

    func testLRCLIBSearchAcceptsCandidateTitleWithFeaturingSuffix() {
        let json = data(#"[{"trackName":"T (feat. X)","duration":200,"syncedLyrics":"[00:01.00]line"}]"#)
        XCTAssertNotNil(LRCLIBProvider.decodeSearch(json, for: track))
    }

    func testLRCLIBSearchSkipsEarlierWrongTitleAndPicksFirstMatchingTitle() throws {
        let json = data("""
        [{"trackName":"T (Remix Of Something Else)","duration":200,"syncedLyrics":"[00:01.00]remix"},
         {"trackName":"Other","duration":200,"syncedLyrics":"[00:01.00]other"},
         {"trackName":"t","duration":200,"syncedLyrics":"[00:01.00]right"},
         {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]later"}]
        """)
        XCTAssertEqual(
            LRCLIBProvider.decodeSearch(json, for: track),
            .synced([LyricLine(time: 1, text: "right")])
        )
    }

    func testLRCLIBSearchWithNoValidCandidateYieldsNil() {
        XCTAssertNil(LRCLIBProvider.decodeSearch(data("[]"), for: track))
    }

    // MARK: LRCLIB requests and error semantics

    func testLRCLIBExactLookupSendsNormalizedParametersAndUserAgent() async throws {
        StubURLProtocol.install { _ in
            (200, data(#"{"duration":200,"syncedLyrics":"[00:01.00]hit"}"#))
        }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())
        let query = TrackIdentity(title: "Song (feat. X)", artist: "A & B", album: "Al", duration: 200)

        let result = try await provider.fetchLyrics(for: query)

        XCTAssertEqual(result, .synced([LyricLine(time: 1, text: "hit")]))
        let requests = StubURLProtocol.requests
        XCTAssertEqual(requests.count, 1)
        let request = try XCTUnwrap(requests.first)
        XCTAssertEqual(request.url?.host, "lrclib.net")
        XCTAssertEqual(request.url?.path, "/api/get")
        XCTAssertEqual(queryValues(request), [
            "track_name": "Song", "artist_name": "A", "album_name": "Al", "duration": "200",
        ])
        XCTAssertEqual(request.value(forHTTPHeaderField: "User-Agent"), "NotchCat")
        XCTAssertEqual(request.timeoutInterval, 5)
    }

    func testLRCLIBOmitsDurationAndAlbumWhenUnknown() async throws {
        StubURLProtocol.install { _ in (200, data(#"{"syncedLyrics":"[00:01.00]hit"}"#)) }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        _ = try await provider.fetchLyrics(for: TrackIdentity(title: "T", artist: "A", album: nil, duration: nil))
        _ = try await provider.fetchLyrics(for: TrackIdentity(title: "T", artist: "A", album: "", duration: 0))

        for request in StubURLProtocol.requests {
            XCTAssertEqual(queryValues(request), ["track_name": "T", "artist_name": "A"])
        }
    }

    func testLRCLIBEncodesCJKAndReservedCharactersInQuery() async throws {
        StubURLProtocol.install { _ in (200, data(#"{"syncedLyrics":"[00:01.00]hit"}"#)) }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        _ = try await provider.fetchLyrics(
            for: TrackIdentity(title: "晴天 + Rock&Roll", artist: "周杰倫", album: nil, duration: nil)
        )

        let request = try XCTUnwrap(StubURLProtocol.requests.first)
        let absolute = try XCTUnwrap(request.url?.absoluteString)
        XCTAssertTrue(absolute.contains("track_name=%E6%99%B4%E5%A4%A9%20%2B%20Rock%26Roll"), absolute)
        XCTAssertTrue(absolute.contains("artist_name=%E5%91%A8%E6%9D%B0%E5%80%AB"), absolute)
        XCTAssertEqual(queryValues(request)["track_name"], "晴天 + Rock&Roll")
    }

    func testLRCLIBExact404FallsThroughToSearch() async throws {
        StubURLProtocol.install { request in
            if request.url?.path == "/api/get" { return (404, data(#"{"code":404}"#)) }
            return (200, data(#"[{"trackName":"T","duration":200,"syncedLyrics":"[00:02.00]found by search"}]"#))
        }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(for: track)

        XCTAssertEqual(result, .synced([LyricLine(time: 2, text: "found by search")]))
        let requests = StubURLProtocol.requests
        XCTAssertEqual(requests.map { $0.url?.path }, ["/api/get", "/api/search"])
        XCTAssertEqual(queryValues(requests[1]), ["q": "T A"])
        XCTAssertEqual(requests[1].value(forHTTPHeaderField: "User-Agent"), "NotchCat")
    }

    func testLRCLIBExactHitRejectedByDurationAlsoFallsThroughToSearch() async throws {
        StubURLProtocol.install { request in
            if request.url?.path == "/api/get" {
                return (200, data(#"{"duration":400,"syncedLyrics":"[00:01.00]wrong song"}"#))
            }
            return (200, data(#"[{"trackName":"T","duration":199,"syncedLyrics":"[00:02.00]right song"}]"#))
        }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(for: track)

        XCTAssertEqual(result, .synced([LyricLine(time: 2, text: "right song")]))
    }

    func testLRCLIBNotFoundEverywhereReturnsNilWithoutThrowing() async throws {
        StubURLProtocol.install { request in
            request.url?.path == "/api/get" ? (404, data("{}")) : (200, data("[]"))
        }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(for: track)

        XCTAssertNil(result)
    }

    func testLRCLIBServerErrorOnExactLookupThrows() async {
        StubURLProtocol.install { _ in (503, data("")) }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a 503 must surface as a failure, not as \"not found\"")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badStatus(503))
        } catch {
            XCTFail("unexpected error \(error)")
        }
        XCTAssertEqual(StubURLProtocol.requests.count, 1)
    }

    func testLRCLIBServerErrorOnSearchThrows() async {
        StubURLProtocol.install { request in
            request.url?.path == "/api/get" ? (404, data("{}")) : (500, data(""))
        }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a failing search must surface as a failure")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badStatus(500))
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testLRCLIBGarbledBodyThrows() async {
        StubURLProtocol.install { _ in (200, data("<html>gateway</html>")) }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a garbled body must surface as a failure")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badResponse)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testLRCLIBTransportErrorPropagates() async {
        StubURLProtocol.install { _ in throw URLError(.notConnectedToInternet) }
        let provider = LRCLIBProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("an outage must surface as a failure, not as \"not found\"")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .notConnectedToInternet)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    // MARK: NetEase decoding

    func testNetEaseDecodesLrcField() throws {
        let json = data(#"{"lrc":{"lyric":"[00:02.00]hello"}}"#)
        XCTAssertEqual(
            try XCTUnwrap(NetEaseProvider.decodeLyric(json)),
            .synced([LyricLine(time: 2, text: "hello")])
        )
    }

    func testNetEaseIgnoresTranslationSoItCannotShadowTheOriginal() throws {
        let json = data("""
        {"lrc":{"lyric":"[00:02.00]original"},"tlyric":{"lyric":"[00:02.00]翻譯"}}
        """)
        XCTAssertEqual(
            try XCTUnwrap(NetEaseProvider.decodeLyric(json)),
            .synced([LyricLine(time: 2, text: "original")])
        )
    }

    func testNetEaseEmptyLyricYieldsNil() {
        XCTAssertNil(NetEaseProvider.decodeLyric(data(#"{"lrc":{"lyric":""}}"#)))
    }

    func testNetEaseMissingLrcYieldsNil() {
        XCTAssertNil(NetEaseProvider.decodeLyric(data("{}")))
        XCTAssertNil(NetEaseProvider.decodeLyric(data(#"{"nolyric":true}"#)))
    }

    // MARK: NetEase song selection

    private func searchJSON(_ songs: String) -> Data {
        data(#"{"code":200,"result":{"songCount":1,"songs":[\#(songs)]}}"#)
    }

    func testNetEaseMatchesSimplifiedTitleForTraditionalQuery() throws {
        let query = TrackIdentity(title: "後來", artist: "劉若英", album: nil, duration: 200)
        let json = searchJSON(#"{"id":7,"name":"后来","duration":200000}"#)
        XCTAssertEqual(try NetEaseProvider.selectSongID(from: json, for: query), 7)
    }

    func testNetEaseDoesNotFallBackToFirstSongWhenNoTitleMatches() throws {
        let json = searchJSON("""
        {"id":1,"name":"Completely Different","duration":200000},
        {"id":2,"name":"Another One","duration":200000}
        """)
        XCTAssertNil(try NetEaseProvider.selectSongID(from: json, for: track.withTitle("Wanted")))
    }

    func testNetEaseTreatsDurationAsMilliseconds() throws {
        let json = searchJSON("""
        {"id":1,"name":"T","duration":400000},
        {"id":2,"name":"T","duration":203000}
        """)
        XCTAssertEqual(try NetEaseProvider.selectSongID(from: json, for: track), 2)
    }

    func testNetEaseSkipsDurationCheckWhenEitherSideHasNone() throws {
        let json = searchJSON(#"{"id":1,"name":"T","duration":0},{"id":2,"name":"T","duration":900000}"#)
        XCTAssertEqual(try NetEaseProvider.selectSongID(from: json, for: track), 1)

        let unknownLength = TrackIdentity(title: "T", artist: "A", album: nil, duration: nil)
        let long = searchJSON(#"{"id":3,"name":"T","duration":900000}"#)
        XCTAssertEqual(try NetEaseProvider.selectSongID(from: long, for: unknownLength), 3)
    }

    func testNetEaseSelectionComparesTitleWithoutFeaturingSuffix() throws {
        let query = TrackIdentity(title: "Song (feat. X)", artist: "A", album: nil, duration: nil)
        let json = searchJSON(#"{"id":4,"name":"Song","duration":0}"#)
        XCTAssertEqual(try NetEaseProvider.selectSongID(from: json, for: query), 4)
    }

    func testNetEaseSelectionWithNoResultsYieldsNil() throws {
        XCTAssertNil(try NetEaseProvider.selectSongID(from: data(#"{"code":200,"result":{"songCount":0}}"#), for: track))
        XCTAssertNil(try NetEaseProvider.selectSongID(from: data(#"{"code":200,"result":{"songs":[]}}"#), for: track))
    }

    func testNetEaseSelectionRejectsGarbledAndRefusedResponses() {
        XCTAssertThrowsError(try NetEaseProvider.selectSongID(from: data("<html/>"), for: track))
        XCTAssertThrowsError(try NetEaseProvider.selectSongID(from: data(#"{"code":-460,"msg":"Cheating"}"#), for: track))
    }

    func testNetEaseSelectionRejectsResponseWithNeitherCodeNorResult() {
        XCTAssertThrowsError(try NetEaseProvider.selectSongID(from: data("{}"), for: track))
        XCTAssertThrowsError(try NetEaseProvider.selectSongID(from: data(#"{"msg":"hi"}"#), for: track))
    }

    func testNetEaseInstrumentalPlaceholderYieldsNil() {
        for placeholder in ["纯音乐，请欣赏", "纯音乐, 请欣赏", "暂无歌词", "没有歌词", "純音樂，請欣賞", "  暫無歌詞 "] {
            let json = data(#"{"lrc":{"lyric":"[00:00.00]\#(placeholder)"}}"#)
            XCTAssertNil(NetEaseProvider.decodeLyric(json), placeholder)
        }
        XCTAssertNil(NetEaseProvider.decodeLyric(data(#"{"lrc":{"lyric":"纯音乐，请欣赏"}}"#)))
    }

    func testNetEaseRealSingleLineLyricIsNotDropped() throws {
        let json = data(#"{"lrc":{"lyric":"[00:05.00]Just one real line"}}"#)
        XCTAssertEqual(
            try XCTUnwrap(NetEaseProvider.decodeLyric(json)),
            .synced([LyricLine(time: 5, text: "Just one real line")])
        )
    }

    func testNetEasePlaceholderOnLaterLineOfLongerLyricIsKept() throws {
        let json = data(#"{"lrc":{"lyric":"[00:01.00]real\n[00:02.00]暂无歌词"}}"#)
        XCTAssertEqual(try XCTUnwrap(NetEaseProvider.decodeLyric(json)), .synced([
            LyricLine(time: 1, text: "real"), LyricLine(time: 2, text: "暂无歌词"),
        ]))
    }

    func testNetEaseRefusedLyricResponseThrowsThroughFetch() async {
        StubURLProtocol.install { request in
            request.url?.path == "/api/search/get/web"
                ? (200, self.searchJSON(#"{"id":1,"name":"T","duration":200000}"#))
                : (200, data(#"{"code":-460,"msg":"Cheating"}"#))
        }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a refused lyric request must surface as a failure, not as \"not found\"")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badResponse)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testNetEaseLyricResponseWithCode200IsAccepted() throws {
        let json = data(#"{"code":200,"lrc":{"lyric":"[00:02.00]hello"}}"#)
        XCTAssertNotNil(NetEaseProvider.decodeLyric(json))
    }

    // MARK: NetEase requests and error semantics

    func testNetEaseFetchSearchesThenFetchesOriginalLyricWithHeaders() async throws {
        StubURLProtocol.install { request in
            if request.url?.path == "/api/search/get/web" {
                return (200, self.searchJSON(#"{"id":555,"name":"晴天","duration":200000}"#))
            }
            return (200, data("""
            {"lrc":{"lyric":"[00:03.00]original"},"tlyric":{"lyric":"[00:03.00]translated"}}
            """))
        }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(
            for: TrackIdentity(title: "晴天", artist: "周杰倫", album: nil, duration: 200)
        )

        XCTAssertEqual(result, .synced([LyricLine(time: 3, text: "original")]))
        let requests = StubURLProtocol.requests
        XCTAssertEqual(requests.map { $0.url?.host }, ["music.163.com", "music.163.com"])
        XCTAssertEqual(requests.map { $0.url?.path }, ["/api/search/get/web", "/api/song/lyric"])
        XCTAssertEqual(queryValues(requests[0])["s"], "晴天 周杰倫")
        XCTAssertEqual(queryValues(requests[0])["type"], "1")
        XCTAssertEqual(queryValues(requests[0])["limit"], "5")
        XCTAssertTrue(requests[0].url!.absoluteString.contains("%E6%99%B4%E5%A4%A9"))
        XCTAssertEqual(queryValues(requests[1]), ["id": "555", "lv": "1", "kv": "1", "tv": "-1"])
        for request in requests {
            XCTAssertEqual(request.value(forHTTPHeaderField: "Referer"), "https://music.163.com/")
            XCTAssertEqual(request.value(forHTTPHeaderField: "User-Agent"), "NotchCat")
            XCTAssertEqual(request.timeoutInterval, 5)
        }
    }

    func testNetEaseNoTitleMatchReturnsNilAndSkipsLyricRequest() async throws {
        StubURLProtocol.install { _ in (200, self.searchJSON(#"{"id":1,"name":"Other","duration":200000}"#)) }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(for: track)

        XCTAssertNil(result)
        XCTAssertEqual(StubURLProtocol.requests.count, 1)
    }

    func testNetEaseSongWithoutLyricReturnsNil() async throws {
        StubURLProtocol.install { request in
            request.url?.path == "/api/search/get/web"
                ? (200, self.searchJSON(#"{"id":1,"name":"T","duration":200000}"#))
                : (200, data(#"{"nolyric":true}"#))
        }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        let result = try await provider.fetchLyrics(for: track)

        XCTAssertNil(result)
    }

    func testNetEaseServerErrorOnSearchThrows() async {
        StubURLProtocol.install { _ in (502, data("")) }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a 502 must surface as a failure")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badStatus(502))
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testNetEaseServerErrorOnLyricThrows() async {
        StubURLProtocol.install { request in
            request.url?.path == "/api/search/get/web"
                ? (200, self.searchJSON(#"{"id":1,"name":"T","duration":200000}"#))
                : (500, data(""))
        }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a failing lyric request must surface as a failure")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badStatus(500))
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testNetEaseGarbledLyricBodyThrows() async {
        StubURLProtocol.install { request in
            request.url?.path == "/api/search/get/web"
                ? (200, self.searchJSON(#"{"id":1,"name":"T","duration":200000}"#))
                : (200, data("not json"))
        }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a garbled body must surface as a failure")
        } catch let error as LyricsProviderError {
            XCTAssertEqual(error, .badResponse)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    func testNetEaseTransportErrorPropagates() async {
        StubURLProtocol.install { _ in throw URLError(.timedOut) }
        let provider = NetEaseProvider(session: StubURLProtocol.makeSession())

        do {
            _ = try await provider.fetchLyrics(for: track)
            XCTFail("a timeout must surface as a failure, not as \"not found\"")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .timedOut)
        } catch {
            XCTFail("unexpected error \(error)")
        }
    }

    // MARK: Live

    /// Run by hand against the live services; excluded from the default run because the
    /// result depends on third-party availability and catalogue coverage.
    func testLiveLookupSmoke() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["NOTCHCAT_LIVE_TESTS"] == "1")
        let hit = TrackIdentity(title: "Bohemian Rhapsody", artist: "Queen", album: nil, duration: nil)
        let lrclib = try await LRCLIBProvider().fetchLyrics(for: hit)
        let netEase = try await NetEaseProvider().fetchLyrics(for: hit)
        print("live smoke: LRCLIB=\(describe(lrclib)) NetEase=\(describe(netEase))")
        XCTAssertTrue(lrclib != nil || netEase != nil, "neither service returned lyrics")
    }

    private func describe(_ lyrics: Lyrics?) -> String {
        switch lyrics {
        case .synced(let lines): return "synced(\(lines.count) lines)"
        case .plain(let text): return "plain(\(text.count) chars)"
        case nil: return "nil"
        }
    }
}

private extension TrackIdentity {
    func withTitle(_ title: String) -> TrackIdentity {
        TrackIdentity(title: title, artist: artist, album: album, duration: duration)
    }
}
