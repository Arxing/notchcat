import Combine
import XCTest
@testable import NotchCatKit

private final class FakeHandle: HelperProcessHandle {
    private(set) var terminateCount = 0
    func terminate() { terminateCount += 1 }
}

private struct FakeLaunchError: Error {}

private final class FakeLauncher: HelperProcessLaunching {
    var shouldThrow = false
    private(set) var launchCount = 0
    private(set) var handles: [FakeHandle] = []
    private var onLine: ((String) -> Void)?
    private var onExit: ((Int32) -> Void)?

    func launch(onLine: @escaping (String) -> Void, onExit: @escaping (Int32) -> Void) throws -> HelperProcessHandle {
        launchCount += 1
        if shouldThrow { throw FakeLaunchError() }
        self.onLine = onLine
        self.onExit = onExit
        let handle = FakeHandle()
        handles.append(handle)
        return handle
    }

    func emit(_ line: String) { onLine?(line) }
    func exit(_ status: Int32 = 1) { onExit?(status) }
}

private final class FakeScheduler {
    private(set) var delays: [TimeInterval] = []
    private var pending: [() -> Void] = []

    var schedule: (TimeInterval, @escaping () -> Void) -> Void {
        { [unowned self] delay, work in
            self.delays.append(delay)
            self.pending.append(work)
        }
    }

    func runPending() {
        let work = pending
        pending = []
        work.forEach { $0() }
    }
}

final class MediaRemoteSourceTests: XCTestCase {
    private let trackLine = #"{"title":"聲聲慢","artist":"鄧福如AFÜ","duration":233.9,"elapsed":12.5,"timestamp":1793777309.5,"rate":1}"#

    private var launcher: FakeLauncher!
    private var scheduler: FakeScheduler!
    private var published: [NowPlayingSnapshot?] = []
    private var cancellables: Set<AnyCancellable> = []

    override func setUp() {
        super.setUp()
        launcher = FakeLauncher()
        scheduler = FakeScheduler()
        published = []
        cancellables = []
    }

    private func makeSource(retryDelays: [TimeInterval] = [1, 2, 4, 8, 16, 30]) -> MediaRemoteSource {
        let source = MediaRemoteSource(launcher: launcher, retryDelays: retryDelays, schedule: scheduler.schedule)
        source.snapshots.dropFirst().sink { [unowned self] in self.published.append($0) }.store(in: &cancellables)
        return source
    }

    func testPublishesTrackFromLine() throws {
        let source = makeSource()
        source.start()
        launcher.emit(trackLine)

        XCTAssertEqual(published.count, 1)
        let snapshot = try XCTUnwrap(published.first ?? nil)
        guard case .track(let expected)? = HelperMessageParser.parse(line: trackLine) else {
            return XCTFail("fixture must parse")
        }
        XCTAssertEqual(snapshot, expected)
        XCTAssertEqual(snapshot.title, "聲聲慢")
        XCTAssertEqual(source.id, "mediaremote")
    }

    func testNoneLinePublishesNil() {
        let source = makeSource()
        source.start()
        launcher.emit(trackLine)
        launcher.emit(#"{"none":true}"#)

        XCTAssertEqual(published.count, 2)
        XCTAssertNotNil(published[0])
        XCTAssertNil(published[1])
    }

    func testGarbageLineIsIgnored() {
        let source = makeSource()
        source.start()
        launcher.emit("keys=14")
        launcher.emit(#"{"title":""}"#)

        XCTAssertTrue(published.isEmpty)
    }

    func testRestartsAfterUnexpectedExitWithBackoff() {
        let source = makeSource()
        source.start()
        for _ in 0..<3 {
            launcher.exit()
            scheduler.runPending()
        }

        XCTAssertEqual(scheduler.delays, [1, 2, 4])
        XCTAssertEqual(launcher.launchCount, 4)
    }

    func testSuccessfulLineResetsBackoff() {
        let source = makeSource()
        source.start()
        launcher.exit()
        scheduler.runPending()
        launcher.emit(trackLine)
        launcher.exit()

        XCTAssertEqual(scheduler.delays, [1, 1])
    }

    func testGivesUpAfterExhaustingDelays() {
        let source = makeSource(retryDelays: [1, 2])
        source.start()
        launcher.emit(trackLine)
        launcher.exit()
        scheduler.runPending()
        launcher.exit()
        scheduler.runPending()
        launcher.exit()
        scheduler.runPending()

        XCTAssertEqual(scheduler.delays, [1, 2])
        XCTAssertEqual(launcher.launchCount, 3)
        XCTAssertEqual(published.count, 2)
        XCTAssertNil(published.last ?? nil)
    }

    func testLaunchThrowingCountsAsFailure() {
        launcher.shouldThrow = true
        let source = makeSource(retryDelays: [1, 2, 4])
        source.start()
        for _ in 0..<5 { scheduler.runPending() }

        XCTAssertEqual(scheduler.delays, [1, 2, 4])
        XCTAssertEqual(launcher.launchCount, 4)
        XCTAssertEqual(published.count, 1)
        XCTAssertNil(published.first ?? nil)
    }

    func testStopTerminatesAndDoesNotRestart() {
        let source = makeSource()
        source.start()
        source.stop()
        launcher.exit(15)
        scheduler.runPending()

        XCTAssertEqual(launcher.handles.first?.terminateCount, 1)
        XCTAssertTrue(scheduler.delays.isEmpty)
        XCTAssertEqual(launcher.launchCount, 1)
    }

    func testStopCancelsPendingRestart() {
        let source = makeSource()
        source.start()
        launcher.exit()
        source.stop()
        scheduler.runPending()

        XCTAssertEqual(launcher.launchCount, 1)
    }

    // MARK: - PerlHelperLauncher

    private func makeTemporaryFile() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("notchcat-\(UUID().uuidString).dylib")
        try Data().write(to: url)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    func testPerlLauncherThrowsWhenPerlMissing() throws {
        let launcher = PerlHelperLauncher(perlPath: "/nonexistent/notchcat/perl", dylibURL: try makeTemporaryFile())
        XCTAssertThrowsError(try launcher.launch(onLine: { _ in }, onExit: { _ in })) { error in
            XCTAssertEqual(error as? PerlHelperLauncher.LaunchError, .perlNotFound)
        }
    }

    func testPerlLauncherThrowsWhenDylibMissing() {
        let missing = PerlHelperLauncher(dylibURL: URL(fileURLWithPath: "/nonexistent/notchcat/libNotchCatMediaRemoteHelper.dylib"))
        XCTAssertThrowsError(try missing.launch(onLine: { _ in }, onExit: { _ in })) { error in
            XCTAssertEqual(error as? PerlHelperLauncher.LaunchError, .dylibNotFound)
        }
        let unset = PerlHelperLauncher(dylibURL: nil)
        XCTAssertThrowsError(try unset.launch(onLine: { _ in }, onExit: { _ in })) { error in
            XCTAssertEqual(error as? PerlHelperLauncher.LaunchError, .dylibNotFound)
        }
    }

    // MARK: - CappedHelperLineBuffer

    func testCappedBufferPassesLinesWithinCap() {
        var buffer = CappedHelperLineBuffer(maxLineBytes: 8)
        XCTAssertEqual(buffer.append(Data("abc".utf8)), [])
        XCTAssertEqual(buffer.append(Data("de\n12345678\nx".utf8)), ["abcde", "12345678"])
        XCTAssertEqual(buffer.append(Data("y\n".utf8)), ["xy"])
    }

    func testCappedBufferDiscardsOversizedUnterminatedLineUntilNewline() {
        var buffer = CappedHelperLineBuffer(maxLineBytes: 8)
        XCTAssertEqual(buffer.append(Data("12345".utf8)), [])
        XCTAssertEqual(buffer.append(Data("6789".utf8)), [])
        XCTAssertEqual(buffer.append(Data("still dropped".utf8)), [])
        XCTAssertEqual(buffer.append(Data("tail\nok\n".utf8)), ["ok"])
    }

    func testCappedBufferDiscardsOversizedLineWithinOneChunk() {
        var buffer = CappedHelperLineBuffer(maxLineBytes: 4)
        XCTAssertEqual(buffer.append(Data("a\ntoolong\nb\n".utf8)), ["a", "b"])
    }

    func testCappedBufferDefaultCapIsFourMiB() {
        XCTAssertEqual(CappedHelperLineBuffer.defaultMaxLineBytes, 4 * 1024 * 1024)
    }
}
