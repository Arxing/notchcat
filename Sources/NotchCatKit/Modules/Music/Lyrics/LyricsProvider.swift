import Foundation

/// One source of lyrics. The module never names a concrete provider: adding,
/// removing or reordering sources means editing the array handed to
/// `LyricsRepository` and nothing else.
protocol LyricsProvider: Sendable {
    var id: String { get }
    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics?
}

/// What the rest of the module depends on, so tests and previews can stand in
/// for the real repository.
protocol LyricsResolving: Sendable {
    func lyrics(for track: TrackIdentity) async -> Lyrics?
}
