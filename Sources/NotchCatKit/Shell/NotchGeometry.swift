import AppKit

/// Physical geometry of the hardware notch, expressed in screen coordinates
/// (AppKit's bottom-left-origin space).
struct NotchGeometry {
    /// The screen that has the notch (the built-in display, when present).
    let screen: NSScreen
    /// Width/height of the notch cutout itself.
    let notchSize: CGSize
    /// X coordinate of the notch's horizontal center, in screen coordinates.
    let centerX: CGFloat
    /// The top edge of the screen (== screen.frame.maxY).
    let topY: CGFloat
}

enum NotchDetector {
    /// Finds the screen exposing the macOS 12+ notch-avoidance APIs and
    /// derives the notch's own rectangle from them.
    ///
    /// `auxiliaryTopLeftArea` / `auxiliaryTopRightArea` describe the menu-bar
    /// strips to the left and right of the notch; there is no direct API for
    /// the notch's own rectangle, so it's recovered by subtracting those two
    /// from the full screen width.
    static func detect() -> NotchGeometry? {
        for screen in NSScreen.screens {
            guard let left = screen.auxiliaryTopLeftArea,
                  let right = screen.auxiliaryTopRightArea else { continue }

            let frame = screen.frame
            let notchWidth = frame.width - left.width - right.width
            guard notchWidth > 0 else { continue }

            // The auxiliary areas run at full menu-bar height, which matches
            // the notch's own height on every notched Mac so far.
            let notchHeight = left.height

            return NotchGeometry(
                screen: screen,
                notchSize: CGSize(width: notchWidth, height: notchHeight),
                centerX: frame.minX + left.width + notchWidth / 2,
                topY: frame.maxY
            )
        }
        return nil
    }
}
