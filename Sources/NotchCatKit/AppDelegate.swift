import AppKit

/// Lives in the library target (not alongside `main.swift`) so the executable
/// stays a three-line entry point and everything else remains unit-testable.
/// `public` only because the executable target has to see it — nothing else
/// here needs to cross the module boundary.
public final class AppDelegate: NSObject, NSApplicationDelegate {
    private var shellController: NotchShellController?
    private var statusItem: NSStatusItem?

    public override init() {
        super.init()
    }

    public func applicationDidFinishLaunching(_ notification: Notification) {
        // Accessory apps have no Dock icon and no app menu — appropriate for
        // a background utility that only ever shows itself over the notch.
        NSApp.setActivationPolicy(.accessory)

        guard let geometry = NotchDetector.detect() else {
            // Phase 0 scope: no notch, no fallback UI yet.
            NSLog("NotchCat: no notch detected on this Mac, exiting.")
            NSApp.terminate(nil)
            return
        }

        let moduleManager = ModuleManager()
        moduleManager.register(MusicModule.makeDefault(notchWidth: geometry.notchSize.width))

        shellController = NotchShellController(geometry: geometry, moduleManager: moduleManager)
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
