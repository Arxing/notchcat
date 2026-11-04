import Foundation

/// Unofficial NetEase Cloud Music endpoints. Better coverage of Chinese-language
/// catalogue than LRCLIB, at the cost of being undocumented and liable to
/// change, which is what `LyricsProvider` exists to contain.
///
/// Only the original lyric (`lrc`) is used. The response also carries a
/// translation (`tlyric`); merging it in would let a translated line that
/// shares a timestamp shadow the original.
struct NetEaseProvider: LyricsProvider {
    let id = "com.music163"
    private let session: URLSession
    private let timeout: TimeInterval

    init(session: URLSession = .shared, timeout: TimeInterval = 5) {
        self.session = session
        self.timeout = timeout
    }

    private struct SearchResponse: Decodable {
        struct Result: Decodable {
            struct Song: Decodable {
                let id: Int
                let name: String?
                /// Milliseconds.
                let duration: Int?
            }
            let songs: [Song]?
        }
        let code: Int?
        let result: Result?
    }

    private struct LyricResponse: Decodable {
        struct Lrc: Decodable { let lyric: String? }
        let code: Int?
        let lrc: Lrc?
    }

    private static let referer = ["Referer": "https://music.163.com/"]

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        let search = LyricsHTTP.request(Self.searchURL(for: track), timeout: timeout, headers: Self.referer)
        let searchBody = try await LyricsHTTP.successBody(search, using: session)
        guard let songID = try Self.selectSongID(from: searchBody, for: track) else { return nil }

        let lyric = LyricsHTTP.request(Self.lyricURL(songID: songID), timeout: timeout, headers: Self.referer)
        let lyricBody = try await LyricsHTTP.successBody(lyric, using: session)
        return try Self.parseLyric(lyricBody)
    }

    static func searchURL(for track: TrackIdentity) -> URL {
        let n = track.normalized
        return LyricsHTTP.url("https://music.163.com/api/search/get/web", query: [
            (name: "s", value: "\(n.title) \(n.artist)"),
            (name: "type", value: "1"),
            (name: "limit", value: "5"),
        ])
    }

    static func lyricURL(songID: Int) -> URL {
        LyricsHTTP.url("https://music.163.com/api/song/lyric", query: [
            (name: "id", value: String(songID)),
            (name: "lv", value: "1"),
            (name: "kv", value: "1"),
            (name: "tv", value: "-1"),
        ])
    }

    /// The first search hit whose title and length fit the query, or nil when
    /// none does. A hit with a different title is never accepted: wrong lyrics
    /// are worse than none. Titles are compared loosely because this service
    /// answers simplified-Chinese titles to traditional queries.
    ///
    /// Throws when the body is not a successful search response.
    static func selectSongID(from data: Data, for track: TrackIdentity) throws -> Int? {
        let response: SearchResponse
        do {
            response = try JSONDecoder().decode(SearchResponse.self, from: data)
        } catch {
            throw LyricsProviderError.badResponse
        }
        if let code = response.code, code != 200 { throw LyricsProviderError.badResponse }
        // Real zero-result responses still carry `code: 200`.
        if response.code == nil, response.result == nil { throw LyricsProviderError.badResponse }

        let wanted = track.normalized.title
        for song in response.result?.songs ?? [] {
            guard let name = song.name else { continue }
            let candidate = TrackIdentity(title: name, artist: "", album: nil, duration: nil).normalized.title
            guard TrackIdentity.loosely(candidate, matches: wanted) else { continue }
            let seconds = song.duration.map { TimeInterval($0) / 1000 }
            guard track.durationMatches(seconds) else { continue }
            return song.id
        }
        return nil
    }

    /// Lyrics from a lyric response, or nil when it carries no original lyric.
    static func decodeLyric(_ data: Data) -> Lyrics? {
        (try? parseLyric(data)) ?? nil
    }

    private static func parseLyric(_ data: Data) throws -> Lyrics? {
        let response: LyricResponse
        do {
            response = try JSONDecoder().decode(LyricResponse.self, from: data)
        } catch {
            throw LyricsProviderError.badResponse
        }
        // A refusal such as `{"code":-460,"msg":"Cheating"}` has no `lrc`, and
        // must not look like a song without lyrics.
        if let code = response.code, code != 200 { throw LyricsProviderError.badResponse }
        guard let raw = response.lrc?.lyric,
              !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              let parsed = LRCParser.parse(raw)
        else { return nil }
        return isPlaceholder(parsed) ? nil : parsed
    }

    /// Instrumentals and uncatalogued songs carry a one-line notice as their
    /// "lyric". Timed, it would count as synced lyrics and stop the provider
    /// walk, so it is treated as no lyrics.
    private static let placeholders = ["纯音乐，请欣赏", "纯音乐, 请欣赏", "暂无歌词", "没有歌词"]

    private static func isPlaceholder(_ lyrics: Lyrics) -> Bool {
        let text: String
        switch lyrics {
        case .synced(let lines):
            guard lines.count == 1 else { return false }
            text = lines[0].text
        case .plain(let plain):
            text = plain
        }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return placeholders.contains { TrackIdentity.loosely($0, matches: trimmed) }
    }
}
