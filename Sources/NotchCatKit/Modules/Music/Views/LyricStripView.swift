import SwiftUI

/// The one-line strip below the notch in Preview: the current lyric line, or
/// the track title when there is no line to show. Draws its own translucent
/// black background, because the shell leaves this band unfilled.
struct LyricStripView: View {
    @ObservedObject var viewModel: MusicViewModel

    /// Matches the shell's Preview corner radius.
    static let bottomCornerRadius: CGFloat = 10
    private static let horizontalPadding: CGFloat = 12

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.25)) { context in
            let content = MusicPresentation.stripContent(
                snapshot: viewModel.snapshot, lyrics: viewModel.lyrics, at: context.date)
            MarqueeText(text(for: content), font: .system(size: 12, weight: .medium))
                .foregroundStyle(Color.white.opacity(isLine(content) ? 0.95 : 0.7))
                .padding(.horizontal, Self.horizontalPadding)
                .frame(maxWidth: .infinity)
                .frame(height: MusicModule.lyricStripHeight)
                .background(
                    UnevenRoundedRectangle(topLeadingRadius: 0,
                                           bottomLeadingRadius: Self.bottomCornerRadius,
                                           bottomTrailingRadius: Self.bottomCornerRadius,
                                           topTrailingRadius: 0,
                                           style: .continuous)
                        .fill(Color.black.opacity(MusicModule.lyricStripOpacity)))
        }
    }

    private func text(for content: StripContent) -> String {
        switch content {
        case .line(let text), .title(let text): return text
        case .none: return ""
        }
    }

    private func isLine(_ content: StripContent) -> Bool {
        if case .line = content { return true }
        return false
    }
}
