import Combine
import SwiftUI

/// A unit of functionality that can be installed into the notch shell.
///
/// The shell (`NotchShellController`/`NotchContentView`) never knows about
/// concrete modules — only this protocol, via `ModuleManager`. See
/// docs/PLANNING.md section 4 for the full design rationale.
protocol NotchModule: AnyObject, Identifiable where ID == String {
    /// Unique identifier, e.g. "com.notchcat.music".
    var id: String { get }

    /// Tie-breaker when more than one module wants to own Preview/Expanded
    /// at the same time — higher wins.
    var priority: Int { get }

    /// Whether this module currently has "ambient" content to show (e.g.
    /// music is playing). Drives the shell's automatic Hidden ⇄ Preview
    /// switching — see `ModuleManager` and `NotchShellController`.
    var isActivePublisher: AnyPublisher<Bool, Never> { get }

    /// Small optional adornment drawn over the Hidden shape (e.g. a badge).
    func hiddenAdornment() -> AnyView?
    /// Content shown while the shell is in Preview mode.
    func previewView() -> AnyView
    /// Content shown while the shell is in Expanded mode.
    func expandedView() -> AnyView
    /// Optional settings UI for this module.
    func settingsView() -> AnyView?

    func onInstall()
    func onStart()
    func onStop()
    func onRemove()
}

extension NotchModule {
    func hiddenAdornment() -> AnyView? { nil }
    func settingsView() -> AnyView? { nil }

    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
}
