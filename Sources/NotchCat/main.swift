import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var shellController: NotchShellController?
    private var statusItem: NSStatusItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Accessory apps have no Dock icon and no app menu — appropriate for
        // a background utility that only ever shows itself over the notch.
        NSApp.setActivationPolicy(.accessory)

        guard let geometry = NotchDetector.detect() else {
            // Phase 0 scope: no notch, no fallback UI yet.
            NSLog("NotchCat: no notch detected on this Mac, exiting.")
            NSApp.terminate(nil)
            return
        }

        shellController = NotchShellController(geometry: geometry)
        setupStatusItem()
    }

    private func setupStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(systemSymbolName: "cat", accessibilityDescription: "NotchCat")

        let menu = NSMenu()
        let quitItem = NSMenuItem(title: "Quit NotchCat", action: #selector(quit), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)
        item.menu = menu

        statusItem = item
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}

let delegate = AppDelegate()
let app = NSApplication.shared
app.delegate = delegate
app.run()
