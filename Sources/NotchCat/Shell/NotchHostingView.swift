import AppKit
import SwiftUI

/// NSHostingView subclass that turns mouse hover into plain closures, so the
/// shell controller can react to hover without knowing anything about
/// AppKit's tracking-area/responder-chain mechanics.
final class NotchHostingView<Content: View>: NSHostingView<Content> {
    var onMouseEntered: (() -> Void)?
    var onMouseExited: (() -> Void)?

    private var trackingArea: NSTrackingArea?

    /// The rect (in this view's own coordinate space) that currently counts
    /// as "hovering the shell". The caller sets this explicitly and updates
    /// it instantly on each mode change — see `NotchShellController` — rather
    /// than letting it track the view's bounds or an animating content size,
    /// which is what previously caused expand/collapse flicker.
    var hoverRect: CGRect = .zero {
        didSet {
            guard hoverRect != oldValue else { return }
            rebuildTrackingArea()
        }
    }

    private func rebuildTrackingArea() {
        if let trackingArea {
            removeTrackingArea(trackingArea)
        }
        let area = NSTrackingArea(rect: hoverRect,
                                   options: [.activeAlways, .mouseEnteredAndExited],
                                   owner: self,
                                   userInfo: nil)
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        onMouseEntered?()
    }

    override func mouseExited(with event: NSEvent) {
        onMouseExited?()
    }
}
