import Foundation

/// Free, keyless, community-maintained LRC database.
///
/// Queried exactly first, then by search: the exact lookup can miss purely
/// because the system reports a title without the `(feat. …)` suffix the
/// database carries. A 404 from the exact endpoint is a clean "not found" and
/// moves on to the search; any other failure is thrown.
struct LRCLIBProvider: LyricsProvider {
    let id = "net.lrclib"
    private let session: URLSession
    private let timeout: TimeInterval

    init(session: URLSession = .shared, timeout: TimeInterval = 5) {
        self.session = session
        self.timeout = timeout
    }

    private struct Payload: Decodable {
        let trackName: String?
        let duration: Double?
        let plainLyrics: String?
        let syncedLyrics: String?
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        let exact = LyricsHTTP.request(Self.exactURL(for: track), timeout: timeout)
        let (status, body) = try await LyricsHTTP.send(exact, using: session)
        if (200..<300).contains(status) {
            let payload = try Self.payload(from: body)
            if let lyrics = Self.lyrics(from: payload, for: track) { return lyrics }
        } else if status != 404 {
            throw LyricsProviderError.badStatus(status)
        }

        let search = LyricsHTTP.request(Self.searchURL(for: track), timeout: timeout)
        let found = try await LyricsHTTP.successBody(search, using: session)
        return Self.selectCandidate(from: try Self.payloads(from: found), for: track)
    }

    static func exactURL(for track: TrackIdentity) -> URL {
        let n = track.normalized
        var query = [
            (name: "track_name", value: n.title),
            (name: "artist_name", value: n.artist),
        ]
        if let album = n.album, !album.isEmpty {
            query.append((name: "album_name", value: album))
        }
        if let duration = n.duration, duration > 0 {
            query.append((name: "duration", value: String(Int(duration.rounded()))))
        }
        return LyricsHTTP.url("https://lrclib.net/api/get", query: query)
    }

    static func searchURL(for track: TrackIdentity) -> URL {
        let n = track.normalized
        return LyricsHTTP.url("https://lrclib.net/api/search", query: [(name: "q", value: "\(n.title) \(n.artist)")])
    }

    /// Lyrics from a single record, or nil when it has none or is the wrong length.
    static func decode(_ data: Data, for track: TrackIdentity) -> Lyrics? {
        guard let payload = try? payload(from: data) else { return nil }
        return lyrics(from: payload, for: track)
    }

    /// The first record of a search result that passes `decode`'s checks and
    /// whose title matches the query.
    static func decodeSearch(_ data: Data, for track: TrackIdentity) -> Lyrics? {
        guard let payloads = try? payloads(from: data) else { return nil }
        return selectCandidate(from: payloads, for: track)
    }

    private static func payload(from data: Data) throws -> Payload {
        do {
            return try JSONDecoder().decode(Payload.self, from: data)
        } catch {
            throw LyricsProviderError.badResponse
        }
    }

    private static func payloads(from data: Data) throws -> [Payload] {
        do {
            return try JSONDecoder().decode([Payload].self, from: data)
        } catch {
            throw LyricsProviderError.badResponse
        }
    }

    /// The search is fuzzy and can return another song, a remix or a cover, so
    /// unlike the exact lookup (where the server matches names itself) every
    /// candidate must carry a title that loosely matches the query. Better no
    /// lyrics than wrong lyrics.
    private static func selectCandidate(from payloads: [Payload], for track: TrackIdentity) -> Lyrics? {
        let wanted = track.normalized.title
        for payload in payloads {
            guard let name = payload.trackName, !name.isEmpty else { continue }
            let candidate = TrackIdentity(title: name, artist: "", album: nil, duration: nil).normalized.title
            guard TrackIdentity.loosely(candidate, matches: wanted) else { continue }
            if let lyrics = lyrics(from: payload, for: track) { return lyrics }
        }
        return nil
    }

    /// Timed lyrics win over plain text; the length check applies only when
    /// both sides report a positive length.
    private static func lyrics(from payload: Payload, for track: TrackIdentity) -> Lyrics? {
        guard track.durationMatches(payload.duration) else { return nil }
        if let synced = payload.syncedLyrics, let parsed = LRCParser.parse(synced), parsed.isSynced {
            return parsed
        }
        if let plain = payload.plainLyrics, !plain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return .plain(plain)
        }
        return nil
    }
}
