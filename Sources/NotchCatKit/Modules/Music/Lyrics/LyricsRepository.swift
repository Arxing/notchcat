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
    private enum CacheEntry {
        case found(Lyrics)
        case missing(at: Date)
    }

    /// A lookup running on behalf of one or more callers of the same track.
    private struct Flight {
        let id: UUID
        let task: Task<Lyrics?, Never>
        var waiters: Int
    }

    private struct ProviderTimedOut: Error {}

    private let providers: [LyricsProvider]
    private let negativeTTL: TimeInterval
    private let providerTimeout: TimeInterval
    private let now: () -> Date
    private var cache: [String: CacheEntry] = [:]
    private var flights: [String: Flight] = [:]

    init(providers: [LyricsProvider],
         negativeTTL: TimeInterval = 86_400,
         providerTimeout: TimeInterval = 5,
         now: @escaping () -> Date = Date.init) {
        self.providers = providers
        self.negativeTTL = negativeTTL
        self.providerTimeout = providerTimeout
        self.now = now
    }

    func lyrics(for track: TrackIdentity) async -> Lyrics? {
        let key = track.cacheKey

        switch cache[key] {
        case .found(let lyrics):
            return lyrics
        case .missing(let at) where now().timeIntervalSince(at) < negativeTTL:
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

        for provider in providers {
            let result = await fetch(from: provider, track: track)
            if Task.isCancelled { break }
            guard let result else { continue }

            if result.isSynced {
                found = result
                break
            }
            // Keep the highest-priority plain result in case nothing synced
            // turns up. Not overwritten, so priority decides among plain ones.
            if fallback == nil { fallback = result }
        }

        if flights[key]?.id == flightID { flights[key] = nil }

        // An abandoned lookup says nothing about the track, so it is not cached,
        // least of all as "missing".
        if Task.isCancelled { return nil }

        if let result = found ?? fallback {
            cache[key] = .found(result)
            return result
        }
        cache[key] = .missing(at: now())
        return nil
    }

    /// Races the provider against the timeout. Timeout and thrown errors both
    /// come back as nil. The loser is cancelled, which a provider must honour
    /// for the timeout to take effect promptly (URLSession does).
    private func fetch(from provider: LyricsProvider, track: TrackIdentity) async -> Lyrics? {
        let timeout = providerTimeout
        do {
            return try await withThrowingTaskGroup(of: Lyrics?.self) { group in
                group.addTask { try await provider.fetchLyrics(for: track) }
                group.addTask {
                    try await Task.sleep(nanoseconds: UInt64(max(0, timeout) * 1_000_000_000))
                    throw ProviderTimedOut()
                }
                defer { group.cancelAll() }
                return try await group.next() ?? nil
            }
        } catch {
            return nil
        }
    }
}
