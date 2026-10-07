import AppKit
import Combine

/// Published shell mode, observed by `NotchContentView`. A separate object
/// (rather than the controller itself) so it can be constructed before the
/// controller's own `let` properties (panel, hostingView) exist — Swift
/// won't let `self` escape a class init until every stored property is set.
final class NotchShellState: ObservableObject {
    @Published var mode: NotchMode = .hidden
}

/// Owns the notch panel and drives the Hidden/Expanded/Preview state machine
/// described in docs/PLANNING.md.
final class NotchShellController {
    /// How long to wait after the cursor leaves before actually collapsing.
    /// Without this, a cursor moving faster than the still-growing Expanded
    /// frame repeatedly slips outside its current (mid-animation) bounds,
    /// each slip firing a real mouseExited that reverses the animation —
    /// producing a rapid expand/collapse flicker. Delaying the collapse and
    /// canceling it on re-entry absorbs those transient exits.
    private static let collapseDelay: TimeInterval = 0.15

    private let geometry: NotchGeometry
    private let moduleManager: ModuleManager
    private let panel: NotchPanel
    private let hostingView: NotchHostingView<NotchContentView>
    private let state = NotchShellState()
    private var pendingCollapse: DispatchWorkItem?
    private var activeModuleCancellable: AnyCancellable?

    var mode: NotchMode { state.mode }

    init(geometry: NotchGeometry, moduleManager: ModuleManager) {
        self.geometry = geometry
        self.moduleManager = moduleManager

        // NotchContentView observes `state`/`moduleManager` directly via
        // @ObservedObject, so mode and module changes flow through SwiftUI's
        // own Combine-driven update path (which properly picks up its
        // `.animation(value:)` modifier) instead of us reassigning
        // `hostingView.rootView` imperatively from AppKit — that path turned
        // out not to animate reliably.
        let hostingView = NotchHostingView(
            rootView: NotchContentView(state: state, moduleManager: moduleManager, geometry: geometry)
        )
        self.hostingView = hostingView

        // Fixed for the panel's whole lifetime — see NotchMode.containerFrame.
        // Only the drawn content (NotchContentView) animates its size within it.
        let panel = NotchPanel(contentRect: NotchMode.containerFrame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        // Opening requires a precise hover over the visible Hidden pill.
        hostingView.hoverRect = NotchMode.hidden.localRect(for: geometry)

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
                let target = restingMode()
                transition(to: target)
                hostingView.hoverRect = target.localRect(for: geometry)
            }
            pendingCollapse = collapse
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseDelay, execute: collapse)
        }

        // Hidden ⇄ Preview is driven entirely by whether a module is active,
        // never by hover — hover only ever leads to/from Expanded (above).
        // If a module becomes (in)active while the shell is at rest, follow
        // it immediately; if the shell is mid-hover (Expanded), leave it
        // alone and let the next hover-exit's `restingMode()` pick up the
        // new state instead of yanking content out from under the cursor.
        activeModuleCancellable = moduleManager.$activeModule
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                guard let self, state.mode != .expanded else { return }
                let target = restingMode()
                guard target != state.mode else { return }
                transition(to: target)
                hostingView.hoverRect = target.localRect(for: geometry)
            }

        panel.orderFrontRegardless()
    }

    private func restingMode() -> NotchMode {
        moduleManager.activeModule != nil ? .preview : .hidden
    }

    func transition(to newMode: NotchMode) {
        guard newMode != state.mode else { return }
        state.mode = newMode
    }
}
