import XCTest
@testable import NotchCatKit

private final class FakeProvider: LyricsProvider, @unchecked Sendable {
    let id: String
    private let result: Lyrics?
    private let error: Error?
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

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        recordCall()
        if delay > 0 {
            try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
        if let error { throw error }
        return result
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
    private let synced = Lyrics.synced([LyricLine(time: 0, text: "line")])

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
        let provider = FakeProvider(id: "p", result: synced, delay: 0.1)
        let repo = LyricsRepository(providers: [provider])

        let survivor = Task { await repo.lyrics(for: track) }
        while provider.callCount == 0 { await Task.yield() }
        let cancelled = Task { await repo.lyrics(for: track) }
        cancelled.cancel()
        _ = await cancelled.value

        let result = await survivor.value

        XCTAssertEqual(result, synced)
        XCTAssertEqual(provider.callCount, 1)
    }
}
