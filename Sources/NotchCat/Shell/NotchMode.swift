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
}
