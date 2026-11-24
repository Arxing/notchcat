// Prints CGWindowList info (layer, bounds, alpha, owner) for windows owned by a PID.
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
let menubar = list.filter { ($0[kCGWindowName as String] as? String) == "Menubar" || ($0[kCGWindowOwnerName as String] as? String) == "Window Server" && ($0[kCGWindowLayer as String] as? Int) == 24 }
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
    print("OURS number=\(w[kCGWindowNumber as String]!) layer=\(w[kCGWindowLayer as String]!) bounds=\(w[kCGWindowBounds as String]!) alpha=\(w[kCGWindowAlpha as String]!) onscreen=\(w[kCGWindowIsOnscreen as String] ?? "nil") name=\(w[kCGWindowName as String] ?? "")")
}
for w in menubar.prefix(3) { print("MENUBAR layer=\(w[kCGWindowLayer as String]!) bounds=\(w[kCGWindowBounds as String]!) owner=\(w[kCGWindowOwnerName as String]!)") }
print("kCGMainMenuWindowLevel=\(CGWindowLevelForKey(.mainMenuWindow)) kCGStatusWindowLevel=\(CGWindowLevelForKey(.statusWindow))")
