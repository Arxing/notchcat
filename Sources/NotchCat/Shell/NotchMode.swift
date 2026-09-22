import CoreGraphics

/// The three panel modes described in docs/PLANNING.md.
enum NotchMode: Equatable {
    case hidden
    case expanded
    case preview
}

extension NotchMode {
    /// Placeholder sizing for Phase 0. Once ModuleKit exists (Phase 1+), the
    /// active module drives Expanded/Preview sizing instead of these fixed
    /// fake-content values, which only exist to prove the state machine and
    /// the resize animation work.
    func size(for geometry: NotchGeometry) -> CGSize {
        switch self {
        case .hidden:
            return geometry.notchSize
        case .expanded:
            return CGSize(width: max(geometry.notchSize.width * 2.4, 320),
                          height: 180)
        case .preview:
            return CGSize(width: geometry.notchSize.width * 1.8,
                          height: geometry.notchSize.height + 8)
        }
    }

    /// Frame for this mode, anchored to the notch's top-center so every mode
    /// grows/shrinks around the same fixed point.
    func frame(for geometry: NotchGeometry) -> CGRect {
        let size = size(for: geometry)
        return CGRect(x: geometry.centerX - size.width / 2,
                       y: geometry.topY - size.height,
                       width: size.width,
                       height: size.height)
    }

    /// The panel's own frame never changes at runtime — see
    /// `NotchShellController`. It's sized to fit every mode's footprint so
    /// the hover hit-test area is always the same stable, generous size:
    /// a hit-test area that shrinks/grows in sync with the mode-change
    /// animation is only as forgiving as its smallest transient size, and
    /// Hidden's is a ~32pt-tall sliver — thin enough that ordinary cursor
    /// jitter while "holding still" crosses its edge and causes the shell to
    /// flicker open/closed. Only the drawn content animates its size within
    /// this fixed container.
    static func containerSize(for geometry: NotchGeometry) -> CGSize {
        let modes: [NotchMode] = [.hidden, .expanded, .preview]
        let sizes = modes.map { $0.size(for: geometry) }
        return CGSize(width: sizes.map(\.width).max() ?? 0,
                       height: sizes.map(\.height).max() ?? 0)
    }

    static func containerFrame(for geometry: NotchGeometry) -> CGRect {
        let size = containerSize(for: geometry)
        return CGRect(x: geometry.centerX - size.width / 2,
                       y: geometry.topY - size.height,
                       width: size.width,
                       height: size.height)
    }

    /// This mode's hover hit-test rect, in the fixed container's own
    /// top-anchored local coordinate space (see `containerFrame`). Used to
    /// require a precise hover over the visible Hidden pill to *open* the
    /// shell, while `NotchShellController` widens the active hit-test rect
    /// to the whole container the instant it opens, so staying open is
    /// forgiving. NSHostingView is flipped (origin top-left), matching the
    /// top-anchored layout in NotchContentView.
    func localRect(for geometry: NotchGeometry) -> CGRect {
        let container = NotchMode.containerSize(for: geometry)
        let size = size(for: geometry)
        return CGRect(x: (container.width - size.width) / 2,
                       y: 0,
                       width: size.width,
                       height: size.height)
    }
}
