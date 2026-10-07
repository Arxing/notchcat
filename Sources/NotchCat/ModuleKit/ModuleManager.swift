import Combine

/// Holds every installed module and tracks which one (if any) currently owns
/// the shell's Preview/Expanded content.
///
/// Phase 1 scope: modules are installed once at startup (see main.swift) —
/// there is no dynamic enable/disable UI yet (see docs/PLANNING.md §4.3, §9).
final class ModuleManager: ObservableObject {
    @Published private(set) var activeModule: (any NotchModule)?
    private(set) var installedModules: [any NotchModule] = []

    private var activeStates: [String: Bool] = [:]
    private var cancellables: [String: AnyCancellable] = [:]

    func install(_ module: some NotchModule) {
        installedModules.append(module)
        activeStates[module.id] = false
        module.onInstall()
        module.onStart()

        cancellables[module.id] = module.isActivePublisher
            .receive(on: DispatchQueue.main)
            .sink { [weak self] isActive in
                self?.activeStates[module.id] = isActive
                self?.recomputeActiveModule()
            }
    }

    /// Among active modules, the highest-priority one wins; ties keep
    /// whichever `max` happens to return first (undefined among equals —
    /// fine for Phase 1's single-module case, worth revisiting once two
    /// modules can actually tie).
    private func recomputeActiveModule() {
        activeModule = installedModules
            .filter { activeStates[$0.id] == true }
            .max { $0.priority < $1.priority }
    }
}
