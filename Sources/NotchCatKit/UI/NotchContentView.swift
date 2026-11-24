import SwiftUI

/// Draws the shell's shape and hosts whatever content the current mode
/// should show, sourced from ModuleManager. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
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
        let extra = NotchMode.clampedPreviewExtra(state.previewExtraHeight)
        let size = mode.size(for: geometry, previewExtraHeight: extra)
        // With extra Preview height, the shell fills only the notch-height
        // band; the band below belongs to the module, which draws its own
        // (translucent) background there.
        let hasExtraStrip = mode == .preview && extra > 0
        let shapeHeight = hasExtraStrip ? geometry.notchSize.height : size.height
        let radius: CGFloat = mode == .preview ? 10 : 24
        let bottomRadius: CGFloat = hasExtraStrip ? 0 : radius
        // Top corners stay square — the shape is flush against the screen's
        // top edge (same anchor as the physical notch), so rounding them
        // would visually detach it from the notch instead of reading as an
        // extension of it.
        ZStack(alignment: .top) {
            UnevenRoundedRectangle(topLeadingRadius: 0,
                                    bottomLeadingRadius: bottomRadius,
                                    bottomTrailingRadius: bottomRadius,
                                    topTrailingRadius: 0,
                                    style: .continuous)
                .fill(Color.black)
                .frame(width: size.width, height: shapeHeight)
            // Preview content fills the whole Preview size; only Expanded
            // is inset from the top.
            content(for: mode)
                .padding(.top, mode == .expanded ? 8 : 0)
                .frame(width: size.width, height: size.height, alignment: .top)
        }
        .frame(width: size.width, height: size.height, alignment: .top)
        // The panel itself is fixed to the largest footprint (see
        // NotchMode.containerFrame); only this shape's own size animates,
        // anchored to the container's top-center to match every mode's
        // shared anchor point.
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .animation(.easeInOut(duration: 0.28), value: mode)
        .animation(.easeInOut(duration: 0.28), value: extra)
    }

    @ViewBuilder
    private func content(for mode: NotchMode) -> some View {
        switch mode {
        case .preview:
            if let activeModule = moduleManager.activeModule {
                activeModule.previewView()
            } else {
                DefaultShellContent(mode: .preview)
            }
        case .expanded:
            VStack(spacing: 8) {
                ModuleAppBar(moduleManager: moduleManager)
                if let selectedModule = moduleManager.selectedModule {
                    selectedModule.expandedView()
                } else {
                    DefaultShellContent(mode: .expanded)
                }
            }
        }
    }
}
