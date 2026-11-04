import Foundation

/// What the lyric strip shows.
enum StripContent: Equatable {
    /// The current lyric line.
    case line(String)
    /// The track title, used whenever there is no current line to show.
    case title(String)
    /// Nothing is playing.
    case none
}

/// Pure presentation rules, kept apart from the views so they can be tested.
enum MusicPresentation {
    /// Where playback is at `now`, or nil when the source reports no anchor.
    /// Never substitutes 0 for an unknown position.
    static func playbackPosition(snapshot: NowPlayingSnapshot, at now: Date) -> TimeInterval? {
        snapshot.position?.position(at: now, cappedTo: snapshot.duration)
    }

    /// The strip steps down as information goes missing:
    /// synced lyrics with a known position show the current line; anything else
    /// with a track shows the title (plain lyrics are never put in the strip,
    /// and neither is a line before the first one is reached); no track is none.
    static func stripContent(snapshot: NowPlayingSnapshot?, lyrics: Lyrics?, at now: Date) -> StripContent {
        guard let snapshot else { return .none }
        if case .synced(let lines)? = lyrics,
           let position = playbackPosition(snapshot: snapshot, at: now),
           let index = lyrics?.lineIndex(at: position) {
            return .line(lines[index].text)
        }
        return .title(snapshot.title)
    }
}
