import Combine
import SwiftUI

/// Validation module: proves the register → activeModule → view rendering
/// pipeline works end to end. Always reports active, so it's the shell's
/// default content until a real ambient-content module (e.g. Music, in a
/// later phase) exists. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class ClockModule: NotchModule {
    let id = "com.notchcat.clock"
    let priority = 100

    /// Always active — this is what lets the manual verification steps in
    /// this plan confirm the whole pipeline works without needing a real
    /// "ambient content" source yet.
    var isActivePublisher: AnyPublisher<Bool, Never> {
        Just(true).eraseToAnyPublisher()
    }

    func tabIcon() -> AnyView {
        AnyView(Image(systemName: "clock").foregroundStyle(.white))
    }

    func previewView() -> AnyView {
        AnyView(ClockText(style: .short))
    }

    func expandedView() -> AnyView {
        AnyView(ClockText(style: .long))
    }

    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}

private struct ClockText: View {
    enum Style {
        case short
        case long
    }

    let style: Style

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(context.date, format: style == .short
                 ? .dateTime.hour().minute()
                 : .dateTime.hour().minute().second())
                .font(.system(size: style == .short ? 10 : 16, weight: .medium, design: .rounded))
                .foregroundStyle(.white)
        }
    }
}
