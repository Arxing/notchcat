import AppKit
import Combine
import SwiftUI
import XCTest
@testable import NotchCatKit

/// Test double that drives both publishers from subjects.
private final class FakePreviewModule: NotchModule {
    let id: String
    let priority: Int
    let isActiveSubject: CurrentValueSubject<Bool, Never>
    let extraHeightSubject: CurrentValueSubject<CGFloat, Never>

    init(id: String, priority: Int = 0, isActive: Bool = true, extraHeight: CGFloat = 0) {
        self.id = id
        self.priority = priority
        self.isActiveSubject = CurrentValueSubject(isActive)
        self.extraHeightSubject = CurrentValueSubject(extraHeight)
    }

    var isActivePublisher: AnyPublisher<Bool, Never> { isActiveSubject.eraseToAnyPublisher() }
    var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never> { extraHeightSubject.eraseToAnyPublisher() }

    func tabIcon() -> AnyView { AnyView(EmptyView()) }
    func previewView() -> AnyView { AnyView(EmptyView()) }
    func expandedView() -> AnyView { AnyView(EmptyView()) }
    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}

/// A module that does not override `previewExtraHeightPublisher`.
private final class PlainModule: NotchModule {
    let id = "test.plain"
    let priority = 0
    var isActivePublisher: AnyPublisher<Bool, Never> { Just(true).eraseToAnyPublisher() }

    func tabIcon() -> AnyView { AnyView(EmptyView()) }
    func previewView() -> AnyView { AnyView(EmptyView()) }
    func expandedView() -> AnyView { AnyView(EmptyView()) }
    func onInstall() {}
    func onStart() {}
    func onStop() {}
    func onRemove() {}
    func settingsView() -> AnyView? { nil }
}

final class ModuleManagerPreviewHeightTests: XCTestCase {
    /// ModuleManager delivers module output on the main queue, so tests spin
    /// the run loop until the condition holds (or time out).
    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: TimeInterval = 1) {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() && Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
    }

    /// Lets already-queued main-queue deliveries run, for "stays unchanged" checks.
    private func settle() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    }

    func testDefaultsToZeroWithoutActiveModule() {
        let manager = ModuleManager()
        settle()
        XCTAssertEqual(manager.previewExtraHeight, 0)
    }

    func testFollowsActiveModuleValue() {
        let manager = ModuleManager()
        let module = FakePreviewModule(id: "a", extraHeight: 24)
        manager.register(module)

        waitUntil(manager.previewExtraHeight == 24)
        XCTAssertEqual(manager.previewExtraHeight, 24)

        module.extraHeightSubject.send(10)
        waitUntil(manager.previewExtraHeight == 10)
        XCTAssertEqual(manager.previewExtraHeight, 10)
    }

    func testClampsToMaximum() {
        let manager = ModuleManager()
        let module = FakePreviewModule(id: "a", extraHeight: 500)
        manager.register(module)

        waitUntil(manager.previewExtraHeight == NotchMode.maxPreviewExtraHeight)
        XCTAssertEqual(manager.previewExtraHeight, NotchMode.maxPreviewExtraHeight)

        module.extraHeightSubject.send(-5)
        waitUntil(manager.previewExtraHeight == 0)
        XCTAssertEqual(manager.previewExtraHeight, 0)
    }

    func testInactiveModuleDoesNotContribute() {
        let manager = ModuleManager()
        manager.register(FakePreviewModule(id: "a", isActive: false, extraHeight: 24))
        settle()
        XCTAssertEqual(manager.previewExtraHeight, 0)
    }

    func testSwitchingActiveModuleSwitchesSource() {
        let manager = ModuleManager()
        let low = FakePreviewModule(id: "low", priority: 1, extraHeight: 12)
        let high = FakePreviewModule(id: "high", priority: 2, extraHeight: 20)
        manager.register(low)
        waitUntil(manager.previewExtraHeight == 12)
        XCTAssertEqual(manager.previewExtraHeight, 12)

        manager.register(high)
        waitUntil(manager.previewExtraHeight == 20)
        XCTAssertEqual(manager.previewExtraHeight, 20)

        // Only the active module's value counts.
        low.extraHeightSubject.send(5)
        settle()
        XCTAssertEqual(manager.previewExtraHeight, 20)

        high.isActiveSubject.send(false)
        waitUntil(manager.previewExtraHeight == 5)
        XCTAssertEqual(manager.previewExtraHeight, 5)

        low.isActiveSubject.send(false)
        waitUntil(manager.previewExtraHeight == 0)
        XCTAssertEqual(manager.previewExtraHeight, 0)
    }

    func testNotchModuleDefaultIsZero() {
        var received: [CGFloat] = []
        let cancellable = PlainModule().previewExtraHeightPublisher.sink { received.append($0) }
        XCTAssertEqual(received, [0])
        cancellable.cancel()
    }

    func testClampedPreviewExtra() {
        XCTAssertEqual(NotchMode.maxPreviewExtraHeight, 28)
        XCTAssertEqual(NotchMode.clampedPreviewExtra(-5), 0)
        XCTAssertEqual(NotchMode.clampedPreviewExtra(0), 0)
        XCTAssertEqual(NotchMode.clampedPreviewExtra(24), 24)
        XCTAssertEqual(NotchMode.clampedPreviewExtra(28), 28)
        XCTAssertEqual(NotchMode.clampedPreviewExtra(500), 28)
    }

    // MARK: - NotchMode geometry

    private func makeGeometry() throws -> NotchGeometry {
        let screen = try XCTUnwrap(NSScreen.screens.first, "no screen available")
        return NotchGeometry(screen: screen,
                             notchSize: CGSize(width: 200, height: 32),
                             centerX: screen.frame.midX,
                             topY: screen.frame.maxY)
    }

    func testPreviewSizeGrowsByClampedExtraHeightOnly() throws {
        let geometry = try makeGeometry()
        let base = NotchMode.preview.size(for: geometry)
        XCTAssertEqual(base.height, 32)

        let grown = NotchMode.preview.size(for: geometry, previewExtraHeight: 24)
        XCTAssertEqual(grown.height, 56)
        XCTAssertEqual(grown.width, base.width)

        XCTAssertEqual(NotchMode.preview.size(for: geometry, previewExtraHeight: 500).height,
                       32 + NotchMode.maxPreviewExtraHeight)

        // Expanded ignores the extra height.
        XCTAssertEqual(NotchMode.expanded.size(for: geometry, previewExtraHeight: 24),
                       NotchMode.expanded.size(for: geometry))
    }

    func testPreviewHoverRectIncludesExtraHeightAndContainerFitsIt() throws {
        let geometry = try makeGeometry()
        let rect = NotchMode.preview.localRect(for: geometry, previewExtraHeight: 24)
        XCTAssertEqual(rect.minY, 0)
        XCTAssertEqual(rect.height, 56)
        XCTAssertEqual(rect.width, NotchMode.preview.localRect(for: geometry).width)

        let container = NotchMode.containerSize(for: geometry)
        let maxPreview = NotchMode.preview.size(for: geometry,
                                                previewExtraHeight: NotchMode.maxPreviewExtraHeight)
        XCTAssertGreaterThanOrEqual(container.height, maxPreview.height)
        XCTAssertGreaterThanOrEqual(container.height, NotchMode.expanded.size(for: geometry).height)
    }
}
