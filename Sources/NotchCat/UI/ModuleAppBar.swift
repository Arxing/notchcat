import SwiftUI

/// The Expanded-only row of installed-module icons. Tapping one sets
/// `ModuleManager.selectedModule`, switching which module's `expandedView()`
/// is shown below it. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
struct ModuleAppBar: View {
    @ObservedObject var moduleManager: ModuleManager

    var body: some View {
        HStack(spacing: 12) {
            ForEach(moduleManager.installedModules, id: \.id) { module in
                Button {
                    moduleManager.select(module)
                } label: {
                    module.tabIcon()
                        .opacity(isSelected(module) ? 1 : 0.4)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func isSelected(_ module: any NotchModule) -> Bool {
        moduleManager.selectedModule?.id == module.id
    }
}
