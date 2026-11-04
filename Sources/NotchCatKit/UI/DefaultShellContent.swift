import SwiftUI

/// The shell's own placeholder content, shown whenever no module is
/// supplying a preview/expanded view. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
struct DefaultShellContent: View {
    let mode: NotchMode

    var body: some View {
        switch mode {
        case .preview:
            EmptyView()
        case .expanded:
            Text("沒有進行中的內容，點上方圖示開啟模組")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        }
    }
}
