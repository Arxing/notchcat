import AppKit
import SwiftUI

/// Album art, cropped to a square from the center, or a music-note stand-in
/// when there is no artwork or its bytes are not an image. Fills the square
/// the parent offers (the smaller side).
struct CoverView: View {
    private let artwork: ArtworkRef?

    init(_ artwork: ArtworkRef?) {
        self.artwork = artwork
    }

    var body: some View {
        Color.clear
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                if let image = ArtworkImageCache.image(for: artwork) {
                    // Fill, then let the clip below trim the longer side, so a
                    // 150x83 image is cropped around its center.
                    Image(nsImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    placeholder
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .accessibilityHidden(true)
    }

    private var placeholder: some View {
        ZStack {
            Color.white.opacity(0.12)
            GeometryReader { proxy in
                Image(systemName: "music.note")
                    .font(.system(size: proxy.size.width * 0.45, weight: .regular))
                    .foregroundStyle(Color.white.opacity(0.6))
                    .frame(width: proxy.size.width, height: proxy.size.height)
            }
        }
    }
}

/// Decoded artwork, so views that redraw every second or so do not decode the
/// same bytes again. Keyed by identifier and byte count; a failed decode is
/// remembered too.
enum ArtworkImageCache {
    private final class Entry {
        let image: NSImage?
        init(_ image: NSImage?) { self.image = image }
    }

    private static let cache: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 8
        return cache
    }()

    static func image(for artwork: ArtworkRef?) -> NSImage? {
        guard let artwork, let data = artwork.data, !data.isEmpty else { return nil }
        let key = "\(artwork.identifier)#\(data.count)" as NSString
        if let entry = cache.object(forKey: key) { return entry.image }
        let decoded = NSImage(data: data).flatMap { $0.size.width > 0 && $0.size.height > 0 ? $0 : nil }
        cache.setObject(Entry(decoded), forKey: key)
        return decoded
    }
}
