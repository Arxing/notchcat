import Foundation

/// Remembers artwork bytes by identifier, so a snapshot that carries only an
/// identifier can be given the bytes seen earlier for it.
///
/// Keeps at most `capacity` entries and drops the one stored longest ago.
/// Not thread-safe: use it from one queue.
final class ArtworkCache {
    private let capacity: Int
    private var entries: [String: Data] = [:]
    /// Identifiers from least to most recently stored.
    private var order: [String] = []

    init(capacity: Int = 16) {
        self.capacity = max(1, capacity)
    }

    /// With bytes: stores them under the identifier and returns `ref` unchanged.
    /// Without bytes: returns `ref` with the bytes stored for that identifier,
    /// or unchanged when there are none. `nil` passes through.
    func resolve(_ ref: ArtworkRef?) -> ArtworkRef? {
        guard let ref else { return nil }
        if let data = ref.data {
            store(data, for: ref.identifier)
            return ref
        }
        guard let cached = entries[ref.identifier] else { return ref }
        return ArtworkRef(identifier: ref.identifier, data: cached)
    }

    private func store(_ data: Data, for identifier: String) {
        if entries.updateValue(data, forKey: identifier) != nil {
            order.removeAll { $0 == identifier }
        }
        order.append(identifier)
        while order.count > capacity {
            entries.removeValue(forKey: order.removeFirst())
        }
    }
}
