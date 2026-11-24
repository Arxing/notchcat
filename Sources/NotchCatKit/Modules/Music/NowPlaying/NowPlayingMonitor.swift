import Combine
import Foundation

/// Combines now-playing sources into one `current` snapshot.
///
/// Sources are listed in priority order. The first one whose latest value is
/// non-nil wins, even when that snapshot is paused. A source that has not
/// published yet counts as nil. The winner's artwork is completed from
/// `ArtworkCache`.
///
/// Updates are applied synchronously on whatever queue a source publishes on;
/// sources publish on the main queue, which is where this is used.
final class NowPlayingMonitor: ObservableObject {
    @Published private(set) var current: NowPlayingSnapshot?

    /// `current?.isPlaying ?? false`, without consecutive repeats.
    var isPlayingPublisher: AnyPublisher<Bool, Never> {
        $current
            .map { $0?.isPlaying ?? false }
            .removeDuplicates()
            .eraseToAnyPublisher()
    }

    private let artwork: ArtworkCache
    /// Latest value per source, indexed like the `sources` passed to `init`.
    private var latest: [NowPlayingSnapshot?]
    private var cancellables: Set<AnyCancellable> = []

    init(sources: [NowPlayingSource], artwork: ArtworkCache = ArtworkCache()) {
        self.artwork = artwork
        self.latest = Array(repeating: nil, count: sources.count)
        for (index, source) in sources.enumerated() {
            source.snapshots
                .sink { [weak self] snapshot in
                    self?.latest[index] = snapshot
                    self?.recompute()
                }
                .store(in: &cancellables)
        }
    }

    private func recompute() {
        var winner = latest.lazy.compactMap { $0 }.first
        if let snapshot = winner {
            winner = NowPlayingSnapshot(
                title: snapshot.title, artist: snapshot.artist, album: snapshot.album,
                duration: snapshot.duration, isPlaying: snapshot.isPlaying,
                position: snapshot.position, artwork: artwork.resolve(snapshot.artwork),
                sourceID: snapshot.sourceID)
        }
        if winner != current { current = winner }
    }
}
