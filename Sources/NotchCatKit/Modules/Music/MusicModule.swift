import Combine
import SwiftUI

/// Shows what is playing: cover and lyric strip in Preview, the full player in
/// Expanded.
final class MusicModule: NotchModule {
    /// Height of the lyric strip below the notch in Preview.
    static let lyricStripHeight: CGFloat = 24
    /// Opacity of the strip's black background.
    static let lyricStripOpacity: Double = 0.6

    let id = "com.notchcat.music"
    let priority = 100

    let viewModel: MusicViewModel
    let preference: LyricStripPreference
    /// Width of the notch, for laying out the two wings in Preview.
    let notchWidth: CGFloat

    private let monitor: NowPlayingMonitor
    private let startSources: () -> Void
    private let stopSources: () -> Void

    init(monitor: NowPlayingMonitor, lyrics: LyricsResolving, control: MediaRemoteControlling?,
         preference: LyricStripPreference, notchWidth: CGFloat,
         startSources: @escaping () -> Void = {}, stopSources: @escaping () -> Void = {}) {
        self.monitor = monitor
        self.preference = preference
        self.notchWidth = notchWidth
        self.startSources = startSources
        self.stopSources = stopSources
        self.viewModel = MusicViewModel(monitor: monitor, lyrics: lyrics, control: control,
                                        preference: preference)
    }

    /// Wires the real sources in priority order, the real lyrics providers and
    /// the system transport control (absent when unavailable).
    /// `onStart` and `onStop` must both run on the main thread: the sources are main-queue only.
    static func makeDefault(notchWidth: CGFloat) -> MusicModule {
        let mediaRemote = MediaRemoteSource(launcher: PerlHelperLauncher())
        let windowTitle = WindowTitleSource()
        let monitor = NowPlayingMonitor(sources: [mediaRemote, windowTitle])
        let repository = LyricsRepository(providers: [LRCLIBProvider(), NetEaseProvider()])
        return MusicModule(
            monitor: monitor, lyrics: repository, control: MediaRemoteControl(),
            preference: LyricStripPreference(), notchWidth: notchWidth,
            startSources: {
                mediaRemote.start()
                windowTitle.start()
            },
            stopSources: {
                mediaRemote.stop()
                windowTitle.stop()
            })
    }

    var isActivePublisher: AnyPublisher<Bool, Never> {
        monitor.isPlayingPublisher
    }

    /// The strip's height while something is playing and the strip is not
    /// folded; otherwise 0.
    var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never> {
        monitor.isPlayingPublisher
            .combineLatest(preference.$isVisible)
            .map { isPlaying, isVisible in isPlaying && isVisible ? Self.lyricStripHeight : 0 }
            .removeDuplicates()
            .eraseToAnyPublisher()
    }

    func tabIcon() -> AnyView { AnyView(Image(systemName: "music.note")) }
    func previewView() -> AnyView {
        AnyView(MusicPreviewView(viewModel: viewModel, preference: preference, notchWidth: notchWidth))
    }
    func expandedView() -> AnyView {
        AnyView(MusicExpandedView(viewModel: viewModel, preference: preference))
    }

    func onInstall() {}
    func onStart() { startSources() }
    func onStop() { stopSources() }
    /// Same as `onStop`; stopping twice is harmless.
    func onRemove() { onStop() }
    func settingsView() -> AnyView? { nil }
}
