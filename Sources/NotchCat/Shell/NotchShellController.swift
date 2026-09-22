import AppKit
import QuartzCore

/// Owns the notch panel and drives the Hidden/Expanded/Preview state machine
/// described in docs/PLANNING.md.
///
/// Phase 0 has no modules yet, so `hasActiveModule` is always false and
/// Preview is unreachable — the transition is wired up already so Phase 1
/// only has to plug ModuleManager's real "is any module active" signal into
/// `hasActiveModule`.
final class NotchShellController {
    private let geometry: NotchGeometry
    private let panel: NotchPanel
    private let hostingView: NotchHostingView<NotchContentView>

    private(set) var mode: NotchMode = .hidden {
        didSet { render() }
    }

    /// Placeholder for "does any module currently want Preview". Phase 1
    /// replaces this with a real published value from ModuleManager.
    var hasActiveModule: Bool = false

    init(geometry: NotchGeometry) {
        self.geometry = geometry

        let hostingView = NotchHostingView(rootView: NotchContentView(mode: .hidden))
        self.hostingView = hostingView

        let panel = NotchPanel(contentRect: NotchMode.hidden.frame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        hostingView.onMouseEntered = { [weak self] in
            self?.transition(to: .expanded)
        }
        hostingView.onMouseExited = { [weak self] in
            guard let self else { return }
            transition(to: hasActiveModule ? .preview : .hidden)
        }

        panel.orderFrontRegardless()
    }

    func transition(to newMode: NotchMode) {
        guard newMode != mode else { return }
        mode = newMode

        let newFrame = newMode.frame(for: geometry)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.28
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            panel.animator().setFrame(newFrame, display: true)
        }
    }

    private func render() {
        hostingView.rootView = NotchContentView(mode: mode)
    }
}
