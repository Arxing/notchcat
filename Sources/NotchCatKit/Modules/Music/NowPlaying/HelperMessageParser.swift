import Foundation

/// One decoded line from the MediaRemote helper.
enum HelperMessage: Equatable {
    /// Nothing is playing.
    case none
    case track(NowPlayingSnapshot)
}

/// Parses the helper's wire format: one JSON object per line.
///
///     {"none":true}
///     {"title":String,"artist":String?,"album":String?,"duration":Number?,
///      "elapsed":Number,"timestamp":Number,"rate":Number,
///      "artworkId":String?,"artworkData":String?}
///
/// `timestamp` is Unix epoch seconds; `artworkData` is base64 image bytes and is
/// absent when the helper did not attach the image to this message.
enum HelperMessageParser {
    static let sourceID = "mediaremote"

    /// Returns `nil` for anything that is not a well-formed message, including a
    /// track without a title or with a non-finite `elapsed`/`timestamp`/`rate`/`duration`.
    static func parse(line: String) -> HelperMessage? {
        guard let data = line.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }

        if object["none"] as? Bool == true { return HelperMessage.none }

        guard let title = object["title"] as? String, !title.isEmpty else { return nil }

        var numbers: [String: Double] = [:]
        for key in ["duration", "elapsed", "timestamp", "rate"] {
            guard let raw = object[key] else { continue }
            guard let number = raw as? NSNumber, number.doubleValue.isFinite else { return nil }
            numbers[key] = number.doubleValue
        }

        let rate = numbers["rate"]
        var position: PositionAnchor?
        if let elapsed = numbers["elapsed"], let timestamp = numbers["timestamp"], let rate {
            position = PositionAnchor(elapsed: elapsed,
                                      timestamp: Date(timeIntervalSince1970: timestamp),
                                      rate: rate)
        }

        let duration = numbers["duration"].flatMap { $0 > 0 ? $0 : nil }

        var artwork: ArtworkRef?
        if let identifier = nonEmpty(object["artworkId"]) {
            let bytes = (object["artworkData"] as? String).flatMap { Data(base64Encoded: $0) }
            artwork = ArtworkRef(identifier: identifier, data: bytes)
        }

        return .track(NowPlayingSnapshot(
            title: title,
            artist: nonEmpty(object["artist"]),
            album: nonEmpty(object["album"]),
            duration: duration,
            isPlaying: (rate ?? 0) > 0,
            position: position,
            artwork: artwork,
            sourceID: sourceID))
    }

    private static func nonEmpty(_ value: Any?) -> String? {
        guard let string = value as? String, !string.isEmpty else { return nil }
        return string
    }
}

/// Accumulates bytes read from the helper's pipe and yields only complete lines.
///
/// Lines are split on `\n` at the byte level, so a multi-byte character split
/// across reads is reassembled. A line that is not valid UTF-8 is dropped without
/// affecting the lines around it. Empty lines are skipped.
struct HelperLineBuffer {
    private var pending = Data()

    mutating func append(_ data: Data) -> [String] {
        pending.append(data)
        var lines: [String] = []
        while let newline = pending.firstIndex(of: 0x0A) {
            let lineBytes = pending[pending.startIndex..<newline]
            pending.removeSubrange(pending.startIndex...newline)
            guard !lineBytes.isEmpty, let line = String(data: lineBytes, encoding: .utf8) else { continue }
            lines.append(line)
        }
        return lines
    }
}
