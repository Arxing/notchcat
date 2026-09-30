import AppKit
import Combine

/// Published shell mode, observed by `NotchContentView`. A separate object
/// (rather than the controller itself) so it can be constructed before the
/// controller's own `let` properties (panel, hostingView) exist — Swift
/// won't let `self` escape a class init until every stored property is set.
final class NotchShellState: ObservableObject {
    @Published var mode: NotchMode = .preview
}

/// Owns the notch panel and drives the Preview/Expanded state machine
/// described in docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class NotchShellController {
    /// How long to wait after the cursor leaves before actually collapsing.
    /// Without this, a cursor moving faster than the still-growing Expanded
    /// frame repeatedly slips outside its current (mid-animation) bounds,
    /// each slip firing a real mouseExited that reverses the animation —
    /// producing a rapid expand/collapse flicker. Delaying the collapse and
    /// canceling it on re-entry absorbs those transient exits.
    private static let collapseDelay: TimeInterval = 0.15

    private let geometry: NotchGeometry
    private let panel: NotchPanel
    private let hostingView: NotchHostingView<NotchContentView>
    private let state = NotchShellState()
    private var pendingCollapse: DispatchWorkItem?

    var mode: NotchMode { state.mode }

    init(geometry: NotchGeometry) {
        self.geometry = geometry

        // NotchContentView observes `state` directly via @ObservedObject, so
        // mode changes flow through SwiftUI's own Combine-driven update path
        // (which properly picks up its `.animation(value:)` modifier) instead
        // of us reassigning `hostingView.rootView` imperatively from AppKit —
        // that path turned out not to animate reliably.
        let hostingView = NotchHostingView(rootView: NotchContentView(state: state, geometry: geometry))
        self.hostingView = hostingView

        // Fixed for the panel's whole lifetime — see NotchMode.containerFrame.
        // Only the drawn content (NotchContentView) animates its size within it.
        let panel = NotchPanel(contentRect: NotchMode.containerFrame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        // Opening requires a precise hover over the visible Preview pill.
        hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)

        hostingView.onMouseEntered = { [weak self] in
            guard let self else { return }
            pendingCollapse?.cancel()
            pendingCollapse = nil
            transition(to: .expanded)
            // Once open, widen the hit-test rect to the whole fixed
            // container so ordinary cursor jitter can't push the cursor
            // outside it — only actually leaving the shell's full footprint
            // counts as an exit from here on.
            hostingView.hoverRect = CGRect(origin: .zero, size: NotchMode.containerSize(for: geometry))
        }
        hostingView.onMouseExited = { [weak self] in
            guard let self else { return }
            let collapse = DispatchWorkItem { [weak self] in
                guard let self else { return }
                transition(to: .preview)
                hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)
            }
            pendingCollapse = collapse
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseDelay, execute: collapse)
        }

        panel.orderFrontRegardless()
    }

    func transition(to newMode: NotchMode) {
        guard newMode != state.mode else { return }
        state.mode = newMode
    }
}
