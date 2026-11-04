import Foundation

/// Identifies a piece of artwork, optionally carrying its bytes.
struct ArtworkRef: Equatable {
    let identifier: String
    let data: Data?
}

/// A known playback position at a moment in time, plus the rate it advances at.
/// The current position is derived, never stored.
struct PositionAnchor: Equatable {
    let elapsed: TimeInterval
    let timestamp: Date
    let rate: Double

    /// `max(0, elapsed + max(0, now - timestamp) * rate)`, capped at `duration` when one is given.
    /// A `now` earlier than `timestamp` never rewinds below `elapsed`.
    func position(at now: Date, cappedTo duration: TimeInterval?) -> TimeInterval {
        let advanced = max(0, elapsed + max(0, now.timeIntervalSince(timestamp)) * rate)
        guard let duration else { return advanced }
        return min(advanced, duration)
    }
}

/// What a source reports about the current track.
struct NowPlayingSnapshot: Equatable {
    let title: String
    let artist: String?
    let album: String?
    let duration: TimeInterval?
    let isPlaying: Bool
    /// `nil` when the playback position is unknown.
    let position: PositionAnchor?
    let artwork: ArtworkRef?
    let sourceID: String
}
