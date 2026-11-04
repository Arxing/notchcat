import Foundation

/// A window as seen by the window-title fallback: who owns it and what its title bar says.
struct WindowInfo: Equatable {
    let owner: String
    let title: String
}

/// A track recovered from a window title.
struct ParsedTrack: Equatable {
    let title: String
    let isPlaying: Bool
}

/// Reads the track out of a known player's window title. Pure: no system calls.
///
/// The only format understood is `YouTube Music - <title> | YouTube Music`,
/// with a trailing speaker symbol while audio is playing. Anything else
/// returns nil rather than a partial string, because the format belongs to a
/// third party and may change.
enum WindowTitleParser {
    static let knownOwners: Set<String> = ["YouTube Music"]

    private static let prefix = "YouTube Music - "
    private static let suffix = " | YouTube Music"
    private static let speaker: Character = "🔊"

    static func parse(_ window: WindowInfo) -> ParsedTrack? {
        guard knownOwners.contains(window.owner) else { return nil }

        var text = window.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let isPlaying = text.last == speaker
        if isPlaying {
            text.removeLast()
            text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        guard text.hasPrefix(prefix), text.hasSuffix(suffix),
              text.count >= prefix.count + suffix.count else { return nil }
        let title = String(text.dropFirst(prefix.count).dropLast(suffix.count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { return nil }
        return ParsedTrack(title: title, isPlaying: isPlaying)
    }
}
