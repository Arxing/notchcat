import XCTest
@testable import NotchCatKit

private final class FakeProvider: LyricsProvider, @unchecked Sendable {
    let id: String
    private var result: Lyrics?
    private var error: Error?
    private let delay: TimeInterval
    private let lock = NSLock()
    private var calls = 0

    var callCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return calls
    }

    private func recordCall() {
        lock.lock()
        calls += 1
        lock.unlock()
    }

    init(id: String, result: Lyrics? = nil, error: Error? = nil, delay: TimeInterval = 0) {
        self.id = id
        self.result = result
        self.error = error
        self.delay = delay
    }

    /// Changes what later calls answer, e.g. a source that recovers.
    func respond(result: Lyrics? = nil, error: Error? = nil) {
        lock.lock()
        self.result = result
        self.error = error
        lock.unlock()
    }

    private func currentResponse() -> (Lyrics?, Error?) {
        lock.lock()
        defer { lock.unlock() }
        return (result, error)
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        recordCall()
        if delay > 0 {
            try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
        let (result, error) = currentResponse()
        if let error { throw error }
        return result
    }
}

/// Ignores cancellation: waits out `delay` on a dispatch timer, then answers.
private final class StubbornProvider: LyricsProvider, @unchecked Sendable {
    let id = "stubborn"
    private let result: Lyrics?
    private let delay: TimeInterval
    private let lock = NSLock()
    private var calls = 0

    var callCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return calls
    }

    init(result: Lyrics?, delay: TimeInterval) {
        self.result = result
        self.delay = delay
    }

    private func recordCall() {
        lock.lock()
        calls += 1
        lock.unlock()
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        recordCall()
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            DispatchQueue.global().asyncAfter(deadline: .now() + delay) { continuation.resume() }
        }
        return result
    }
}

/// Answers only after `open()`, and ignores cancellation until then.
private final class GatedProvider: LyricsProvider, @unchecked Sendable {
    let id = "gated"
    private let result: Lyrics?
    private let lock = NSLock()
    private var calls = 0
    private var isOpen = false
    private var waiting: [CheckedContinuation<Void, Never>] = []

    var callCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return calls
    }

    init(result: Lyrics?) { self.result = result }

    private func enter(_ continuation: CheckedContinuation<Void, Never>) {
        lock.lock()
        calls += 1
        if isOpen {
            lock.unlock()
            continuation.resume()
        } else {
            waiting.append(continuation)
            lock.unlock()
        }
    }

    func open() {
        lock.lock()
        isOpen = true
        let toResume = waiting
        waiting = []
        lock.unlock()
        toResume.forEach { $0.resume() }
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        await withCheckedContinuation { enter($0) }
        return result
    }
}

/// Spins (yielding) until `condition` holds, failing the test after a deadline
/// instead of hanging.
private func waitUntil(_ description: String, file: StaticString = #filePath, line: UInt = #line,
                       _ condition: () async -> Bool) async {
    let deadline = Date().addingTimeInterval(5)
    while await !condition() {
        if Date() > deadline {
            XCTFail("Timed out waiting for \(description)", file: file, line: line)
            return
        }
        await Task.yield()
    }
}

private struct ProviderFailure: Error {}

/// Mutable clock the repository can read from any isolation.
private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var current = Date(timeIntervalSince1970: 0)

    var now: Date {
        lock.lock()
        defer { lock.unlock() }
        return current
    }

    func advance(by interval: TimeInterval) {
        lock.lock()
        current = current.addingTimeInterval(interval)
        lock.unlock()
    }
}

final class LyricsRepositoryTests: XCTestCase {
    private let track = TrackIdentity(title: "T", artist: "A", album: nil, duration: 200)
    private let synced = Lyrics.synced([LyricLine(time: 0, text: "line"), LyricLine(time: 10, text: "next")])
    /// A single timed line is the usual "instrumental" placeholder, which the
    /// plausibility gate rejects.
    private let implausible = Lyrics.synced([LyricLine(time: 0, text: "純音樂，請欣賞")])

    /// The rule the whole lyrics design rests on: a synced result outranks
    /// provider priority, so a plain hit from an earlier source must not stop
    /// the walk.
    func testSyncedResultBeatsHigherPriorityPlain() async {
        let first = FakeProvider(id: "first", result: .plain("words"))
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
    }

    func testEachProviderIsQueriedAtMostOnce() async {
        let first = FakeProvider(id: "first", result: .plain("words"))
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        _ = await repo.lyrics(for: track)

        XCTAssertEqual(first.callCount, 1)
        XCTAssertEqual(second.callCount, 1)
    }

    func testStopsAtFirstSyncedResult() async {
        let first = FakeProvider(id: "first", result: synced)
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        _ = await repo.lyrics(for: track)

        XCTAssertEqual(first.callCount, 1)
        XCTAssertEqual(second.callCount, 0, "No source after the first synced hit should be asked")
    }

    func testFallsBackToPlainKeepingEarlierProvider() async {
        let first = FakeProvider(id: "first", result: .plain("first words"))
        let second = FakeProvider(id: "second", result: .plain("second words"))
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, .plain("first words"))
        XCTAssertEqual(second.callCount, 1, "A plain hit must not stop the walk")
    }

    func testFailingProviderDoesNotBlockLaterOnes() async {
        let first = FakeProvider(id: "first", error: ProviderFailure())
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
        XCTAssertEqual(first.callCount, 1)
    }

    func testReturnsNilWhenNobodyHasAnything() async {
        let repo = LyricsRepository(providers: [FakeProvider(id: "only", result: nil)])
        let result = await repo.lyrics(for: track)
        XCTAssertNil(result)
    }

    func testPositiveResultIsCached() async {
        let provider = FakeProvider(id: "p", result: synced)
        let repo = LyricsRepository(providers: [provider])

        let first = await repo.lyrics(for: track)
        let second = await repo.lyrics(for: track)

        XCTAssertEqual(first, synced)
        XCTAssertEqual(second, synced)
        XCTAssertEqual(provider.callCount, 1)
    }

    func testPlainFallbackIsCached() async {
        let provider = FakeProvider(id: "p", result: .plain("words"))
        let repo = LyricsRepository(providers: [provider])

        _ = await repo.lyrics(for: track)
        let second = await repo.lyrics(for: track)

        XCTAssertEqual(second, .plain("words"))
        XCTAssertEqual(provider.callCount, 1)
    }

    func testNegativeResultIsCachedWithinTTL() async {
        let provider = FakeProvider(id: "p", result: nil)
        let repo = LyricsRepository(providers: [provider], negativeTTL: 86_400)

        _ = await repo.lyrics(for: track)
        let second = await repo.lyrics(for: track)

        XCTAssertNil(second)
        XCTAssertEqual(provider.callCount, 1, "A miss is cached so switching back does not re-query every source")
    }

    func testNegativeCacheExpires() async {
        let provider = FakeProvider(id: "p", result: nil)
        let clock = TestClock()
        let repo = LyricsRepository(providers: [provider], negativeTTL: 60, now: { clock.now })

        _ = await repo.lyrics(for: track)
        clock.advance(by: 59)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 1, "Still inside the TTL")

        clock.advance(by: 2)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 2, "Past the TTL the sources are asked again")
    }

    /// Cache keys come from the normalized identity, so the same track with a
    /// feat. suffix and a slightly different duration must not re-query.
    func testCacheIsKeyedOnNormalizedIdentity() async {
        let provider = FakeProvider(id: "p", result: synced)
        let repo = LyricsRepository(providers: [provider])

        _ = await repo.lyrics(for: track)
        let result = await repo.lyrics(for: TrackIdentity(title: "T (feat. X)", artist: "A", album: nil, duration: 203))

        XCTAssertEqual(result, synced)
        XCTAssertEqual(provider.callCount, 1)
    }

    func testProviderTimeoutCountsAsMiss() async {
        let slow = FakeProvider(id: "slow", result: synced, delay: 5)
        let second = FakeProvider(id: "second", result: .plain("words"))
        let repo = LyricsRepository(providers: [slow, second], providerTimeout: 0.05)

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, .plain("words"))
        XCTAssertEqual(slow.callCount, 1)
        XCTAssertEqual(second.callCount, 1, "A timed-out source must not block later ones")
    }

    func testFastProviderIsNotCutOffByTimeout() async {
        let provider = FakeProvider(id: "p", result: synced, delay: 0.01)
        let repo = LyricsRepository(providers: [provider], providerTimeout: 2)

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
    }

    func testConcurrentRequestsForSameTrackQueryProvidersOnce() async {
        let provider = FakeProvider(id: "p", result: synced, delay: 0.1)
        let repo = LyricsRepository(providers: [provider])

        async let a = repo.lyrics(for: track)
        async let b = repo.lyrics(for: TrackIdentity(title: "T (feat. X)", artist: "A", album: nil, duration: 201))
        let (first, second) = await (a, b)

        XCTAssertEqual(first, synced)
        XCTAssertEqual(second, synced)
        XCTAssertEqual(provider.callCount, 1)
    }

    /// Cancelling the caller abandons the lookup; the abandoned lookup must not
    /// be remembered as "nobody has lyrics for this track".
    func testCancelledLookupIsNotCachedAsMissing() async throws {
        let provider = FakeProvider(id: "p", result: synced, delay: 0.2)
        let repo = LyricsRepository(providers: [provider])

        let cancelled = Task { await repo.lyrics(for: track) }
        while provider.callCount == 0 { await Task.yield() }
        cancelled.cancel()
        let abandoned = await cancelled.value
        XCTAssertNil(abandoned)

        let retry = await repo.lyrics(for: track)

        XCTAssertEqual(retry, synced)
        XCTAssertEqual(provider.callCount, 2, "The retry must query again rather than hit a cached miss")
    }

    func testCancelledCallerDoesNotCancelLookupSharedWithAnotherCaller() async {
        let provider = GatedProvider(result: synced)
        let repo = LyricsRepository(providers: [provider])

        let survivor = Task { await repo.lyrics(for: track) }
        await waitUntil("the first caller to reach the provider") { provider.callCount == 1 }
        let joiner = Task { await repo.lyrics(for: track) }
        await waitUntil("the second caller to join the lookup") { await repo.inFlightWaiters(for: track) == 2 }

        joiner.cancel()
        await waitUntil("the cancelled caller to leave") { await repo.inFlightWaiters(for: track) == 1 }
        provider.open()

        let result = await survivor.value
        _ = await joiner.value

        XCTAssertEqual(result, synced)
        XCTAssertEqual(provider.callCount, 1)
    }

    // MARK: - Timeout that the provider ignores

    func testProviderIgnoringCancellationDoesNotStallTheWalk() async {
        let stubborn = StubbornProvider(result: synced, delay: 1)
        let second = FakeProvider(id: "second", result: .plain("words"))
        let repo = LyricsRepository(providers: [stubborn, second], providerTimeout: 0.05)

        let started = Date()
        let result = await repo.lyrics(for: track)
        let elapsed = Date().timeIntervalSince(started)

        XCTAssertEqual(result, .plain("words"))
        XCTAssertEqual(stubborn.callCount, 1)
        XCTAssertEqual(second.callCount, 1)
        XCTAssertLessThan(elapsed, 0.5, "The walk must move on at the timeout, not when the provider finishes")
    }

    func testInfiniteTimeoutDoesNotTrap() async {
        let provider = FakeProvider(id: "p", result: synced, delay: 0.01)
        let repo = LyricsRepository(providers: [provider], providerTimeout: .infinity)

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
    }

    // MARK: - Failure vs. not found

    private func makeRepo(
        _ providers: [LyricsProvider], clock: TestClock, timeout: TimeInterval = 5
    ) -> LyricsRepository {
        LyricsRepository(providers: providers, negativeTTL: 3_600, failureTTL: 60,
                         providerTimeout: timeout, now: { clock.now })
    }

    func testCleanMissIsRememberedForNegativeTTLNotFailureTTL() async {
        let provider = FakeProvider(id: "p", result: nil)
        let clock = TestClock()
        let repo = makeRepo([provider], clock: clock)

        _ = await repo.lyrics(for: track)
        clock.advance(by: 3_599)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 1)

        clock.advance(by: 2)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 2)
    }

    func testMissWithAFailingProviderIsRememberedOnlyForFailureTTL() async {
        let provider = FakeProvider(id: "p", error: ProviderFailure())
        let clock = TestClock()
        let repo = makeRepo([provider], clock: clock)

        let first = await repo.lyrics(for: track)
        XCTAssertNil(first)
        clock.advance(by: 59)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 1, "Inside failureTTL the failure is still remembered")

        clock.advance(by: 2)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 2, "Past failureTTL the source is asked again, long before negativeTTL")
    }

    func testMixedFailedAndCleanMissCountsAsFailed() async {
        let failing = FakeProvider(id: "failing", error: ProviderFailure())
        let empty = FakeProvider(id: "empty", result: nil)
        let clock = TestClock()
        let repo = makeRepo([failing, empty], clock: clock)

        _ = await repo.lyrics(for: track)
        clock.advance(by: 61)
        _ = await repo.lyrics(for: track)

        XCTAssertEqual(failing.callCount, 2)
        XCTAssertEqual(empty.callCount, 2)
    }

    func testTimeoutCountsAsFailureForCaching() async {
        let slow = FakeProvider(id: "slow", result: synced, delay: 5)
        let clock = TestClock()
        let repo = makeRepo([slow], clock: clock, timeout: 0.05)

        let first = await repo.lyrics(for: track)
        XCTAssertNil(first)
        clock.advance(by: 61)
        _ = await repo.lyrics(for: track)

        XCTAssertEqual(slow.callCount, 2)
    }

    func testPlainFallbackFoundWhileAProviderFailedIsRetriedAfterFailureTTL() async {
        let flaky = FakeProvider(id: "flaky", error: ProviderFailure())
        let plain = FakeProvider(id: "plain", result: .plain("words"))
        let clock = TestClock()
        let repo = makeRepo([flaky, plain], clock: clock)

        let first = await repo.lyrics(for: track)
        XCTAssertEqual(first, .plain("words"))
        clock.advance(by: 59)
        let cached = await repo.lyrics(for: track)
        XCTAssertEqual(cached, .plain("words"))
        XCTAssertEqual(flaky.callCount, 1, "Inside failureTTL the plain fallback is served from cache")

        flaky.respond(result: synced)
        clock.advance(by: 2)
        let retried = await repo.lyrics(for: track)

        XCTAssertEqual(retried, synced, "The retry finds the synced lyrics the failed source holds")
        XCTAssertEqual(flaky.callCount, 2)
    }

    func testPlainFallbackWithAllProvidersCleanStaysCached() async {
        let provider = FakeProvider(id: "p", result: .plain("words"))
        let clock = TestClock()
        let repo = makeRepo([provider], clock: clock)

        _ = await repo.lyrics(for: track)
        clock.advance(by: 1_000_000_000)
        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, .plain("words"))
        XCTAssertEqual(provider.callCount, 1)
    }

    func testSyncedResultStaysCachedEvenIfAnEarlierProviderFailed() async {
        let flaky = FakeProvider(id: "flaky", error: ProviderFailure())
        let good = FakeProvider(id: "good", result: synced)
        let clock = TestClock()
        let repo = makeRepo([flaky, good], clock: clock)

        _ = await repo.lyrics(for: track)
        clock.advance(by: 1_000_000_000)
        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
        XCTAssertEqual(flaky.callCount, 1)
        XCTAssertEqual(good.callCount, 1)
    }

    // MARK: - Plausibility gate

    func testImplausibleSyncedResultIsSkippedAndWalkContinues() async {
        let first = FakeProvider(id: "first", result: implausible)
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, synced)
        XCTAssertEqual(first.callCount, 1)
        XCTAssertEqual(second.callCount, 1)
    }

    func testImplausibleSyncedDoesNotBecomeFallback() async {
        let only = FakeProvider(id: "only", result: implausible)
        let repo = LyricsRepository(providers: [only])

        let result = await repo.lyrics(for: track)

        XCTAssertNil(result)
    }

    func testImplausibleSyncedThenLaterPlainReturnsPlain() async {
        let first = FakeProvider(id: "first", result: implausible)
        let second = FakeProvider(id: "second", result: .plain("words"))
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, .plain("words"))
    }

    func testImplausibleSyncedCountsAsCleanAnswerForCaching() async {
        let provider = FakeProvider(id: "p", result: implausible)
        let clock = TestClock()
        let repo = makeRepo([provider], clock: clock)

        let first = await repo.lyrics(for: track)
        XCTAssertNil(first)
        clock.advance(by: 3_599)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 1, "Remembered for negativeTTL (3600), not failureTTL (60)")

        clock.advance(by: 2)
        _ = await repo.lyrics(for: track)
        XCTAssertEqual(provider.callCount, 2)
    }

    func testPlausibilityUsesTrackDuration() async {
        // Last line at 150 s: fits a 200 s track, but not a 100 s one.
        let lyrics = Lyrics.synced([LyricLine(time: 5, text: "a"), LyricLine(time: 150, text: "b")])

        let longEnough = await LyricsRepository(providers: [FakeProvider(id: "p", result: lyrics)])
            .lyrics(for: TrackIdentity(title: "T", artist: "A", album: nil, duration: 200))
        let tooShort = await LyricsRepository(providers: [FakeProvider(id: "p", result: lyrics)])
            .lyrics(for: TrackIdentity(title: "T", artist: "A", album: nil, duration: 100))

        XCTAssertEqual(longEnough, lyrics)
        XCTAssertNil(tooShort)
    }
}
