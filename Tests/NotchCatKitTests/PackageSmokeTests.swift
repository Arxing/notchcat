import XCTest
@testable import NotchCatKit

final class PackageSmokeTests: XCTestCase {
    /// Proves the test target can see the library's internal types at all.
    /// If this fails, every later task's tests are unrunnable.
    func testCanReachInternalTypes() {
        let manager = ModuleManager()
        XCTAssertNil(manager.activeModule)
        XCTAssertTrue(manager.installedModules.isEmpty)
    }
}
