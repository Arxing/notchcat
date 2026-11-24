import Combine
import Foundation

/// Whether the lyric strip below the notch is shown in Preview. Folding it
/// only affects Preview; Expanded keeps its lyrics. Persisted on every change,
/// and visible by default.
final class LyricStripPreference: ObservableObject {
    @Published var isVisible: Bool {
        didSet { defaults.set(isVisible, forKey: key) }
    }

    private let defaults: UserDefaults
    private let key: String

    init(defaults: UserDefaults = .standard, key: String = "music.lyricStripVisible") {
        self.defaults = defaults
        self.key = key
        self.isVisible = defaults.object(forKey: key) as? Bool ?? true
    }
}
