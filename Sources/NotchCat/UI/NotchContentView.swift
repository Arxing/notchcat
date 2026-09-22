import SwiftUI

/// Fake content for Phase 0: just enough to see each mode's shape and prove
/// the animation reads correctly. Real content arrives with modules in
/// Phase 1+ (see docs/PLANNING.md) via ModuleManager's active module.
struct NotchContentView: View {
    /// Observed directly (rather than taking `mode` as a plain value) so
    /// mode changes flow through SwiftUI's own update pipeline, which is
    /// what makes `.animation(value:)` below actually animate — see
    /// NotchShellController.
    @ObservedObject var state: NotchShellState
    let geometry: NotchGeometry

    var body: some View {
        let mode = state.mode
        let size = mode.size(for: geometry)
        RoundedRectangle(cornerRadius: mode == .hidden ? 10 : 24, style: .continuous)
            .fill(Color.black)
            .overlay(alignment: .center) {
                switch mode {
                case .hidden:
                    EmptyView()
                case .expanded:
                    Text("NotchCat — Phase 0 (no module installed)")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.white.opacity(0.7))
                case .preview:
                    Text("preview")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.5))
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
}
