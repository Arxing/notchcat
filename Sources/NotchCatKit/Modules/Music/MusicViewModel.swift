import Combine
import Foundation

/// Feeds the music views: the current snapshot, the lyrics for that track, and
/// transport commands.
///
/// Lyrics are looked up when the track changes (by `TrackIdentity.cacheKey`),
/// not when only the playing state or position anchor does. Everything runs on
/// the main thread, where `NowPlayingMonitor` publishes.
final class MusicViewModel: ObservableObject {
    @Published private(set) var snapshot: NowPlayingSnapshot?
    @Published private(set) var lyrics: Lyrics?

    private let resolver: LyricsResolving
    private let control: MediaRemoteControlling?
    private let preference: LyricStripPreference
    private var currentKey: String?
    private var lookup: Task<Void, Never>?
    private var cancellables: Set<AnyCancellable> = []

    init(monitor: NowPlayingMonitor, lyrics: LyricsResolving, control: MediaRemoteControlling?,
         preference: LyricStripPreference) {
        self.resolver = lyrics
        self.control = control
        self.preference = preference
        monitor.$current
            .sink { [weak self] in self?.apply($0) }
            .store(in: &cancellables)
    }

    deinit { lookup?.cancel() }

    /// Whether transport commands reach anything; the buttons disable when not.
    var hasControl: Bool { control != nil }

    /// No-op when the system control is unavailable.
    func send(_ command: MediaRemoteCommand) {
        control?.send(command)
    }

    private func apply(_ new: NowPlayingSnapshot?) {
        snapshot = new

        let identity = new.flatMap(TrackNormalizer.identity(from:))
        let key = identity?.cacheKey
        guard key != currentKey else { return }

        currentKey = key
        lookup?.cancel()
        lookup = nil
        lyrics = nil

        guard let identity else { return }
        let resolver = self.resolver
        lookup = Task { [weak self] in
            let result = await resolver.lyrics(for: identity)
            await MainActor.run { [weak self] in
                // The track may have changed while the request was in flight.
                guard let self, self.currentKey == identity.cacheKey else { return }
                self.lyrics = result
            }
        }
    }
}
