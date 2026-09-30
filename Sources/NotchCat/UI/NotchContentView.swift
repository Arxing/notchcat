import SwiftUI

/// Fake content for Phase 0: just enough to see each mode's shape and prove
/// the animation reads correctly. Task 5 of this plan wires this up to
/// ModuleManager; until then it always shows DefaultShellContent.
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
        let radius: CGFloat = mode == .preview ? 10 : 24
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
                DefaultShellContent(mode: mode)
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
