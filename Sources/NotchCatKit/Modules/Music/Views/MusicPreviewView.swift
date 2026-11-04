import SwiftUI

/// Preview: two wings flanking the physical notch, plus the lyric strip below.
///
/// The shell lays this out in the full Preview size, (notchWidth + 160) wide
/// and notch height + extra height tall, with no top padding.
///
///     |<-- wing -->|<-- notchWidth -->|<-- wing -->|     wing = 80pt
///     [cover]        (hidden by notch)       [bars]    height = notch height
///     [          lyric strip, 24pt, own translucent background          ]
///
/// The strip takes its fixed `MusicModule.lyricStripHeight` from the bottom,
/// so the wing row gets exactly what remains, which is the notch height while
/// the strip is shown (the module asks for that extra height only then) and
/// the whole height otherwise. Nothing is drawn in the middle `notchWidth`.
struct MusicPreviewView: View {
    @ObservedObject var viewModel: MusicViewModel
    @ObservedObject var preference: LyricStripPreference
    /// Width of the physical notch; the middle of the wing row stays empty.
    let notchWidth: CGFloat

    /// Gap between a wing's outer edge and its content.
    private static let coverInset: CGFloat = 8
    private static let barsInset: CGFloat = 12
    /// Space kept above and below the cover inside the wing row.
    private static let coverVerticalInset: CGFloat = 4

    private var showsStrip: Bool {
        preference.isVisible && viewModel.snapshot?.isPlaying == true
    }

    var body: some View {
        VStack(spacing: 0) {
            wings
            if showsStrip {
                LyricStripView(viewModel: viewModel)
            }
        }
    }

    private var wings: some View {
        HStack(spacing: 0) {
            // Left wing: cover, flush to the bar's left end.
            GeometryReader { proxy in
                let side = max(0, min(proxy.size.height - 2 * Self.coverVerticalInset,
                                      proxy.size.width - Self.coverInset))
                CoverView(viewModel.snapshot?.artwork)
                    .frame(width: side, height: side)
                    .padding(.leading, Self.coverInset)
                    .frame(width: proxy.size.width, height: proxy.size.height, alignment: .leading)
            }
            .frame(maxWidth: .infinity)

            // Behind the notch: nothing important may be drawn here.
            Color.clear.frame(width: notchWidth)

            // Right wing: equalizer, flush to the bar's right end.
            EqualizerBars(isAnimating: viewModel.snapshot?.isPlaying == true)
                .padding(.trailing, Self.barsInset)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
        }
    }
}
