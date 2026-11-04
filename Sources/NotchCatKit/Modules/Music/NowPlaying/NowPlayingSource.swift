import Combine

/// Something that can report the current now-playing snapshot.
/// Publishes `nil` when nothing is playing.
protocol NowPlayingSource {
    var id: String { get }
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { get }
}
