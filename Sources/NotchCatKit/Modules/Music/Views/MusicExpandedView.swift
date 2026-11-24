import SwiftUI

/// Expanded: cover, track info, progress, transport controls, the lyric
/// strip's fold switch, and the lyrics.
///
/// Space: the shell makes Expanded 180pt tall with 8pt of top padding, then
/// stacks the AppBar (about 20pt) and 8pt of spacing above this view, so this
/// view gets about 172 - 20 - 8 = 144pt. Width is notchWidth + 160 (about
/// 360pt). Budget inside the view:
///
///     8   bottom padding
///     68  header (cover 68x68 beside the info column)
///     6   gap
///     <=62 lyrics (flexible; shrinks, never overflows)
///
/// so the fixed parts take 82pt and the lyrics get the remaining ~62pt (four
/// lines of 11pt text). The root clips as a last guard.
struct MusicExpandedView: View {
    @ObservedObject var viewModel: MusicViewModel
    @ObservedObject var preference: LyricStripPreference

    private static let coverSide: CGFloat = 68
    private static let controlRowHeight: CGFloat = 24

    var body: some View {
        Group {
            if let snapshot = viewModel.snapshot {
                player(snapshot)
            } else {
                Text("目前沒有播放")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.white.opacity(0.6))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
    }

    private func player(_ snapshot: NowPlayingSnapshot) -> some View {
        VStack(spacing: 6) {
            HStack(alignment: .top, spacing: 12) {
                CoverView(snapshot.artwork)
                    .frame(width: Self.coverSide, height: Self.coverSide)
                info(snapshot)
                    .frame(maxWidth: .infinity)
                    .frame(height: Self.coverSide)
            }
            if let lyrics = viewModel.lyrics {
                ExpandedLyricsView(snapshot: snapshot, lyrics: lyrics)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        // Without lyrics the header block is centered in the space.
        .frame(maxHeight: .infinity, alignment: .center)
    }

    /// Title (16pt line), artist (14pt line, omitted when nil), progress
    /// (3pt + 4pt gap, omitted without duration and position) and the control
    /// row (24pt): at most 61pt of the 68pt column; a spacer takes the rest.
    private func info(_ snapshot: NowPlayingSnapshot) -> some View {
        VStack(spacing: 0) {
            MarqueeText(snapshot.title, font: .system(size: 13, weight: .semibold))
                .foregroundStyle(Color.white)
            if let artist = snapshot.artist {
                MarqueeText(artist, font: .system(size: 11))
                    .foregroundStyle(Color.white.opacity(0.65))
            }
            Spacer(minLength: 2)
            if MusicPresentation.hasProgress(snapshot: snapshot) {
                ProgressBar(snapshot: snapshot)
                    .padding(.bottom, 4)
            }
            controls(snapshot)
        }
    }

    private func controls(_ snapshot: NowPlayingSnapshot) -> some View {
        let enabled = viewModel.hasControl
        return HStack(spacing: 0) {
            // Balances the fold switch on the right so the buttons stay centered.
            Color.clear.frame(width: Self.controlRowHeight, height: Self.controlRowHeight)
            Spacer(minLength: 0)
            HStack(spacing: 14) {
                controlButton("backward.fill", size: 13, label: "上一首", enabled: enabled) {
                    viewModel.send(.previous)
                }
                controlButton(snapshot.isPlaying ? "pause.fill" : "play.fill", size: 16,
                              label: snapshot.isPlaying ? "暫停" : "播放", enabled: enabled) {
                    viewModel.send(.togglePlayPause)
                }
                controlButton("forward.fill", size: 13, label: "下一首", enabled: enabled) {
                    viewModel.send(.next)
                }
            }
            Spacer(minLength: 0)
            Button {
                preference.isVisible.toggle()
            } label: {
                Image(systemName: preference.isVisible ? "chevron.up" : "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.6))
                    .frame(width: Self.controlRowHeight, height: Self.controlRowHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(preference.isVisible ? "收起歌詞條" : "展開歌詞條")
            .accessibilityLabel(preference.isVisible ? "收起歌詞條" : "展開歌詞條")
        }
        .frame(height: Self.controlRowHeight)
    }

    private func controlButton(_ symbol: String, size: CGFloat, label: String, enabled: Bool,
                               action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size))
                .foregroundStyle(Color.white.opacity(enabled ? 0.95 : 0.3))
                .frame(width: 28, height: Self.controlRowHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

/// Thin progress bar. A TimelineView re-reads the position once a second, so
/// there is no timer to manage; the fraction is clamped to 0...1.
private struct ProgressBar: View {
    let snapshot: NowPlayingSnapshot

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let fraction = MusicPresentation.progressFraction(snapshot: snapshot, at: context.date) ?? 0
            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.white.opacity(0.2))
                    Capsule().fill(Color.white.opacity(0.85))
                        .frame(width: proxy.size.width * fraction)
                        .animation(.linear(duration: 1), value: fraction)
                }
            }
            .frame(height: 3)
        }
        .accessibilityHidden(true)
    }
}

/// The lyrics area. Plain text is static and scrollable. Synced lyrics with a
/// known position follow playback and highlight the current line; without a
/// position they are static full text.
private struct ExpandedLyricsView: View {
    let snapshot: NowPlayingSnapshot
    let lyrics: Lyrics

    var body: some View {
        switch lyrics {
        case .plain(let text):
            ScrollView {
                Text(text)
                    .font(LyricStyle.font)
                    .foregroundStyle(Color.white.opacity(0.75))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
            }
            .scrollIndicators(.hidden)
        case .synced(let lines):
            if snapshot.position != nil {
                FollowingLyrics(snapshot: snapshot, lyrics: lyrics, lines: lines)
            } else {
                ScrollView {
                    SyncedLines(lines: lines, current: nil)
                }
                .scrollIndicators(.hidden)
            }
        }
    }
}

private enum LyricStyle {
    static let font = Font.system(size: 11)
    static let currentFont = Font.system(size: 11, weight: .semibold)
}

/// All lines of synced lyrics, the current one (if any) brighter.
private struct SyncedLines: View {
    let lines: [LyricLine]
    let current: Int?

    var body: some View {
        VStack(spacing: 3) {
            ForEach(lines.indices, id: \.self) { index in
                let isCurrent = index == current
                Text(lines[index].text)
                    .font(isCurrent ? LyricStyle.currentFont : LyricStyle.font)
                    .foregroundStyle(Color.white.opacity(isCurrent ? 1 : 0.45))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .id(index)
            }
        }
    }
}

/// Synced lyrics that scroll to the current line, re-evaluated four times a
/// second by a TimelineView.
private struct FollowingLyrics: View {
    let snapshot: NowPlayingSnapshot
    let lyrics: Lyrics
    let lines: [LyricLine]

    var body: some View {
        ScrollViewReader { proxy in
            TimelineView(.periodic(from: .now, by: 0.25)) { context in
                let current = currentIndex(at: context.date)
                ScrollView {
                    SyncedLines(lines: lines, current: current)
                }
                .scrollIndicators(.hidden)
                .onAppear { scroll(proxy, to: current, animated: false) }
                .onChange(of: current) { _, new in scroll(proxy, to: new, animated: true) }
            }
        }
    }

    private func currentIndex(at date: Date) -> Int? {
        MusicPresentation.playbackPosition(snapshot: snapshot, at: date)
            .flatMap { lyrics.lineIndex(at: $0) }
    }

    private func scroll(_ proxy: ScrollViewProxy, to index: Int?, animated: Bool) {
        guard let index else { return }
        if animated {
            withAnimation(.easeInOut(duration: 0.3)) { proxy.scrollTo(index, anchor: .center) }
        } else {
            proxy.scrollTo(index, anchor: .center)
        }
    }
}
