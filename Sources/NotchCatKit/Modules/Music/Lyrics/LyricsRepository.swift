import Foundation

/// Resolves lyrics across providers, and caches what it learns.
///
/// The ordering rule is the important part: a synced result outranks provider
/// priority. A higher-priority source may hold only plain text for a track
/// where a lower-priority one has a fully timed version, and timed lyrics are
/// the point of the module, so first-hit-wins would be the wrong rule.
///
/// This is a single pass, not two. A plain result is kept aside as we go, so
/// falling back to it costs no extra network calls. Each provider is asked at
/// most once and gets a time limit; a timeout or an error counts as a miss.
actor LyricsRepository: LyricsResolving {
    /// `until == nil` means the entry never expires.
    private enum CacheEntry {
        case found(Lyrics, until: Date?)
        case missing(until: Date)
    }

    /// A lookup running on behalf of one or more callers of the same track.
    private struct Flight {
        let id: UUID
        let task: Task<Lyrics?, Never>
        var waiters: Int
    }

    private let providers: [LyricsProvider]
    private let negativeTTL: TimeInterval
    private let failureTTL: TimeInterval
    private let providerTimeout: TimeInterval
    private let now: () -> Date
    private var cache: [String: CacheEntry] = [:]
    private var flights: [String: Flight] = [:]

    /// - Parameters:
    ///   - negativeTTL: how long "every source answered and none has it" is remembered.
    ///   - failureTTL: how long an outcome is remembered when at least one
    ///     source failed or timed out and no synced lyrics were found. That
    ///     covers both "nothing" and a plain-text fallback, so a retry soon
    ///     after can still find synced lyrics. Much shorter than `negativeTTL`
    ///     because being offline says nothing about the track.
    ///   - providerTimeout: the limit for each source; exceeding it is a miss.
    init(providers: [LyricsProvider],
         negativeTTL: TimeInterval = 86_400,
         failureTTL: TimeInterval = 120,
         providerTimeout: TimeInterval = 5,
         now: @escaping () -> Date = Date.init) {
        self.providers = providers
        self.negativeTTL = negativeTTL
        self.failureTTL = failureTTL
        self.providerTimeout = providerTimeout
        self.now = now
    }

    func lyrics(for track: TrackIdentity) async -> Lyrics? {
        let key = track.cacheKey

        switch cache[key] {
        case .found(let lyrics, let until) where until.map({ now() < $0 }) ?? true:
            return lyrics
        case .missing(let until) where now() < until:
            return nil
        default:
            break
        }
        if Task.isCancelled { return nil }

        // Callers asking for the same track share one lookup, so quick
        // back-and-forth track changes never query a provider twice.
        let flightID: UUID
        let task: Task<Lyrics?, Never>
        if var existing = flights[key] {
            existing.waiters += 1
            flights[key] = existing
            flightID = existing.id
            task = existing.task
        } else {
            flightID = UUID()
            task = Task { await self.resolve(track, key: key, flightID: flightID) }
            flights[key] = Flight(id: flightID, task: task, waiters: 1)
        }

        return await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            Task { await self.leave(key: key, flightID: flightID) }
        }
    }

    /// A caller was cancelled. The lookup is abandoned only when nobody else
    /// is still waiting for it.
    private func leave(key: String, flightID: UUID) {
        guard var flight = flights[key], flight.id == flightID else { return }
        flight.waiters -= 1
        if flight.waiters <= 0 {
            flights[key] = nil
            flight.task.cancel()
        } else {
            flights[key] = flight
        }
    }

    private func resolve(_ track: TrackIdentity, key: String, flightID: UUID) async -> Lyrics? {
        var fallback: Lyrics?
        var found: Lyrics?
        var anyFailed = false

        for provider in providers {
            let outcome = await Self.fetch(from: provider, track: track, timeout: providerTimeout)
            if Task.isCancelled { break }

            switch outcome {
            case .failed:
                anyFailed = true
            case .empty:
                break
            case .hit(let result):
                if result.isSynced {
                    found = result
                } else if fallback == nil {
                    // Keep the highest-priority plain result in case nothing
                    // synced turns up. Not overwritten, so priority decides.
                    fallback = result
                }
            }
            if found != nil { break }
        }

        if flights[key]?.id == flightID { flights[key] = nil }

        // An abandoned lookup says nothing about the track, so it is not cached,
        // least of all as "missing".
        if Task.isCancelled { return nil }

        let failureExpiry = now().addingTimeInterval(failureTTL)
        if let found {
            cache[key] = .found(found, until: nil)
            return found
        }
        if let fallback {
            // A plain result found while a source was down may be beaten by a
            // synced one later, so it is only remembered briefly.
            cache[key] = .found(fallback, until: anyFailed ? failureExpiry : nil)
            return fallback
        }
        cache[key] = .missing(until: anyFailed ? failureExpiry : now().addingTimeInterval(negativeTTL))
        return nil
    }

    private enum FetchOutcome: Sendable {
        case hit(Lyrics)
        /// The source answered and has nothing.
        case empty
        /// Error or timeout.
        case failed
    }

    /// Races the provider against the timeout without waiting for the loser:
    /// whichever finishes first resumes the caller, so a provider that ignores
    /// cancellation cannot stall the walk. Its task is cancelled and left to
    /// finish in the background.
    private static func fetch(
        from provider: LyricsProvider, track: TrackIdentity, timeout: TimeInterval
    ) async -> FetchOutcome {
        // Clamped so .infinity or a huge value cannot overflow the conversion.
        let seconds = timeout.isNaN ? 0 : min(max(0, timeout), 365 * 86_400)
        let nanoseconds = UInt64(seconds * 1_000_000_000)
        let race = FetchRace()

        return await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<FetchOutcome, Never>) in
                race.install(continuation)
                race.track(Task {
                    do {
                        let lyrics = try await provider.fetchLyrics(for: track)
                        race.finish(lyrics.map(FetchOutcome.hit) ?? .empty)
                    } catch {
                        race.finish(.failed)
                    }
                })
                race.track(Task {
                    // Throws only when cancelled, i.e. the race is already decided.
                    guard (try? await Task.sleep(nanoseconds: nanoseconds)) != nil else { return }
                    race.finish(.failed)
                })
            }
        } onCancel: {
            race.finish(.failed)
        }
    }

    /// Delivers the first outcome to the waiting caller exactly once and
    /// cancels the tasks racing for it.
    private final class FetchRace: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<FetchOutcome, Never>?
        private var outcome: FetchOutcome?
        private var tasks: [Task<Void, Never>] = []

        func install(_ continuation: CheckedContinuation<FetchOutcome, Never>) {
            lock.lock()
            if let outcome {
                lock.unlock()
                continuation.resume(returning: outcome)
            } else {
                self.continuation = continuation
                lock.unlock()
            }
        }

        func track(_ task: Task<Void, Never>) {
            lock.lock()
            let decided = outcome != nil
            if !decided { tasks.append(task) }
            lock.unlock()
            if decided { task.cancel() }
        }

        func finish(_ result: FetchOutcome) {
            lock.lock()
            guard outcome == nil else {
                lock.unlock()
                return
            }
            outcome = result
            let waiting = continuation
            continuation = nil
            let losers = tasks
            tasks = []
            lock.unlock()
            waiting?.resume(returning: result)
            losers.forEach { $0.cancel() }
        }
    }

    /// How many callers are waiting on the lookup for `track`; 0 when none is
    /// running. Exists so tests can wait for a caller to have joined.
    func inFlightWaiters(for track: TrackIdentity) -> Int {
        flights[track.cacheKey]?.waiters ?? 0
    }
}
