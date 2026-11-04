import SwiftUI

/// The pure rules behind `MarqueeText`, kept apart so they can be tested.
enum MarqueeLayout {
    /// Scroll speed in points per second.
    static let pointsPerSecond: CGFloat = 30
    /// Empty space between the end of the text and its repeat.
    static let gap: CGFloat = 32

    /// Text scrolls only when it is wider than its container. A container
    /// with no width yet (before the first layout pass) never scrolls.
    static func needsScroll(textWidth: CGFloat, containerWidth: CGFloat) -> Bool {
        containerWidth > 0 && textWidth > containerWidth
    }

    /// Time for one full loop, in seconds.
    static func loopDuration(textWidth: CGFloat) -> Double {
        Double((textWidth + gap) / pointsPerSecond)
    }
}

/// A single line that scrolls sideways when it does not fit, and otherwise
/// sits still, centered. It takes whatever width its parent offers and never
/// asks for more, so a very long title cannot widen the layout.
struct MarqueeText: View {
    private let text: String
    private let font: Font
    @State private var textWidth: CGFloat = 0

    init(_ text: String, font: Font) {
        self.text = text
        self.font = font
    }

    var body: some View {
        // This invisible copy only supplies the height and a width equal to
        // what the parent offers: a plain Text truncates instead of growing.
        Text(text)
            .font(font)
            .lineLimit(1)
            .opacity(0)
            .frame(maxWidth: .infinity)
            // Natural, untruncated width of the text. A background cannot
            // change the size of the view it is attached to.
            .background(alignment: .leading) {
                label
                    .opacity(0)
                    .background(GeometryReader { proxy in
                        Color.clear.preference(key: MarqueeTextWidthKey.self, value: proxy.size.width)
                    })
            }
            .onPreferenceChange(MarqueeTextWidthKey.self) { textWidth = $0 }
            .overlay(alignment: .leading) {
                GeometryReader { proxy in
                    visibleContent(containerWidth: proxy.size.width)
                }
            }
            .clipped()
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(text)
    }

    private var label: some View {
        Text(text).font(font).lineLimit(1).fixedSize()
    }

    @ViewBuilder
    private func visibleContent(containerWidth: CGFloat) -> some View {
        if MarqueeLayout.needsScroll(textWidth: textWidth, containerWidth: containerWidth) {
            // A new text or width restarts the loop from the beginning.
            ScrollingLabel(text: text, font: font, textWidth: textWidth)
                .id("\(text)|\(textWidth)")
                .frame(width: containerWidth, alignment: .leading)
        } else {
            label.frame(width: containerWidth, alignment: .center)
        }
    }
}

private struct MarqueeTextWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

/// Two copies of the text, one gap apart, slid left by exactly one copy plus
/// the gap per loop so the repeat joins seamlessly. One repeating animation
/// started once: no timer and no per-frame work in SwiftUI.
private struct ScrollingLabel: View {
    let text: String
    let font: Font
    let textWidth: CGFloat
    @State private var shifted = false

    var body: some View {
        HStack(spacing: MarqueeLayout.gap) {
            copy
            copy
        }
        .offset(x: shifted ? -(textWidth + MarqueeLayout.gap) : 0)
        .onAppear {
            withAnimation(.linear(duration: MarqueeLayout.loopDuration(textWidth: textWidth))
                .repeatForever(autoreverses: false)) {
                shifted = true
            }
        }
    }

    private var copy: some View {
        Text(text).font(font).lineLimit(1).fixedSize()
    }
}
