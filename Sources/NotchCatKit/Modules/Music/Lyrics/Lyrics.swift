import Foundation

struct LyricLine: Equatable {
    let time: TimeInterval
    let text: String
}

enum Lyrics: Equatable {
    case synced([LyricLine])
    case plain(String)

    var isSynced: Bool {
        if case .synced = self { return true }
        return false
    }

    /// Index of the line that should be showing at `position`, or nil when the
    /// lyrics are plain or the track has not reached the first line yet.
    /// When several lines share a timestamp, the last of them is current.
    func lineIndex(at position: TimeInterval) -> Int? {
        guard case .synced(let lines) = self, let first = lines.first else { return nil }
        guard position >= first.time else { return nil }  // also rejects NaN

        // Upper bound: first line strictly after `position`; the one before it is current.
        var low = 1
        var high = lines.count
        while low < high {
            let mid = (low + high) / 2
            if lines[mid].time <= position {
                low = mid + 1
            } else {
                high = mid
            }
        }
        return low - 1
    }
}

/// Parses LRC text from whichever provider supplied it.
///
/// Community lyric databases vary wildly in quality, so this is deliberately
/// forgiving: it sorts what it finds, drops empty lines, and refuses to call
/// something synced unless it actually carries timestamps. It scans by hand
/// rather than with a regular expression so that hostile input (huge numbers,
/// megabyte lines, thousands of tags) costs linear time and cannot trap.
enum LRCParser {
    /// Returns nil for empty input, and for input whose only timestamps sit on
    /// lines with no lyric text. Text with no valid time tag at all, including
    /// metadata-only tags such as `[ar:…]`, comes back as `.plain`.
    static func parse(_ raw: String) -> Lyrics? {
        let withoutBOM = raw.drop(while: { $0 == "\u{FEFF}" })
        let trimmed = withoutBOM.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        var lines: [LyricLine] = []
        var sawTimeTag = false
        for rawLine in trimmed.split(whereSeparator: \.isNewline) {
            var rest = rawLine.drop(while: \.isWhitespace)
            var times: [TimeInterval] = []
            while let (time, remainder) = leadingTimeTag(in: rest) {
                times.append(time)
                rest = remainder
            }
            guard !times.isEmpty else { continue }
            sawTimeTag = true

            let text = rest.trimmingCharacters(in: .whitespaces)
            guard !text.isEmpty else { continue }
            for time in times {
                lines.append(LyricLine(time: time, text: text))
            }
        }

        guard sawTimeTag else { return .plain(trimmed) }
        guard !lines.isEmpty else { return nil }

        // Sort on (time, original position) so equal timestamps keep input order.
        let ordered = lines.enumerated().sorted {
            $0.element.time != $1.element.time
                ? $0.element.time < $1.element.time
                : $0.offset < $1.offset
        }
        return .synced(ordered.map(\.element))
    }

    /// Reads one `[mm:ss]`, `[mm:ss.x…]` or `[mm:ss:x…]` tag at the start of
    /// `text`. Minutes take 1–3 digits, seconds 1–2, the fraction 1–3; anything
    /// else (signs, longer numbers, metadata keys) is not a time tag.
    private static func leadingTimeTag(in text: Substring) -> (TimeInterval, Substring)? {
        // Tags are pure ASCII, so walking UTF-8 indices never splits a character.
        let bytes = text.utf8
        var index = bytes.startIndex

        func consume(_ byte: UInt8) -> Bool {
            guard index < bytes.endIndex, bytes[index] == byte else { return false }
            index = bytes.index(after: index)
            return true
        }
        func digits(maxCount: Int) -> (value: Int, count: Int)? {
            var value = 0
            var count = 0
            while count < maxCount, index < bytes.endIndex,
                  (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(bytes[index]) {
                value = value * 10 + Int(bytes[index] - UInt8(ascii: "0"))
                count += 1
                index = bytes.index(after: index)
            }
            return count > 0 ? (value, count) : nil
        }

        guard consume(UInt8(ascii: "[")),
              let minutes = digits(maxCount: 3),
              consume(UInt8(ascii: ":")),
              let seconds = digits(maxCount: 2) else { return nil }

        var fraction = 0.0
        if consume(UInt8(ascii: ".")) || consume(UInt8(ascii: ":")) {
            guard let parts = digits(maxCount: 3) else { return nil }
            fraction = Double(parts.value) / pow(10, Double(parts.count))
        }
        guard consume(UInt8(ascii: "]")) else { return nil }

        let time = Double(minutes.value) * 60 + Double(seconds.value) + fraction
        return (time, text[index...])
    }
}
