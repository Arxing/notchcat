import SwiftUI

/// Renders the shell's current mode. Hidden/Expanded/Preview shapes and
/// sizing are the shell's own concern (see `NotchMode`); the *content* drawn
/// inside Expanded/Preview comes from whichever module `ModuleManager` says
/// is active, or — in Expanded only — the first installed module as a
/// fallback, so there's something to look at before any module goes active
/// (see docs/PLANNING.md §4.2). Preview never falls back: it only ever shows
/// the active module, because Preview's entire reason to exist is ambient
/// "a module wants your attention" content.
struct NotchContentView: View {
    /// Observed directly (rather than taking `mode` as a plain value) so
    /// mode changes flow through SwiftUI's own update pipeline, which is
    /// what makes `.animation(value:)` below actually animate — see
    /// NotchShellController.
    @ObservedObject var state: NotchShellState
    @ObservedObject var moduleManager: ModuleManager
    let geometry: NotchGeometry

    var body: some View {
        let mode = state.mode
        let size = mode.size(for: geometry)
        let radius: CGFloat = mode == .hidden ? 10 : 24
        // Top corners stay square — the shape is flush against the screen's
        // top edge (same anchor as the physical notch), so rounding them
        // would visually detach it from the notch instead of reading as an
        // extension of it.
        UnevenRoundedRectangle(topLeadingRadius: 0,
                                bottomLeadingRadius: radius,
                                bottomTrailingRadius: radius,
                                topTrailingRadius: 0,
                                style: .continuous)
            .fill(Color.black)
            .overlay(alignment: .center) {
                switch mode {
                case .hidden:
                    EmptyView()
                case .expanded:
                    expandedContent
                case .preview:
                    previewContent
                }
            }
            .frame(width: size.width, height: size.height)
            // The panel itself is fixed to the largest footprint (see
            // NotchMode.containerFrame); only this shape's own size animates,
            // anchored to the container's top-center to match every mode's
            // shared anchor point.
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .animation(.easeInOut(duration: 0.28), value: mode)
    }

    @ViewBuilder
    private var expandedContent: some View {
        if let module = moduleManager.activeModule ?? moduleManager.installedModules.first {
            module.expandedView()
        } else {
            Text("NotchCat — no module installed")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        }
    }

    @ViewBuilder
    private var previewContent: some View {
        if let module = moduleManager.activeModule {
            module.previewView()
        } else {
            EmptyView()
        }
    }
}
