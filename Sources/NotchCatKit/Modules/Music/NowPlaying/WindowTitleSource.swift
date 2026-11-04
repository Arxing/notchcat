import Combine
import CoreGraphics
import Foundation

/// Fallback now-playing source that reads the YouTube Music window title.
///
/// It reports no artist, album, duration or artwork, and a position anchor
/// only after seeing a track change (see `PlaybackEstimator`). Polls every
/// second while something is playing and every five seconds otherwise.
///
/// Not thread-safe: call it, and run the scheduled work, on one queue (the
/// main queue by default).
final class WindowTitleSource: NowPlayingSource {
    let id = "windowtitle"

    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { subject.eraseToAnyPublisher() }

    static let playingPollInterval: TimeInterval = 1
    static let idlePollInterval: TimeInterval = 5

    private let windows: () -> [WindowInfo]
    private let now: () -> Date
    private let schedule: (TimeInterval, @escaping () -> Void) -> Void
    private let subject = CurrentValueSubject<NowPlayingSnapshot?, Never>(nil)

    private var estimator = PlaybackEstimator()
    private var isRunning = false
    /// Bumped on `start()` and `stop()`, so a poll scheduled before either is ignored.
    private var generation = 0

    init(windows: @escaping () -> [WindowInfo] = WindowTitleSource.systemWindows,
         now: @escaping () -> Date = Date.init,
         schedule: @escaping (TimeInterval, @escaping () -> Void) -> Void = {
             DispatchQueue.main.asyncAfter(deadline: .now() + $0, execute: $1)
         }) {
        self.windows = windows
        self.now = now
        self.schedule = schedule
    }

    func start() {
        guard !isRunning else { return }
        isRunning = true
        generation += 1
        poll(generation: generation)
    }

    /// Stops polling, forgets the estimator state and publishes nil.
    func stop() {
        isRunning = false
        generation += 1
        estimator = PlaybackEstimator()
        subject.send(nil)
    }

    private func poll(generation pollGeneration: Int) {
        guard isRunning, pollGeneration == generation else { return }

        // Prefer a window that is playing over one that is merely open.
        let tracks = windows().compactMap(WindowTitleParser.parse)
        let track = tracks.first(where: \.isPlaying) ?? tracks.first
        estimator.observe(title: track?.title, isPlaying: track?.isPlaying ?? false, at: now())

        let snapshot = track.map {
            NowPlayingSnapshot(title: $0.title, artist: nil, album: nil, duration: nil,
                               isPlaying: $0.isPlaying, position: estimator.anchor(),
                               artwork: nil, sourceID: id)
        }
        if snapshot != subject.value { subject.send(snapshot) }

        let interval = (track?.isPlaying ?? false) ? Self.playingPollInterval : Self.idlePollInterval
        schedule(interval) { [weak self] in self?.poll(generation: pollGeneration) }
    }

    /// Titled windows of all apps, including ones on other Spaces or minimized
    /// (about 9 ms per call). The owner name is always present; `kCGWindowName`
    /// can be absent or empty, in which case the window is skipped.
    static func systemWindows() -> [WindowInfo] {
        let options: CGWindowListOption = [.optionAll, .excludeDesktopElements]
        guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
            return []
        }
        return list.compactMap { entry in
            guard let owner = entry[kCGWindowOwnerName as String] as? String,
                  let title = entry[kCGWindowName as String] as? String,
                  !title.isEmpty else { return nil }
            return WindowInfo(owner: owner, title: title)
        }
    }
}
