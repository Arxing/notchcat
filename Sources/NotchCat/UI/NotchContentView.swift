import SwiftUI

/// Fake content for Phase 0: just enough to see each mode's shape and prove
/// the animation reads correctly. Real content arrives with modules in
/// Phase 1+ (see docs/PLANNING.md) via ModuleManager's active module.
struct NotchContentView: View {
    let mode: NotchMode

    var body: some View {
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
    }
}
