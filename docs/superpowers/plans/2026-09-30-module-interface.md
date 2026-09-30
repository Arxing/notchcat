# Module 溝通介面 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Simplify the shell's state machine to two modes (Preview/Expanded) and add a `ModuleKit` layer (`NotchModule` protocol + `ModuleManager`) so functionality can be installed into the shell as independent modules, validated with two minimal demo modules.

**Architecture:** `NotchMode` drops its third case; `ModuleManager` (a new `ObservableObject`) owns the list of installed modules and computes which one is "active" from each module's `isActivePublisher` + `priority`. `NotchContentView` renders `activeModule?.previewView()` in Preview and, in Expanded, a module-switcher AppBar above `selectedModule?.expandedView()`, falling back to `DefaultShellContent` when nothing is active/selected. `NotchShellController` resets `selectedModule` to `activeModule` every time a fresh hover opens the shell.

**Tech Stack:** Swift 5.9, SwiftUI, AppKit (`NSPanel`), Combine (`ObservableObject`/`@Published`/`AnyPublisher`). No automated test target — this project verifies manually via `swift build` / `swift run` on real notch hardware (see spec's "測試方式").

**Spec:** [docs/superpowers/specs/2026-09-23-module-interface-design.md](../specs/2026-09-23-module-interface-design.md)

## Global Constraints

- Minimum target: macOS 14 (`Package.swift` already sets `.macOS(.v14)`).
- State management stays on Combine (`ObservableObject`/`@Published`/`AnyPublisher`) — do not introduce the Observation framework (`@Observable`).
- No automated test target exists or should be added in this plan. Verification is `swift build` (compiles) plus manual `swift run` checks described in each task.
- v1 modules are compiled-in Swift types registered at launch via `ModuleManager.register(_:)` — no dynamic/external module loading.
- The old third shell state's name must never appear anywhere (code, comments, docs) — not even to describe history. If you need to refer to what used to exist, describe it functionally ("Preview absorbs what used to be a separate collapsed-with-no-content state") without naming it.

---

## Task 1: Simplify `NotchMode` to Preview/Expanded and update the Shell/UI to match

**Files:**
- Modify (full rewrite): `Sources/NotchCat/Shell/NotchMode.swift`
- Modify (full rewrite): `Sources/NotchCat/Shell/NotchShellController.swift`
- Modify (full rewrite): `Sources/NotchCat/UI/NotchContentView.swift`

**Interfaces:**
- Produces: `enum NotchMode { case preview, expanded }` with `size(for:)`, `frame(for:)`, `localRect(for:)` instance methods and `containerSize(for:)`, `containerFrame(for:)` static methods — same signatures as before, just two cases instead of three.
- Produces: `NotchShellState.mode: NotchMode` (default `.preview`), `NotchShellController.mode: NotchMode`, `NotchShellController.transition(to:)` — same signatures as before.
- Consumes: `NotchGeometry`, `NotchPanel`, `NotchHostingView` — unchanged from Phase 0.

- [ ] **Step 1: Rewrite `NotchMode.swift`**

```swift
import CoreGraphics

/// The two panel modes. See docs/superpowers/specs/2026-09-23-module-interface-design.md.
enum NotchMode: Equatable {
    case preview
    case expanded
}

extension NotchMode {
    /// How much wider than the physical notch the bar sits, so it reads as a
    /// deliberate bar hugging the notch rather than exactly tracing its
    /// cutout (the boring.notch-style look). Both modes share this same
    /// width — only the height grows downward — matching boring.notch, where
    /// opening the shell doesn't change its width at all.
    private static let widthPadding: CGFloat = 160

    /// Preview's size (the shell's resting state) and Expanded's size
    /// (hover-triggered, holds the module-switcher AppBar plus content).
    func size(for geometry: NotchGeometry) -> CGSize {
        let width = geometry.notchSize.width + Self.widthPadding
        switch self {
        case .preview:
            return CGSize(width: width, height: geometry.notchSize.height)
        case .expanded:
            return CGSize(width: width, height: 180)
        }
    }

    /// Frame for this mode, anchored to the notch's top-center so every mode
    /// grows/shrinks around the same fixed point.
    func frame(for geometry: NotchGeometry) -> CGRect {
        let size = size(for: geometry)
        return CGRect(x: geometry.centerX - size.width / 2,
                       y: geometry.topY - size.height,
                       width: size.width,
                       height: size.height)
    }

    /// The panel's own frame never changes at runtime — see
    /// `NotchShellController`. It's sized to fit every mode's footprint so
    /// the hover hit-test area is always the same stable, generous size:
    /// a hit-test area that shrinks/grows in sync with the mode-change
    /// animation is only as forgiving as its smallest transient size, and
    /// Preview's is a ~32pt-tall sliver — thin enough that ordinary cursor
    /// jitter while "holding still" crosses its edge and causes the shell to
    /// flicker open/closed. Only the drawn content animates its size within
    /// this fixed container.
    static func containerSize(for geometry: NotchGeometry) -> CGSize {
        let modes: [NotchMode] = [.preview, .expanded]
        let sizes = modes.map { $0.size(for: geometry) }
        return CGSize(width: sizes.map(\.width).max() ?? 0,
                       height: sizes.map(\.height).max() ?? 0)
    }

    static func containerFrame(for geometry: NotchGeometry) -> CGRect {
        let size = containerSize(for: geometry)
        return CGRect(x: geometry.centerX - size.width / 2,
                       y: geometry.topY - size.height,
                       width: size.width,
                       height: size.height)
    }

    /// This mode's hover hit-test rect, in the fixed container's own
    /// top-anchored local coordinate space (see `containerFrame`). Used to
    /// require a precise hover over the visible Preview pill to *open* the
    /// shell, while `NotchShellController` widens the active hit-test rect
    /// to the whole container the instant it opens, so staying open is
    /// forgiving. NSHostingView is flipped (origin top-left), matching the
    /// top-anchored layout in NotchContentView.
    func localRect(for geometry: NotchGeometry) -> CGRect {
        let container = NotchMode.containerSize(for: geometry)
        let size = size(for: geometry)
        return CGRect(x: (container.width - size.width) / 2,
                       y: 0,
                       width: size.width,
                       height: size.height)
    }
}
```

- [ ] **Step 2: Rewrite `NotchShellController.swift`**

```swift
import AppKit
import Combine

/// Published shell mode, observed by `NotchContentView`. A separate object
/// (rather than the controller itself) so it can be constructed before the
/// controller's own `let` properties (panel, hostingView) exist — Swift
/// won't let `self` escape a class init until every stored property is set.
final class NotchShellState: ObservableObject {
    @Published var mode: NotchMode = .preview
}

/// Owns the notch panel and drives the Preview/Expanded state machine
/// described in docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class NotchShellController {
    /// How long to wait after the cursor leaves before actually collapsing.
    /// Without this, a cursor moving faster than the still-growing Expanded
    /// frame repeatedly slips outside its current (mid-animation) bounds,
    /// each slip firing a real mouseExited that reverses the animation —
    /// producing a rapid expand/collapse flicker. Delaying the collapse and
    /// canceling it on re-entry absorbs those transient exits.
    private static let collapseDelay: TimeInterval = 0.15

    private let geometry: NotchGeometry
    private let panel: NotchPanel
    private let hostingView: NotchHostingView<NotchContentView>
    private let state = NotchShellState()
    private var pendingCollapse: DispatchWorkItem?

    var mode: NotchMode { state.mode }

    init(geometry: NotchGeometry) {
        self.geometry = geometry

        // NotchContentView observes `state` directly via @ObservedObject, so
        // mode changes flow through SwiftUI's own Combine-driven update path
        // (which properly picks up its `.animation(value:)` modifier) instead
        // of us reassigning `hostingView.rootView` imperatively from AppKit —
        // that path turned out not to animate reliably.
        let hostingView = NotchHostingView(rootView: NotchContentView(state: state, geometry: geometry))
        self.hostingView = hostingView

        // Fixed for the panel's whole lifetime — see NotchMode.containerFrame.
        // Only the drawn content (NotchContentView) animates its size within it.
        let panel = NotchPanel(contentRect: NotchMode.containerFrame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        // Opening requires a precise hover over the visible Preview pill.
        hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)

        hostingView.onMouseEntered = { [weak self] in
            guard let self else { return }
            pendingCollapse?.cancel()
            pendingCollapse = nil
            transition(to: .expanded)
            // Once open, widen the hit-test rect to the whole fixed
            // container so ordinary cursor jitter can't push the cursor
            // outside it — only actually leaving the shell's full footprint
            // counts as an exit from here on.
            hostingView.hoverRect = CGRect(origin: .zero, size: NotchMode.containerSize(for: geometry))
        }
        hostingView.onMouseExited = { [weak self] in
            guard let self else { return }
            let collapse = DispatchWorkItem { [weak self] in
                guard let self else { return }
                transition(to: .preview)
                hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)
            }
            pendingCollapse = collapse
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseDelay, execute: collapse)
        }

        panel.orderFrontRegardless()
    }

    func transition(to newMode: NotchMode) {
        guard newMode != state.mode else { return }
        state.mode = newMode
    }
}
```

- [ ] **Step 3: Rewrite `NotchContentView.swift`**

```swift
import SwiftUI

/// Fake content for Phase 0: just enough to see each mode's shape and prove
/// the animation reads correctly. Task 5 of this plan wires this up to
/// ModuleManager; until then it always shows this placeholder.
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
                switch mode {
                case .preview:
                    EmptyView()
                case .expanded:
                    Text("NotchCat — Phase 0 (no module installed)")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.white.opacity(0.7))
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
```

- [ ] **Step 4: Build**

Run: `cd /Users/arxing/Workspace/notchcat && swift build`
Expected: `Build complete!` with no errors.

- [ ] **Step 5: Manual verification**

Run: `swift run` (background it if your shell needs the foreground, same as every prior manual-verification round in this project).
Expected, on the real notch: identical behavior to before this task — a small bar hugging the notch at rest, hover expands it with the same "NotchCat — Phase 0 (no module installed)" text, moving away collapses it back. No visual or behavioral difference is expected from this task; it's a pure rename/simplification.

- [ ] **Step 6: Commit**

```bash
cd /Users/arxing/Workspace/notchcat
git add Sources/NotchCat/Shell/NotchMode.swift Sources/NotchCat/Shell/NotchShellController.swift Sources/NotchCat/UI/NotchContentView.swift
git commit -m "Simplify shell state machine to Preview/Expanded"
```

---

## Task 2: Extract `DefaultShellContent`

**Files:**
- Create: `Sources/NotchCat/UI/DefaultShellContent.swift`
- Modify: `Sources/NotchCat/UI/NotchContentView.swift`

**Interfaces:**
- Consumes: `NotchMode` (from Task 1).
- Produces: `struct DefaultShellContent: View { let mode: NotchMode }` — later tasks (Task 5) use this as the "no module" fallback in both Preview and Expanded.

- [ ] **Step 1: Create `DefaultShellContent.swift`**

```swift
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
            Text("NotchCat — Phase 0 (no module installed)")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        }
    }
}
```

- [ ] **Step 2: Update `NotchContentView.swift` to use it**

Replace the `.overlay(alignment: .center) { switch mode { ... } }` block with:

```swift
            .overlay(alignment: .center) {
                DefaultShellContent(mode: mode)
            }
```

The full file after this change:

```swift
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
```

- [ ] **Step 3: Build**

Run: `cd /Users/arxing/Workspace/notchcat && swift build`
Expected: `Build complete!` with no errors.

- [ ] **Step 4: Manual verification**

Run: `swift run`. Expected: pixel-identical behavior to Task 1's end state (this is a pure refactor — the text and shape are unchanged, just moved into a new file).

- [ ] **Step 5: Commit**

```bash
cd /Users/arxing/Workspace/notchcat
git add Sources/NotchCat/UI/DefaultShellContent.swift Sources/NotchCat/UI/NotchContentView.swift
git commit -m "Extract DefaultShellContent as a reusable fallback view"
```

---

## Task 3: Add `ModuleKit` — `NotchModule` protocol and `ModuleManager`

**Files:**
- Create: `Sources/NotchCat/ModuleKit/NotchModule.swift`
- Create: `Sources/NotchCat/ModuleKit/ModuleManager.swift`

**Interfaces:**
- Produces: `protocol NotchModule: AnyObject, Identifiable where ID == String` with `id`, `priority`, `isActivePublisher`, `tabIcon()`, `previewView()`, `expandedView()`, `onInstall()`, `onStart()`, `onStop()`, `onRemove()`, `settingsView()`.
- Produces: `final class ModuleManager: ObservableObject` with `@Published private(set) var installedModules: [any NotchModule]`, `@Published private(set) var activeModule: (any NotchModule)?`, `@Published var selectedModule: (any NotchModule)?`, `func register(_ module: any NotchModule)`, `func select(_ module: any NotchModule)`.
- Consumes: nothing from earlier tasks (this is a standalone addition — nothing in the app calls it yet).

- [ ] **Step 1: Create `NotchModule.swift`**

```swift
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
```

- [ ] **Step 2: Create `ModuleManager.swift`**

```swift
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
```

- [ ] **Step 3: Build**

Run: `cd /Users/arxing/Workspace/notchcat && swift build`
Expected: `Build complete!` with no errors. These two files are additive only — nothing in the app references them yet, so there's no runtime behavior to verify.

- [ ] **Step 4: Commit**

```bash
cd /Users/arxing/Workspace/notchcat
git add Sources/NotchCat/ModuleKit/NotchModule.swift Sources/NotchCat/ModuleKit/ModuleManager.swift
git commit -m "Add ModuleKit: NotchModule protocol and ModuleManager"
```

---

## Task 4: Add the Clock and Placeholder validation modules

**Files:**
- Create: `Sources/NotchCat/Modules/Clock/ClockModule.swift`
- Create: `Sources/NotchCat/Modules/Placeholder/PlaceholderModule.swift`

**Interfaces:**
- Consumes: `protocol NotchModule` (Task 3).
- Produces: `final class ClockModule: NotchModule` (`id = "com.notchcat.clock"`, `priority = 100`, always-active), `final class PlaceholderModule: NotchModule` (`id = "com.notchcat.placeholder"`, `priority = 0`, never-active) — Task 5 registers both.

- [ ] **Step 1: Create `ClockModule.swift`**

```swift
import Combine
import SwiftUI

/// Validation module: proves the register → activeModule → view rendering
/// pipeline works end to end. Always reports active, so it's the shell's
/// default content until a real ambient-content module (e.g. Music, in a
/// later phase) exists. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class ClockModule: NotchModule {
    let id = "com.notchcat.clock"
    let priority = 100

    /// Always active — this is what lets the manual verification steps in
    /// this plan confirm the whole pipeline works without needing a real
    /// "ambient content" source yet.
    var isActivePublisher: AnyPublisher<Bool, Never> {
        Just(true).eraseToAnyPublisher()
    }

    func tabIcon() -> AnyView {
        AnyView(Image(systemName: "clock").foregroundStyle(.white))
    }

    func previewView() -> AnyView {
        AnyView(ClockText(style: .short))
    }

    func expandedView() -> AnyView {
        AnyView(ClockText(style: .long))
    }

    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}

private struct ClockText: View {
    enum Style {
        case short
        case long
    }

    let style: Style

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(context.date, format: style == .short
                 ? .dateTime.hour().minute()
                 : .dateTime.hour().minute().second())
                .font(.system(size: style == .short ? 10 : 16, weight: .medium, design: .rounded))
                .foregroundStyle(.white)
        }
    }
}
```

- [ ] **Step 2: Create `PlaceholderModule.swift`**

```swift
import Combine
import SwiftUI

/// Validation module: never auto-activates, but stays installed and
/// selectable in the Expanded AppBar — proves a module can be "installed but
/// not active" and still be manually switched to. See
/// docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class PlaceholderModule: NotchModule {
    let id = "com.notchcat.placeholder"
    let priority = 0

    var isActivePublisher: AnyPublisher<Bool, Never> {
        Just(false).eraseToAnyPublisher()
    }

    func tabIcon() -> AnyView {
        AnyView(Image(systemName: "square.dashed").foregroundStyle(.white))
    }

    func previewView() -> AnyView {
        AnyView(EmptyView())
    }

    func expandedView() -> AnyView {
        AnyView(
            Text("Placeholder module")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white.opacity(0.7))
        )
    }

    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}
```

- [ ] **Step 3: Build**

Run: `cd /Users/arxing/Workspace/notchcat && swift build`
Expected: `Build complete!` with no errors. Neither module is registered anywhere yet, so there's no runtime behavior to verify.

- [ ] **Step 4: Commit**

```bash
cd /Users/arxing/Workspace/notchcat
git add Sources/NotchCat/Modules/Clock/ClockModule.swift Sources/NotchCat/Modules/Placeholder/PlaceholderModule.swift
git commit -m "Add Clock and Placeholder validation modules"
```

---

## Task 5: Wire `ModuleManager` into the shell and add the Expanded AppBar

**Files:**
- Create: `Sources/NotchCat/UI/ModuleAppBar.swift`
- Modify (full rewrite): `Sources/NotchCat/UI/NotchContentView.swift`
- Modify (full rewrite): `Sources/NotchCat/Shell/NotchShellController.swift`
- Modify: `Sources/NotchCat/main.swift`

**Interfaces:**
- Consumes: `ModuleManager`, `NotchModule` (Task 3), `ClockModule`, `PlaceholderModule` (Task 4), `DefaultShellContent` (Task 2), `NotchMode` (Task 1).
- Produces: `NotchShellController.init(geometry:moduleManager:)` (new parameter), `NotchContentView.init(state:moduleManager:geometry:)` (new parameter), `struct ModuleAppBar: View { @ObservedObject var moduleManager: ModuleManager }`.

- [ ] **Step 1: Create `ModuleAppBar.swift`**

```swift
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
```

- [ ] **Step 2: Rewrite `NotchContentView.swift`**

```swift
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
            .overlay(alignment: .top) {
                content(for: mode)
                    .padding(.top, 8)
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
```

- [ ] **Step 3: Rewrite `NotchShellController.swift`**

```swift
import AppKit
import Combine

/// Published shell mode, observed by `NotchContentView`. A separate object
/// (rather than the controller itself) so it can be constructed before the
/// controller's own `let` properties (panel, hostingView) exist — Swift
/// won't let `self` escape a class init until every stored property is set.
final class NotchShellState: ObservableObject {
    @Published var mode: NotchMode = .preview
}

/// Owns the notch panel and drives the Preview/Expanded state machine
/// described in docs/superpowers/specs/2026-09-23-module-interface-design.md.
final class NotchShellController {
    /// How long to wait after the cursor leaves before actually collapsing.
    /// Without this, a cursor moving faster than the still-growing Expanded
    /// frame repeatedly slips outside its current (mid-animation) bounds,
    /// each slip firing a real mouseExited that reverses the animation —
    /// producing a rapid expand/collapse flicker. Delaying the collapse and
    /// canceling it on re-entry absorbs those transient exits.
    private static let collapseDelay: TimeInterval = 0.15

    private let geometry: NotchGeometry
    private let panel: NotchPanel
    private let hostingView: NotchHostingView<NotchContentView>
    private let state = NotchShellState()
    private let moduleManager: ModuleManager
    private var pendingCollapse: DispatchWorkItem?

    var mode: NotchMode { state.mode }

    init(geometry: NotchGeometry, moduleManager: ModuleManager) {
        self.geometry = geometry
        self.moduleManager = moduleManager

        // NotchContentView observes `state`/`moduleManager` directly via
        // @ObservedObject, so mode and module changes flow through SwiftUI's
        // own Combine-driven update path (which properly picks up its
        // `.animation(value:)` modifier) instead of us reassigning
        // `hostingView.rootView` imperatively from AppKit — that path turned
        // out not to animate reliably.
        let hostingView = NotchHostingView(rootView: NotchContentView(state: state, moduleManager: moduleManager, geometry: geometry))
        self.hostingView = hostingView

        // Fixed for the panel's whole lifetime — see NotchMode.containerFrame.
        // Only the drawn content (NotchContentView) animates its size within it.
        let panel = NotchPanel(contentRect: NotchMode.containerFrame(for: geometry))
        panel.contentView = hostingView
        self.panel = panel

        // Opening requires a precise hover over the visible Preview pill.
        hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)

        hostingView.onMouseEntered = { [weak self] in
            guard let self else { return }
            pendingCollapse?.cancel()
            pendingCollapse = nil
            // Every fresh hover resets the AppBar's selection to whatever's
            // currently active — see the design spec's "selectedModule"
            // behavior. A manual pick from a previous expansion never
            // carries over.
            moduleManager.selectedModule = moduleManager.activeModule
            transition(to: .expanded)
            // Once open, widen the hit-test rect to the whole fixed
            // container so ordinary cursor jitter can't push the cursor
            // outside it — only actually leaving the shell's full footprint
            // counts as an exit from here on.
            hostingView.hoverRect = CGRect(origin: .zero, size: NotchMode.containerSize(for: geometry))
        }
        hostingView.onMouseExited = { [weak self] in
            guard let self else { return }
            let collapse = DispatchWorkItem { [weak self] in
                guard let self else { return }
                transition(to: .preview)
                hostingView.hoverRect = NotchMode.preview.localRect(for: geometry)
            }
            pendingCollapse = collapse
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseDelay, execute: collapse)
        }

        panel.orderFrontRegardless()
    }

    func transition(to newMode: NotchMode) {
        guard newMode != state.mode else { return }
        state.mode = newMode
    }
}
```

- [ ] **Step 4: Update `main.swift`**

In `Sources/NotchCat/main.swift`, replace this line inside `applicationDidFinishLaunching`:

```swift
        shellController = NotchShellController(geometry: geometry)
```

with:

```swift
        let moduleManager = ModuleManager()
        moduleManager.register(ClockModule())
        moduleManager.register(PlaceholderModule())

        shellController = NotchShellController(geometry: geometry, moduleManager: moduleManager)
```

The full method after this change:

```swift
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Accessory apps have no Dock icon and no app menu — appropriate for
        // a background utility that only ever shows itself over the notch.
        NSApp.setActivationPolicy(.accessory)

        guard let geometry = NotchDetector.detect() else {
            // Phase 0 scope: no notch, no fallback UI yet.
            NSLog("NotchCat: no notch detected on this Mac, exiting.")
            NSApp.terminate(nil)
            return
        }

        let moduleManager = ModuleManager()
        moduleManager.register(ClockModule())
        moduleManager.register(PlaceholderModule())

        shellController = NotchShellController(geometry: geometry, moduleManager: moduleManager)
        setupStatusItem()
    }
```

- [ ] **Step 5: Build**

Run: `cd /Users/arxing/Workspace/notchcat && swift build`
Expected: `Build complete!` with no errors.

- [ ] **Step 6: Manual verification**

Kill any previously running `NotchCat` process first (same pattern used throughout this project):

Run: `pkill -f "\.build/arm64-apple-macosx/debug/NotchCat"; sleep 1; swift run` (background it)

On the real notch, verify each of these in order:
1. At launch, before hovering: the Preview bar shows a short time readout (from `ClockModule`, since it's the only always-active module) instead of being empty.
2. Hover over the notch: it expands. The AppBar at the top shows two icons (a clock and a dashed square). Below the AppBar, the content shows a larger time readout (Clock's `expandedView()`), because `selectedModule` was reset to `activeModule` (Clock) on this hover.
3. Click the dashed-square icon: the content below the AppBar switches to "Placeholder module" text.
4. Move the mouse away and wait for it to collapse, then hover again: the AppBar's selection is back on the clock icon and the content shows the clock again — not "Placeholder module" — confirming the reset-on-reopen behavior.

- [ ] **Step 7: Commit**

```bash
cd /Users/arxing/Workspace/notchcat
git add Sources/NotchCat/UI/ModuleAppBar.swift Sources/NotchCat/UI/NotchContentView.swift Sources/NotchCat/Shell/NotchShellController.swift Sources/NotchCat/main.swift
git commit -m "Wire ModuleManager into the shell and add the Expanded AppBar"
```
