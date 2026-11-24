import Foundation

/// Estimates the playback position for sources that cannot report one.
///
/// A position is only known once a track change is seen while observing:
/// the new track then starts at 0 at the moment it was first seen. A track
/// that was already playing when observation began has an unknown start, so
/// `anchor()` is nil for it, never an anchor at 0. Pausing freezes the
/// position and resuming continues from it.
struct PlaybackEstimator {
    private var currentTitle: String?
    private var current: PositionAnchor?

    init() {}

    mutating func observe(title: String?, isPlaying: Bool, at now: Date) {
        guard let title else {
            currentTitle = nil
            current = nil
            return
        }

        if title != currentTitle {
            let isChange = currentTitle != nil
            currentTitle = title
            current = isChange ? PositionAnchor(elapsed: 0, timestamp: now, rate: isPlaying ? 1 : 0) : nil
            return
        }

        guard let anchor = current else { return }
        let rate: Double = isPlaying ? 1 : 0
        guard rate != anchor.rate else { return }
        current = PositionAnchor(elapsed: anchor.position(at: now, cappedTo: nil), timestamp: now, rate: rate)
    }

    /// `nil` while the position cannot be estimated.
    func anchor() -> PositionAnchor? { current }
}
