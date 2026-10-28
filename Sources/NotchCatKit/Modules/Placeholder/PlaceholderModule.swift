import Combine
import SwiftUI

/// Validation module: never auto-activates, but stays installed and
/// selectable in the Expanded AppBar — proves a module can be "installed but
/// not active" and still be manually switched to. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class PlaceholderModule: NotchModule {
    let id = "com.notchcat.placeholder"
    let priority = 0

    var isActivePublisher: AnyPublisher<Bool, Never> {
        Just(false).eraseToAnyPublisher()
    }

    func tabIcon() -> AnyView {
        AnyView(Image(systemName: "square.dashed").foregroundStyle(.white))
    }

    func previewView() -> AnyView {
        AnyView(EmptyView())
    }

    func expandedView() -> AnyView {
        AnyView(
            Text("Placeholder module")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        )
    }

    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}
