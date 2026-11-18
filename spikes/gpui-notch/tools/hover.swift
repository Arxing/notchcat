// Moves the cursor (CGWarpMouseCursorPosition, no clicks) through hover
// scenarios. Usage: hover <centerX> <notchH>. CG coords (top-left origin, primary display).
import CoreGraphics
import Foundation
let cx = Double(CommandLine.arguments[1])!
let nh = Double(CommandLine.arguments[2])!
let saved = CGEvent(source: nil)!.location
let t0 = Date()
func stamp() -> String { String(format: "%.3f", Date().timeIntervalSince1970) }
func go(_ x: Double, _ y: Double) {
    CGWarpMouseCursorPosition(CGPoint(x: x, y: y))
    CGAssociateMouseAndMouseCursorPosition(1)
}
func step(_ name: String) { print("[\(stamp())] STEP \(name)"); fflush(stdout) }
func wait(_ s: Double) { usleep(useconds_t(s * 1_000_000)) }
// glide from a to b in n substeps over dur
func glide(_ a: (Double, Double), _ b: (Double, Double), _ dur: Double, _ n: Int = 20) {
    for i in 1...n { let t = Double(i)/Double(n); go(a.0 + (b.0-a.0)*t, a.1 + (b.1-a.1)*t); wait(dur/Double(n)) }
}
let out = (cx, 320.0), belowPill = (cx, nh + 60), inPill = (cx - 120, nh / 2), deep = (cx, 170.0)

step("1 start outside"); go(out.0, out.1); wait(0.6)
step("2 into container but below pill (expect NO expand)"); glide(out, belowPill, 0.3); wait(0.8)
step("3 up into pill (expect 1 enter + expand)"); glide(belowPill, inPill, 0.2); wait(0.2)
step("4 hold still 1.5s with +-1pt jitter (expect nothing)")
for i in 0..<50 { go(inPill.0 + (i % 2 == 0 ? 1 : -1), inPill.1 + (i % 3 == 0 ? 1 : 0)); wait(0.03) }
step("5 move down inside expanded container (expect nothing)"); glide(inPill, deep, 0.3); wait(0.5)
step("6 leave container (expect 1 exit, collapse 150ms later)"); glide(deep, out, 0.15); wait(0.8)
step("7 enter pill then immediately dive to y=170 during the 0.28s animation (expect no exit)")
glide(out, inPill, 0.15); glide(inPill, deep, 0.08, 4); wait(0.6)
step("8 out-and-back within 100ms (expect exit+enter, NO collapse)")
go(cx, 260); wait(0.08); go(deep.0, deep.1); wait(0.5)
step("9 leave (expect exit + collapse)"); glide(deep, out, 0.1); wait(0.6)
step("10 rapid in/out of pill x6, 120ms each side")
for _ in 0..<6 { go(inPill.0, inPill.1); wait(0.12); go(out.0, out.1); wait(0.12) }
wait(0.5)
step("11 jitter across pill bottom edge while collapsed, 1.5s")
for i in 0..<50 { go(cx - 120, nh + (i % 2 == 0 ? 2 : -2)); wait(0.03) }
step("12 leave"); go(out.0, out.1); wait(0.6)
step("13 rest still under the pill (collapsed, transparent area) 1s (expect nothing)"); go(cx, nh + 80); wait(1.0)
go(saved.x, saved.y)
step("done, cursor restored to \(saved), elapsed \(String(format: "%.1f", Date().timeIntervalSince(t0)))s")
