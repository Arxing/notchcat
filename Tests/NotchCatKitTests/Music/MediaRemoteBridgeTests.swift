import XCTest
@testable import NotchCatKit

final class MediaRemoteBridgeTests: XCTestCase {
    /// The bridge must construct on a supported machine. A nil here means the
    /// private framework moved or its symbols were withdrawn — the module is
    /// expected to disable itself cleanly in that case (see spec section 9).
    func testBridgeConstructs() throws {
        let bridge = try XCTUnwrap(MediaRemoteBridge(), "MediaRemote 無法載入")
        _ = bridge
    }

    func testCommandRawValuesMatchMediaRemote() {
        XCTAssertEqual(MediaRemoteCommand.play.rawValue, 0)
        XCTAssertEqual(MediaRemoteCommand.pause.rawValue, 1)
        XCTAssertEqual(MediaRemoteCommand.togglePlayPause.rawValue, 2)
        XCTAssertEqual(MediaRemoteCommand.next.rawValue, 4)
        XCTAssertEqual(MediaRemoteCommand.previous.rawValue, 5)
    }

    // MARK: - Ruling 1: reading must not depend on control

    /// If Apple withdraws only the control symbol, the read-only display (the
    /// module's core value) must survive: the bridge still constructs and
    /// `send` reports `false` instead of crashing.
    func testSendReturnsFalseWhenControlSymbolIsAbsent() throws {
        var symbols = MediaRemoteBridge.SymbolNames()
        symbols.sendCommand = "MRMediaRemoteSendCommand_DoesNotExist"
        let bridge = try XCTUnwrap(MediaRemoteBridge(symbols: symbols),
                                   "an absent optional symbol must not fail init")
        XCTAssertFalse(bridge.send(.togglePlayPause))
        XCTAssertFalse(bridge.send(.next))
    }

    /// Notification registration is optional too: observing without the
    /// register symbol must be harmless and stoppable.
    func testObservingSurvivesAbsentRegistrationSymbols() throws {
        var symbols = MediaRemoteBridge.SymbolNames()
        symbols.registerForNotifications = "MRMediaRemoteRegisterForNowPlayingNotifications_DoesNotExist"
        symbols.unregisterForNotifications = "MRMediaRemoteUnregisterForNowPlayingNotifications_DoesNotExist"
        let bridge = try XCTUnwrap(MediaRemoteBridge(symbols: symbols))
        bridge.startObserving {}
        bridge.stopObserving()
    }

    func testInitFailsWhenARequiredReadSymbolIsAbsent() {
        var symbols = MediaRemoteBridge.SymbolNames()
        symbols.getNowPlayingInfo = "MRMediaRemoteGetNowPlayingInfo_DoesNotExist"
        XCTAssertNil(MediaRemoteBridge(symbols: symbols))

        symbols = MediaRemoteBridge.SymbolNames()
        symbols.getIsPlaying = "MRMediaRemoteGetNowPlayingApplicationIsPlaying_DoesNotExist"
        XCTAssertNil(MediaRemoteBridge(symbols: symbols))
    }

    func testInitFailsWhenFrameworkPathIsWrong() {
        XCTAssertNil(MediaRemoteBridge(frameworkPath: "/nonexistent/MediaRemote"))
    }

    // MARK: - Ruling 2: change observation

    /// Posting one of the now-playing notifications must reach the callback;
    /// after `stopObserving` it must not.
    func testObserverFiresOnNotificationAndStopsAfterStop() throws {
        let bridge = try XCTUnwrap(MediaRemoteBridge())
        var count = 0
        let fired = expectation(description: "onChange fired")
        fired.assertForOverFulfill = false
        bridge.startObserving {
            count += 1
            fired.fulfill()
        }

        for name in MediaRemoteBridge.changeNotificationNames {
            NotificationCenter.default.post(name: name, object: nil)
        }
        wait(for: [fired], timeout: 2)
        XCTAssertGreaterThanOrEqual(count, 1)

        bridge.stopObserving()
        let before = count
        for name in MediaRemoteBridge.changeNotificationNames {
            NotificationCenter.default.post(name: name, object: nil)
        }
        XCTAssertEqual(count, before)
    }

    func testStartObservingTwiceDoesNotDuplicateCallbacks() throws {
        let bridge = try XCTUnwrap(MediaRemoteBridge())
        var count = 0
        bridge.startObserving { count += 1 }
        bridge.startObserving { count += 1 }
        defer { bridge.stopObserving() }

        NotificationCenter.default.post(name: MediaRemoteBridge.changeNotificationNames[0], object: nil)
        XCTAssertEqual(count, 1)
    }
}
