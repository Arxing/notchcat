import AppKit
import SwiftUI

/// NSHostingView subclass that turns mouse hover into plain closures, so the
/// shell controller can react to hover without knowing anything about
/// AppKit's tracking-area/responder-chain mechanics.
final class NotchHostingView<Content: View>: NSHostingView<Content> {
    var onMouseEntered: (() -> Void)?
    var onMouseExited: (() -> Void)?

    private var trackingArea: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let trackingArea {
            removeTrackingArea(trackingArea)
        }
        // .inVisibleRect keeps the tracking rect in sync as the view resizes
        // during the Hidden/Expanded/Preview frame animation.
        let area = NSTrackingArea(rect: bounds,
                                   options: [.activeAlways, .mouseEnteredAndExited, .inVisibleRect],
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
