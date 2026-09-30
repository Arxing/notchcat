import Combine
import Foundation

/// Holds every installed module and figures out which one is currently
/// "active" (has ambient content to show without a hover). The shell only
/// ever talks to modules through this type — see
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class ModuleManager: ObservableObject {
    @Published private(set) var installedModules: [any NotchModule] = []
    @Published private(set) var activeModule: (any NotchModule)?
    @Published var selectedModule: (any NotchModule)?

    private var isActiveByID: [String: Bool] = [:]
    private var cancellables: [String: AnyCancellable] = [:]

    /// Installs a module: adds it to `installedModules`, then calls
    /// `onInstall()` followed by `onStart()`. v1 has no enable/disable UI,
    /// so registering a module starts it immediately.
    func register(_ module: any NotchModule) {
        installedModules.append(module)
        module.onInstall()
        module.onStart()

        isActiveByID[module.id] = false
        cancellables[module.id] = module.isActivePublisher
            .receive(on: DispatchQueue.main)
            .sink { [weak self] isActive in
                self?.isActiveByID[module.id] = isActive
                self?.recomputeActiveModule()
            }
    }

    /// Sets which module's Expanded content the AppBar currently shows.
    func select(_ module: any NotchModule) {
        selectedModule = module
    }

    private func recomputeActiveModule() {
        activeModule = installedModules
            .filter { isActiveByID[$0.id] == true }
            .max { $0.priority < $1.priority }
    }
}
