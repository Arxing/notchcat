import SwiftUI

/// A few bars that bounce while music plays and settle low when it does not.
///
/// Each bar runs one repeating animation that Core Animation drives, so
/// nothing is recomputed per frame; turning `isAnimating` off replaces it with
/// a short settle and the bars stop moving.
struct EqualizerBars: View {
    let isAnimating: Bool

    init(isAnimating: Bool) {
        self.isAnimating = isAnimating
    }

    static let barWidth: CGFloat = 3
    static let barSpacing: CGFloat = 2
    static let height: CGFloat = 14
    /// Per-bar loop length and peak, so the bars do not move in step.
    private static let durations: [Double] = [0.45, 0.62, 0.38, 0.54]
    private static let peaks: [CGFloat] = [1.0, 0.75, 0.9, 0.6]

    var body: some View {
        HStack(alignment: .bottom, spacing: Self.barSpacing) {
            ForEach(Self.durations.indices, id: \.self) { index in
                Bar(duration: Self.durations[index], peak: Self.peaks[index], isAnimating: isAnimating)
            }
        }
        .frame(width: Self.width, height: Self.height, alignment: .bottom)
        .accessibilityHidden(true)
    }

    /// Bars plus the gaps between them.
    static var width: CGFloat {
        CGFloat(durations.count) * barWidth + CGFloat(durations.count - 1) * barSpacing
    }
}

private struct Bar: View {
    let duration: Double
    let peak: CGFloat
    let isAnimating: Bool
    @State private var raised = false

    private static let restingScale: CGFloat = 0.25

    var body: some View {
        Capsule()
            .fill(Color.white.opacity(0.9))
            .frame(width: EqualizerBars.barWidth, height: EqualizerBars.height)
            .scaleEffect(y: raised ? peak : Self.restingScale, anchor: .bottom)
            .onAppear { update(isAnimating) }
            .onChange(of: isAnimating) { _, now in update(now) }
    }

    private func update(_ animating: Bool) {
        if animating {
            withAnimation(.easeInOut(duration: duration).repeatForever(autoreverses: true)) {
                raised = true
            }
        } else {
            withAnimation(.easeOut(duration: 0.2)) { raised = false }
        }
    }
}
