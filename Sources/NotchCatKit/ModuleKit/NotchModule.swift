import Combine
import SwiftUI

/// A unit of functionality that can be installed into the shell. The shell
/// only ever talks to modules through this protocol — see
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
protocol NotchModule: AnyObject, Identifiable where ID == String {
    /// Unique identifier, e.g. "com.notchcat.clock".
    var id: String { get }

    /// When multiple modules report `isActive == true` at once, the one
    /// with the highest priority becomes `ModuleManager.activeModule`.
    var priority: Int { get }

    /// Whether this module currently has "ambient" content it wants shown
    /// without the user having to hover (e.g. music currently playing).
    var isActivePublisher: AnyPublisher<Bool, Never> { get }

    /// How much taller than the notch this module wants Preview to be while
    /// it is the active module (e.g. a strip drawn below the notch). The
    /// shell clamps the value to `0...NotchMode.maxPreviewExtraHeight`.
    /// Defaults to a constant 0.
    var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never> { get }

    /// Icon shown in the Expanded AppBar's module switcher.
    func tabIcon() -> AnyView

    /// Content shown in Preview (collapsed) when this module is the active one.
    func previewView() -> AnyView

    /// Content shown in Expanded when this module is the selected one.
    func expandedView() -> AnyView

    /// Called once, right after `ModuleManager.register(_:)` adds this
    /// module to the installed list.
    func onInstall()

    /// Called right after `onInstall()` — v1 has no separate enable/disable
    /// UI, so installing and starting happen back to back.
    func onStart()

    /// Not called anywhere in v1 (no disable UI yet) — defined so modules
    /// can rely on it once that UI exists.
    func onStop()

    /// Not called anywhere in v1 (no uninstall UI yet) — same as above.
    func onRemove()

    /// This module's own settings screen, if it has one. v1 has nowhere to
    /// show it yet.
    func settingsView() -> AnyView?
}

extension NotchModule {
    var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never> {
        Just(0).eraseToAnyPublisher()
    }
}
