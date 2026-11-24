import AppKit
import Combine

/// Published shell mode, observed by `NotchContentView`. A separate object
/// (rather than the controller itself) so it can be constructed before the
/// controller's own `let` properties (panel, hostingView) exist — Swift
/// won't let `self` escape a class init until every stored property is set.
final class NotchShellState: ObservableObject {
    @Published var mode: NotchMode = .preview
    /// Extra Preview height requested by the active module (already clamped
    /// by `ModuleManager`). Only affects Preview.
    @Published var previewExtraHeight: CGFloat = 0
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
    private let moduleManager: ModuleManager
    private var pendingCollapse: DispatchWorkItem?
    private var previewExtraHeightCancellable: AnyCancellable?

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
        let hostingView = NotchHostingView(rootView: NotchContentView(state: state, moduleManager: moduleManager, geometry: geometry))
        self.hostingView = hostingView

        // Fixed for the panel's whole lifetime — see NotchMode.containerFrame.
        // Only the drawn content (NotchContentView) animates its size within it.
        let panel = NotchPanel(contentRect: NotchMode.containerFrame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        // Opening requires a precise hover over the visible Preview pill.
        hostingView.hoverRect = previewHoverRect()

        hostingView.onMouseEntered = { [weak self] in
            guard let self else { return }
            pendingCollapse?.cancel()
            pendingCollapse = nil
            // Every fresh hover resets the AppBar's selection to whatever's
            // currently active — see the design spec's "selectedModule"
            // behavior. A manual pick from a previous expansion never
            // carries over.
            moduleManager.selectedModule = moduleManager.activeModule
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
                hostingView.hoverRect = previewHoverRect()
            }
            pendingCollapse = collapse
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseDelay, execute: collapse)
        }

        // Mirror the active module's requested extra Preview height into
        // the view state, and — while collapsed — grow/shrink the hover
        // rect with it right away so the extra strip counts as hover.
        // Expanded keeps the whole container as its hover rect.
        previewExtraHeightCancellable = moduleManager.$previewExtraHeight
            .sink { [weak self] height in
                guard let self else { return }
                state.previewExtraHeight = height
                if state.mode == .preview {
                    hostingView.hoverRect = previewHoverRect()
                }
            }

        panel.orderFrontRegardless()
    }

    /// Preview's hover rect at the current extra height.
    private func previewHoverRect() -> CGRect {
        NotchMode.preview.localRect(for: geometry, previewExtraHeight: state.previewExtraHeight)
    }

    func transition(to newMode: NotchMode) {
        guard newMode != state.mode else { return }
        state.mode = newMode
    }
}
