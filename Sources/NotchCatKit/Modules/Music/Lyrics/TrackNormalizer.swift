import Foundation

/// Turns what a player reports into what a lyrics provider can be asked.
///
/// Browser-hosted players report the video title, which often embeds the
/// channel name, a featured artist and MV-style tags, for example
/// `蘇裔非 - 抱我!  ft. 莊岱諺` from channel `蘇裔非`.
enum TrackNormalizer {
    /// `nil` when the title or artist is missing: without an artist there is no
    /// reliable lookup, so none is attempted.
    static func identity(from snapshot: NowPlayingSnapshot) -> TrackIdentity? {
        let rawTitle = snapshot.title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !rawTitle.isEmpty,
              let artist = snapshot.artist?.trimmingCharacters(in: .whitespacesAndNewlines),
              !artist.isEmpty
        else { return nil }

        return TrackIdentity(
            title: cleanTitle(rawTitle, artist: artist),
            artist: artist,
            album: snapshot.album,
            duration: snapshot.duration
        )
    }

    private static let dashSeparator = try! NSRegularExpression(pattern: #"\s+[-–—]\s+"#)

    /// A bracket group (ASCII or full-width) holding a featured-artist segment
    /// anywhere inside it. The whole group goes, so no opener is left behind.
    private static let bracketedFeat = try! NSRegularExpression(
        pattern: #"\s*[\(\[（【][^\)\]）】]*\b(?:ft\.|feat\.|featuring\s)[^\)\]）】]*[\)\]）】]"#,
        options: .caseInsensitive
    )

    /// A bare `ft.` / `feat.` / `featuring` plus everything after it.
    private static let featSegment = try! NSRegularExpression(
        pattern: #"\s*\b(?:ft\.|feat\.|featuring\s).*$"#,
        options: [.caseInsensitive, .dotMatchesLineSeparators]
    )

    /// Bracketed tags that describe the upload rather than the song.
    private static let videoTag = try! NSRegularExpression(
        pattern: #"\s*[\(\[（【]\s*(?:official\s+)?(?:music\s+video|lyric\s+video|lyrics?\s+video|video|mv|m/v|lyrics?|audio|visualizer|官方[^\)\]）】]*|歌詞[^\)\]）】]*|歌词[^\)\]）】]*)\s*[\)\]）】]"#,
        options: .caseInsensitive
    )

    private static let whitespaceRun = try! NSRegularExpression(pattern: #"\s+"#)

    private static func cleanTitle(_ title: String, artist: String) -> String {
        var result = splitOffArtist(title, artist: artist) ?? title
        result = replace(bracketedFeat, in: result)
        result = replace(featSegment, in: result)
        result = replace(videoTag, in: result)
        result = replace(whitespaceRun, in: result, with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return result.isEmpty ? title : result
    }

    /// `B` of `A - B` when `A` equals or is contained in the artist; `nil` when
    /// the title does not have that shape.
    private static func splitOffArtist(_ title: String, artist: String) -> String? {
        let range = NSRange(title.startIndex..., in: title)
        guard let match = dashSeparator.firstMatch(in: title, range: range),
              let separator = Range(match.range, in: title) else { return nil }
        let head = String(title[..<separator.lowerBound])
        let tail = String(title[separator.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
        let squashedHead = squash(head)
        guard !squashedHead.isEmpty, !tail.isEmpty, squash(artist).contains(squashedHead) else { return nil }
        return tail
    }

    /// Case, width and whitespace insensitive form for the artist comparison.
    private static func squash(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .components(separatedBy: .whitespacesAndNewlines)
            .joined()
    }

    private static func replace(_ regex: NSRegularExpression, in text: String, with template: String = "") -> String {
        regex.stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: template)
    }
}
