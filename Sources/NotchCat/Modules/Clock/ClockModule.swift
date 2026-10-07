import Combine
import SwiftUI

/// Phase 1 proof-of-concept module: exists only to prove `ModuleManager` and
/// the installation mechanism work end to end (see docs/PLANNING.md §8,
/// Phase 1). It never reports itself active, so it never reaches Preview —
/// it only ever appears in Expanded, as the fallback-to-first-installed-
/// module content (see `NotchContentView`).
final class ClockModule: NotchModule {
    let id = "com.notchcat.clock"
    let priority = 0

    var isActivePublisher: AnyPublisher<Bool, Never> {
        Just(false).eraseToAnyPublisher()
    }

    func expandedView() -> AnyView {
        AnyView(ClockExpandedView())
    }

    func previewView() -> AnyView {
        AnyView(EmptyView())
    }
}

private struct ClockExpandedView: View {
    @State private var now = Date()
    private let timer = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        Text(now, style: .time)
            .font(.system(size: 28, weight: .medium, design: .rounded))
            .foregroundStyle(.white)
            .onReceive(timer) { now = $0 }
    }
}
