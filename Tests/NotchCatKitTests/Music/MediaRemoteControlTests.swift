import XCTest
@testable import NotchCatKit

final class MediaRemoteControlTests: XCTestCase {
    func testCommandRawValuesMatchMediaRemote() {
        XCTAssertEqual(MediaRemoteCommand.play.rawValue, 0)
        XCTAssertEqual(MediaRemoteCommand.pause.rawValue, 1)
        XCTAssertEqual(MediaRemoteCommand.togglePlayPause.rawValue, 2)
        XCTAssertEqual(MediaRemoteCommand.next.rawValue, 4)
        XCTAssertEqual(MediaRemoteCommand.previous.rawValue, 5)
    }

    func testInitFailsWhenFrameworkPathIsWrong() {
        XCTAssertNil(MediaRemoteControl(frameworkPath: "/nonexistent/MediaRemote"))
    }

    func testInitFailsWhenSymbolIsAbsent() {
        XCTAssertNil(MediaRemoteControl(symbolName: "MRMediaRemoteSendCommand_DoesNotExist"))
    }

    /// A nil here means the private framework moved or the control symbol was
    /// withdrawn; the module is expected to run without transport buttons then.
    func testConstructsAgainstRealFramework() throws {
        let control = try XCTUnwrap(MediaRemoteControl(), "MediaRemote 無法載入")
        _ = control
    }
}
