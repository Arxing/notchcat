// Short cursor run for animation timing: enter pill, hold, leave. ~3 s. No clicks.
import CoreGraphics
import Foundation
let cx = Double(CommandLine.arguments[1])!, nh = Double(CommandLine.arguments[2])!
let saved = CGEvent(source: nil)!.location
func st(_ n: String) { print(String(format: "[%.3f] STEP ", Date().timeIntervalSince1970) + n); fflush(stdout) }
CGWarpMouseCursorPosition(CGPoint(x: cx, y: 320)); usleep(400_000)
st("enter pill"); CGWarpMouseCursorPosition(CGPoint(x: cx - 120, y: nh / 2)); usleep(1_200_000)
st("leave"); CGWarpMouseCursorPosition(CGPoint(x: cx, y: 320)); usleep(1_000_000)
CGWarpMouseCursorPosition(saved); st("restored \(saved)")
