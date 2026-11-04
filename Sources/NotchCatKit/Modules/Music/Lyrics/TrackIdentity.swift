import Foundation

/// What a lyrics provider is asked to look up.
///
/// Kept separate from `NowPlayingSnapshot` because lookup only cares about
/// identity, not playback state, and because the normalization rules below
/// exist purely to serve lookup.
struct TrackIdentity: Hashable {
    let title: String
    let artist: String
    let album: String?
    /// `nil` when the source does not report a length.
    let duration: TimeInterval?

    private static let suffixPattern = try! NSRegularExpression(
        pattern: #"\s*[\(\[](feat\.?|ft\.?|remaster(ed)?|live|explicit)[^\)\]]*[\)\]]"#,
        options: .caseInsensitive
    )

    private static let artistSeparators = CharacterSet(charactersIn: "/&,;")

    /// A looser form of the query. An exact lookup can miss purely because the
    /// system reports the title without the `(feat. ...)` suffix the database
    /// carries, or the other way around.
    var normalized: TrackIdentity {
        let range = NSRange(title.startIndex..., in: title)
        let strippedTitle = Self.suffixPattern
            .stringByReplacingMatches(in: title, range: range, withTemplate: "")
            .trimmingCharacters(in: .whitespaces)

        let primaryArtist = artist
            .components(separatedBy: Self.artistSeparators)
            .first?
            .trimmingCharacters(in: .whitespaces) ?? artist

        return TrackIdentity(
            title: strippedTitle.isEmpty ? title : strippedTitle,
            artist: primaryArtist.isEmpty ? artist : primaryArtist,
            album: album,
            duration: duration
        )
    }

    /// Normalized title and artist, lowercased, without the length.
    var cacheKey: String {
        let n = normalized
        return "\(n.title.lowercased())\u{1}\(n.artist.lowercased())"
    }

    /// Tolerance rather than equality: an edit of a track is often a few
    /// seconds off the original. A missing or non-positive length on either
    /// side means "unknown", so it never rejects a candidate.
    func durationMatches(_ other: TimeInterval?) -> Bool {
        guard let duration, let other, duration > 0, other > 0 else { return true }
        return abs(duration - other) <= 5
    }

    /// Never compare provider titles with `==`: the same song can come back in
    /// simplified rather than traditional Chinese, in another case, or in full
    /// width characters. Both sides are converted to one script, then folded.
    static func loosely(_ a: String, matches b: String) -> Bool {
        fold(a) == fold(b)
    }

    private static func fold(_ raw: String) -> String {
        let mutable = NSMutableString(string: raw)
        CFStringTransform(mutable as CFMutableString, nil, "Hans-Hant" as CFString, false)
        return (mutable as String)
            .folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
