# Phase 2 Music 模組實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 NotchCat 的瀏海面板上顯示系統層級的現正播放資訊與逐行同步歌詞，作為 Phase 1 模組系統之後的第一個真實模組。

**Architecture:** 私有 API（MediaRemote）全部隔離在單一 bridge 檔案後面，其餘程式碼只接觸純值型別。現正播放的更新採「通知為主、兩檔保底查詢」；播放位置不輪詢，由時間戳與播放速率本地推算。歌詞來源做成可抽換的 provider 清單，解析時同步歌詞的優先級高於來源順位。

**Tech Stack:** Swift 5.9 工具鏈（編譯器 6.2）、SwiftUI、AppKit、Combine、XCTest、SPM。

**Spec:** [`docs/superpowers/specs/2026-10-14-music-module-design.md`](../specs/2026-10-14-music-module-design.md)

## Global Constraints

- 最低系統需求 macOS 14，`Package.swift` 的 `platforms: [.macOS(.v14)]` 不得調降。
- `swift-tools-version: 5.9` 維持不變。**不要升到 6.0**：那會啟用 Swift 6 語言模式的嚴格並行檢查，既有的 Shell 程式碼會大量報錯，不在本計畫範圍內。因此測試框架用 XCTest，不用 swift-testing。
- **私有 API 只准出現在 `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteBridge.swift` 一個檔案裡。** 其餘任何檔案不得出現 `dlopen`、`dlsym`、或任何 `MRMediaRemote*` 符號名稱。
- 歌詞來源逾時一律 5 秒，逾時視同未命中，不得阻塞畫面。
- 曲目長度比對採 ±5 秒容差，不得使用相等比較。
- 字串比對不得使用完全相等，必須容忍簡繁體差異。
- 所有新增檔案置於 `Sources/NotchCatKit/`，測試置於 `Tests/NotchCatKitTests/`。
- 每個 Task 結束時 `swift build` 與 `swift test` 都必須通過才能提交。

## Review Focus

spec 隱含但沒有任何任務的測試會自然涵蓋的輸入類別，依「使用者實際踩到的機率」排序。每一條都已在下方對應任務補上測試。

1. **MediaRemote 回傳缺欄位的字典。** 實測看到的是完整的 13 個鍵，但切歌瞬間、廣播串流、或播放器自身狀態未就緒時，歌名或演出者可能缺席。缺欄位不得造成崩潰，應視為「無曲目」。測試歸 Task 3。
2. **播放速率不等於 1。** 有聲書與 podcast 常用 1.5 倍或 2 倍速。`PlaybackClock` 若寫死速率為 1，進度條與歌詞捲動會整個對不上。測試歸 Task 3。
3. **畸形的 LRC 內容。** 社群歌詞資料庫品質參差：時間軸未排序、超出曲長、負數、同一時間多行、空行、以及完全沒有時間標籤卻被當成同步歌詞。解析器不得崩潰，且必須把「沒有任何有效時間標籤」判為非同步。測試歸 Task 5。
4. **快速連續切歌造成的競態。** 連按下一首時，前一首的歌詞請求可能晚於後一首才回來，把舊歌詞套到新歌上。這是使用者一定會遇到、而且症狀很明顯的錯誤。測試歸 Task 6。
5. **極長的歌名與演出者。** 中文曲目加上 `(feat. …)` 後綴很容易超出瀏海寬度。文字必須截斷或跑馬燈，不得把版面推開或把控制鈕擠出畫面。測試歸 Task 11。

---

## Task 1：拆出 library target，讓單元測試做得起來

目前 `Package.swift` 只有一個 `executableTarget`，SPM 對 executable target 做單元測試會踩到連結問題，而 spec 第 10 節要求的純邏輯測試全都需要能 `@testable import`。本任務把程式碼搬進 library target，執行檔只留進入點。

**搬移策略刻意選最小改動版本：`AppDelegate` 也搬進 library，因此只有它需要改成 `public`，其餘型別維持 internal，測試靠 `@testable import` 存取。**

**Files:**
- Modify: `Package.swift`
- Create: `Sources/NotchCatKit/AppDelegate.swift`（內容來自現有 `main.swift`）
- Modify: `Sources/NotchCat/main.swift`（縮減為進入點）
- Move: `Sources/NotchCat/{Shell,UI,ModuleKit,Modules}` → `Sources/NotchCatKit/{Shell,UI,ModuleKit,Modules}`
- Create: `Tests/NotchCatKitTests/PackageSmokeTests.swift`

**Interfaces:**
- Consumes: 無（本任務是全計畫的起點）
- Produces: library target 名稱 `NotchCatKit`；`public final class AppDelegate: NSObject, NSApplicationDelegate`，具 `public override init()`；測試得以 `@testable import NotchCatKit`

- [ ] **Step 1：搬移原始碼目錄**

```bash
cd /Users/arxing/Workspace/notchcat
mkdir -p Sources/NotchCatKit
git mv Sources/NotchCat/Shell Sources/NotchCatKit/Shell
git mv Sources/NotchCat/UI Sources/NotchCatKit/UI
git mv Sources/NotchCat/ModuleKit Sources/NotchCatKit/ModuleKit
git mv Sources/NotchCat/Modules Sources/NotchCatKit/Modules
```

- [ ] **Step 2：把 AppDelegate 從 main.swift 抽到 library**

建立 `Sources/NotchCatKit/AppDelegate.swift`：

```swift
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
        NSApp.setActivationPolicy(.accessory)

        guard let geometry = NotchDetector.detect() else {
            NSLog("NotchCat: no notch detected on this Mac, exiting.")
            NSApp.terminate(nil)
            return
        }

        let moduleManager = ModuleManager()
        moduleManager.install(ClockModule())

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
```

- [ ] **Step 3：把 main.swift 縮減為進入點**

`Sources/NotchCat/main.swift` 全部內容換成：

```swift
import AppKit
import NotchCatKit

let delegate = AppDelegate()
let app = NSApplication.shared
app.delegate = delegate
app.run()
```

- [ ] **Step 4：更新 Package.swift**

```swift
// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NotchCat",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "NotchCatKit"),
        .executableTarget(name: "NotchCat", dependencies: ["NotchCatKit"]),
        .testTarget(name: "NotchCatKitTests", dependencies: ["NotchCatKit"]),
    ]
)
```

- [ ] **Step 5：寫一個會失敗的煙霧測試**

建立 `Tests/NotchCatKitTests/PackageSmokeTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class PackageSmokeTests: XCTestCase {
    /// Proves the test target can see the library's internal types at all.
    /// If this fails, every later task's tests are unrunnable.
    func testCanReachInternalTypes() {
        let manager = ModuleManager()
        XCTAssertNil(manager.activeModule)
        XCTAssertTrue(manager.installedModules.isEmpty)
    }
}
```

- [ ] **Step 6：執行測試確認通過**

Run: `swift test 2>&1 | tail -20`
Expected: PASS，`PackageSmokeTests` 一項通過。若報 `no such module 'NotchCatKit'`，表示 Step 1 的檔案沒搬乾淨。

- [ ] **Step 7：確認 App 仍可建置並執行**

Run: `swift build && ./.build/debug/NotchCat`
Expected: 建置通過，選單列出現貓咪圖示，hover 瀏海仍會展開並顯示時鐘。確認後 Ctrl-C 結束。

- [ ] **Step 8：提交**

```bash
git add -A
git commit -m "Split NotchCatKit library out of the executable for testability"
```

---

## Task 2：MediaRemote bridge，並驗證播放控制是否可行

封裝私有 API，**同時解決 spec 第 8 節標記的唯一未驗證假設**：讀取 now-playing 已實測可行，但「送出播放控制指令」從未驗證過，寫入類操作有可能受到比讀取更嚴格的限制。

> **這個步驟會實際影響正在播放的音樂（短暫暫停再恢復）。執行前先告知使用者。**
> **若驗證失敗：** 跳過 Task 10，並把 Task 11 的 Expanded 控制列改為唯讀的播放狀態指示。spec 第 7 節的版面需同步修訂。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteBridge.swift`
- Create: `Tests/NotchCatKitTests/Music/MediaRemoteBridgeTests.swift`

**Interfaces:**
- Consumes: Task 1 的 `NotchCatKit` target
- Produces:
  - `enum MediaRemoteCommand: UInt32 { case play = 0, pause = 1, togglePlayPause = 2, next = 4, previous = 5 }`
  - `protocol MediaRemoteProviding { func nowPlayingInfo() async -> [String: Any]?; func isPlaying() async -> Bool; @discardableResult func send(_ command: MediaRemoteCommand) -> Bool }`
  - `final class MediaRemoteBridge: MediaRemoteProviding`，失敗時 `init?` 回 nil

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/MediaRemoteBridgeTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class MediaRemoteBridgeTests: XCTestCase {
    /// The bridge must construct on a supported machine. A nil here means the
    /// private framework moved or its symbols were withdrawn — the module is
    /// expected to disable itself cleanly in that case (see spec section 9).
    func testBridgeConstructs() throws {
        let bridge = try XCTUnwrap(MediaRemoteBridge(), "MediaRemote 無法載入")
        _ = bridge
    }

    func testCommandRawValuesMatchMediaRemote() {
        XCTAssertEqual(MediaRemoteCommand.togglePlayPause.rawValue, 2)
        XCTAssertEqual(MediaRemoteCommand.next.rawValue, 4)
        XCTAssertEqual(MediaRemoteCommand.previous.rawValue, 5)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter MediaRemoteBridgeTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'MediaRemoteBridge' in scope`

- [ ] **Step 3：實作 bridge**

建立 `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteBridge.swift`：

```swift
import Foundation

/// Raw MediaRemote command codes.
enum MediaRemoteCommand: UInt32 {
    case play = 0
    case pause = 1
    case togglePlayPause = 2
    case next = 4
    case previous = 5
}

/// Everything the Music module needs from the system's now-playing service.
/// Exists so the rest of the module can be built and tested against a fake.
protocol MediaRemoteProviding: AnyObject {
    func nowPlayingInfo() async -> [String: Any]?
    func isPlaying() async -> Bool
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool
}

/// The ONLY file in this project allowed to touch the private MediaRemote
/// framework. Everything downstream consumes plain value types, so if Apple
/// ever withdraws these symbols only this file changes — and the module
/// disables itself rather than taking the shell down with it.
final class MediaRemoteBridge: MediaRemoteProviding {
    private typealias GetInfoFn = @convention(c) (DispatchQueue, @escaping ([String: Any]) -> Void) -> Void
    private typealias GetIsPlayingFn = @convention(c) (DispatchQueue, @escaping (Bool) -> Void) -> Void
    private typealias SendCommandFn = @convention(c) (UInt32, [String: Any]?) -> Bool

    private let handle: UnsafeMutableRawPointer
    private let getInfo: GetInfoFn
    private let getIsPlaying: GetIsPlayingFn
    private let sendCommand: SendCommandFn
    private let queue = DispatchQueue(label: "com.notchcat.mediaremote")

    init?() {
        let path = "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote"
        guard let handle = dlopen(path, RTLD_NOW),
              let info = dlsym(handle, "MRMediaRemoteGetNowPlayingInfo"),
              let playing = dlsym(handle, "MRMediaRemoteGetNowPlayingApplicationIsPlaying"),
              let send = dlsym(handle, "MRMediaRemoteSendCommand")
        else { return nil }

        self.handle = handle
        self.getInfo = unsafeBitCast(info, to: GetInfoFn.self)
        self.getIsPlaying = unsafeBitCast(playing, to: GetIsPlayingFn.self)
        self.sendCommand = unsafeBitCast(send, to: SendCommandFn.self)
    }

    func nowPlayingInfo() async -> [String: Any]? {
        await withCheckedContinuation { continuation in
            var resumed = false
            let lock = NSLock()
            getInfo(queue) { info in
                lock.lock(); defer { lock.unlock() }
                guard !resumed else { return }
                resumed = true
                continuation.resume(returning: info)
            }
        }
    }

    func isPlaying() async -> Bool {
        await withCheckedContinuation { continuation in
            var resumed = false
            let lock = NSLock()
            getIsPlaying(queue) { playing in
                lock.lock(); defer { lock.unlock() }
                guard !resumed else { return }
                resumed = true
                continuation.resume(returning: playing)
            }
        }
    }

    @discardableResult
    func send(_ command: MediaRemoteCommand) -> Bool {
        sendCommand(command.rawValue, nil)
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter MediaRemoteBridgeTests 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5：實測播放控制是否真的生效**

**先告知使用者這會短暫中斷他們的音樂。** 請使用者播放音樂，然後建立並執行 `/tmp/control_probe.swift`：

```swift
import Foundation
let h = dlopen("/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote", RTLD_NOW)!
typealias Send = @convention(c) (UInt32, [String: Any]?) -> Bool
typealias IsPlaying = @convention(c) (DispatchQueue, @escaping (Bool) -> Void) -> Void
let send = unsafeBitCast(dlsym(h, "MRMediaRemoteSendCommand")!, to: Send.self)
let getPlaying = unsafeBitCast(dlsym(h, "MRMediaRemoteGetNowPlayingApplicationIsPlaying")!, to: IsPlaying.self)
let q = DispatchQueue(label: "p")

func playing() -> Bool {
    let s = DispatchSemaphore(value: 0); var r = false
    getPlaying(q) { r = $0; s.signal() }; _ = s.wait(timeout: .now() + 5); return r
}

let before = playing()
print("送出前正在播放：\(before)")
let accepted = send(2, nil)          // togglePlayPause
print("SendCommand 回傳：\(accepted)")
Thread.sleep(forTimeInterval: 1.5)
let after = playing()
print("送出後正在播放：\(after)")
print(before != after ? "✅ 播放控制可行" : "❌ 指令沒有生效，Task 10 要跳過")
_ = send(2, nil)                      // 復原使用者的播放狀態
```

Run: `swift /tmp/control_probe.swift`
Expected: 印出「✅ 播放控制可行」。**若印出 ❌，停止並回報使用者**，依本任務開頭的指示調整 Task 10 與 Task 11。

- [ ] **Step 6：把結論記進 spec**

把 Step 5 的結果寫入 spec 第 8 節，把「未驗證」的警告換成實測結論（可行或不可行、日期）。

- [ ] **Step 7：提交**

```bash
git add Sources/NotchCatKit/Modules/Music Tests/NotchCatKitTests/Music docs/superpowers/specs
git commit -m "Add MediaRemote bridge and verify playback control works"
```

---

## Task 3：NowPlayingSnapshot 與 PlaybackClock

把 MediaRemote 的字典正規化成純值型別，並實作本地推算播放位置。兩者都是純邏輯，不需要 MediaRemote 也不需要網路，是全計畫測試覆蓋最紮實的一塊。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingSnapshot.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/PlaybackClock.swift`
- Create: `Tests/NotchCatKitTests/Music/NowPlayingSnapshotTests.swift`
- Create: `Tests/NotchCatKitTests/Music/PlaybackClockTests.swift`

**Interfaces:**
- Consumes: 無（純值型別）
- Produces:
  - `struct NowPlayingSnapshot: Equatable { let title: String; let artist: String; let album: String?; let duration: TimeInterval; let elapsed: TimeInterval; let timestamp: Date; let rate: Double; let artworkIdentifier: String?; let artworkData: Data? }`
  - `init?(mediaRemoteInfo: [String: Any])` — 缺歌名或演出者時回 nil
  - `struct PlaybackClock { let elapsed: TimeInterval; let timestamp: Date; let rate: Double; let duration: TimeInterval; func position(at now: Date) -> TimeInterval }`
  - `init(snapshot: NowPlayingSnapshot)`

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/NowPlayingSnapshotTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class NowPlayingSnapshotTests: XCTestCase {
    private func info(_ overrides: [String: Any] = [:]) -> [String: Any] {
        var base: [String: Any] = [
            "kMRMediaRemoteNowPlayingInfoTitle": "Song",
            "kMRMediaRemoteNowPlayingInfoArtist": "Artist",
            "kMRMediaRemoteNowPlayingInfoAlbum": "Album",
            "kMRMediaRemoteNowPlayingInfoDuration": 205.5,
            "kMRMediaRemoteNowPlayingInfoElapsedTime": 10.0,
            "kMRMediaRemoteNowPlayingInfoTimestamp": Date(timeIntervalSince1970: 1000),
            "kMRMediaRemoteNowPlayingInfoPlaybackRate": 1.0,
        ]
        for (k, v) in overrides { base[k] = v }
        return base
    }

    func testParsesCompleteDictionary() throws {
        let snapshot = try XCTUnwrap(NowPlayingSnapshot(mediaRemoteInfo: info()))
        XCTAssertEqual(snapshot.title, "Song")
        XCTAssertEqual(snapshot.artist, "Artist")
        XCTAssertEqual(snapshot.album, "Album")
        XCTAssertEqual(snapshot.duration, 205.5, accuracy: 0.001)
        XCTAssertEqual(snapshot.rate, 1.0, accuracy: 0.001)
    }

    // Review Focus 1: 缺欄位不得崩潰
    func testMissingTitleYieldsNil() {
        var partial = info()
        partial.removeValue(forKey: "kMRMediaRemoteNowPlayingInfoTitle")
        XCTAssertNil(NowPlayingSnapshot(mediaRemoteInfo: partial))
    }

    func testMissingArtistYieldsNil() {
        var partial = info()
        partial.removeValue(forKey: "kMRMediaRemoteNowPlayingInfoArtist")
        XCTAssertNil(NowPlayingSnapshot(mediaRemoteInfo: partial))
    }

    func testEmptyDictionaryYieldsNil() {
        XCTAssertNil(NowPlayingSnapshot(mediaRemoteInfo: [:]))
    }

    /// Album and artwork are genuinely optional — the probe saw payloads with
    /// artwork metadata but no bytes, and that must still parse.
    func testAbsentOptionalFieldsStillParse() throws {
        var partial = info()
        partial.removeValue(forKey: "kMRMediaRemoteNowPlayingInfoAlbum")
        let snapshot = try XCTUnwrap(NowPlayingSnapshot(mediaRemoteInfo: partial))
        XCTAssertNil(snapshot.album)
        XCTAssertNil(snapshot.artworkData)
    }

    func testMissingTimestampFallsBackToDistantPast() throws {
        var partial = info()
        partial.removeValue(forKey: "kMRMediaRemoteNowPlayingInfoTimestamp")
        let snapshot = try XCTUnwrap(NowPlayingSnapshot(mediaRemoteInfo: partial))
        XCTAssertEqual(snapshot.elapsed, 10.0, accuracy: 0.001)
    }
}
```

建立 `Tests/NotchCatKitTests/Music/PlaybackClockTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class PlaybackClockTests: XCTestCase {
    private let base = Date(timeIntervalSince1970: 1000)

    func testInterpolatesForward() {
        let clock = PlaybackClock(elapsed: 10, timestamp: base, rate: 1, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(5)), 15, accuracy: 0.001)
    }

    func testPausedClockDoesNotAdvance() {
        let clock = PlaybackClock(elapsed: 10, timestamp: base, rate: 0, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(60)), 10, accuracy: 0.001)
    }

    // Review Focus 2: 倍速播放
    func testDoubleRateAdvancesTwiceAsFast() {
        let clock = PlaybackClock(elapsed: 10, timestamp: base, rate: 2, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(5)), 20, accuracy: 0.001)
    }

    func testHalfRateAdvancesHalfAsFast() {
        let clock = PlaybackClock(elapsed: 10, timestamp: base, rate: 0.5, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(10)), 15, accuracy: 0.001)
    }

    func testPositionIsClampedToDuration() {
        let clock = PlaybackClock(elapsed: 190, timestamp: base, rate: 1, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(60)), 200, accuracy: 0.001)
    }

    /// Guards against a system clock adjustment yanking the computed position
    /// backwards past the start of the track.
    func testPositionNeverGoesNegative() {
        let clock = PlaybackClock(elapsed: 5, timestamp: base, rate: 1, duration: 200)
        XCTAssertEqual(clock.position(at: base.addingTimeInterval(-60)), 0, accuracy: 0.001)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter "NowPlayingSnapshotTests|PlaybackClockTests" 2>&1 | tail -20`
Expected: FAIL，`cannot find 'NowPlayingSnapshot' in scope`

- [ ] **Step 3：實作兩個型別**

建立 `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingSnapshot.swift`：

```swift
import Foundation

/// One normalized reading of the system's now-playing state.
///
/// Title and artist are required: without them there is nothing to show and
/// nothing to look lyrics up with, so a payload missing either is treated as
/// "no track" rather than a half-populated one. Everything else is optional
/// because the probe showed the system genuinely omits fields — artwork bytes
/// were absent from two of three consecutive reads.
struct NowPlayingSnapshot: Equatable {
    let title: String
    let artist: String
    let album: String?
    let duration: TimeInterval
    let elapsed: TimeInterval
    let timestamp: Date
    let rate: Double
    let artworkIdentifier: String?
    let artworkData: Data?

    init?(mediaRemoteInfo info: [String: Any]) {
        guard let title = info["kMRMediaRemoteNowPlayingInfoTitle"] as? String, !title.isEmpty,
              let artist = info["kMRMediaRemoteNowPlayingInfoArtist"] as? String, !artist.isEmpty
        else { return nil }

        self.title = title
        self.artist = artist
        self.album = info["kMRMediaRemoteNowPlayingInfoAlbum"] as? String
        self.duration = info["kMRMediaRemoteNowPlayingInfoDuration"] as? Double ?? 0
        self.elapsed = info["kMRMediaRemoteNowPlayingInfoElapsedTime"] as? Double ?? 0
        self.timestamp = info["kMRMediaRemoteNowPlayingInfoTimestamp"] as? Date ?? .distantPast
        self.rate = info["kMRMediaRemoteNowPlayingInfoPlaybackRate"] as? Double ?? 0
        self.artworkIdentifier = info["kMRMediaRemoteNowPlayingInfoArtworkIdentifier"] as? String
        self.artworkData = info["kMRMediaRemoteNowPlayingInfoArtworkData"] as? Data
    }

    /// Identity for "is this the same track", deliberately ignoring elapsed
    /// time, rate and artwork so a progress update is not mistaken for a
    /// track change.
    var trackKey: String { "\(title)\u{1}\(artist)\u{1}\(album ?? "")" }
}
```

建立 `Sources/NotchCatKit/Modules/Music/NowPlaying/PlaybackClock.swift`：

```swift
import Foundation

/// Computes the current playback position locally instead of polling for it.
///
/// MediaRemote hands back an elapsed value, the instant that value was taken,
/// and a rate — which is enough to extrapolate. That keeps the progress bar
/// smooth without any further calls into the private framework.
struct PlaybackClock: Equatable {
    let elapsed: TimeInterval
    let timestamp: Date
    let rate: Double
    let duration: TimeInterval

    init(elapsed: TimeInterval, timestamp: Date, rate: Double, duration: TimeInterval) {
        self.elapsed = elapsed
        self.timestamp = timestamp
        self.rate = rate
        self.duration = duration
    }

    init(snapshot: NowPlayingSnapshot) {
        self.init(elapsed: snapshot.elapsed,
                  timestamp: snapshot.timestamp,
                  rate: snapshot.rate,
                  duration: snapshot.duration)
    }

    /// Rate is honoured rather than assumed to be 1 — podcasts and audiobooks
    /// routinely play at 1.5x or 2x, and assuming 1 would desync both the
    /// progress bar and the lyric scroll.
    func position(at now: Date) -> TimeInterval {
        let raw = elapsed + now.timeIntervalSince(timestamp) * rate
        let upperBound = duration > 0 ? duration : raw
        return min(max(raw, 0), upperBound)
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter "NowPlayingSnapshotTests|PlaybackClockTests" 2>&1 | tail -20`
Expected: PASS，12 項全過

- [ ] **Step 5：提交**

```bash
git add Sources/NotchCatKit/Modules/Music Tests/NotchCatKitTests/Music
git commit -m "Add NowPlayingSnapshot parsing and PlaybackClock interpolation"
```

---

## Task 4：TrackIdentity 與查詢正規化

歌詞查詢用的曲目識別，以及 spec 第 6.3 節那套正規化規則。實測顯示精確查詢會因為 `(feat. …)` 後綴而落空，所以正規化不是加分項而是必要機制。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/TrackIdentity.swift`
- Create: `Tests/NotchCatKitTests/Music/TrackIdentityTests.swift`

**Interfaces:**
- Consumes: Task 3 的 `NowPlayingSnapshot`
- Produces:
  - `struct TrackIdentity: Hashable { let title: String; let artist: String; let album: String?; let duration: TimeInterval }`
  - `init(snapshot: NowPlayingSnapshot)`
  - `var normalized: TrackIdentity` — 去除括號後綴、取主要演出者
  - `var cacheKey: String`
  - `func durationMatches(_ other: TimeInterval) -> Bool` — ±5 秒容差
  - `static func loosely(_ a: String, matches b: String) -> Bool` — 簡繁與大小寫不敏感

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/TrackIdentityTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class TrackIdentityTests: XCTestCase {
    private func make(title: String, artist: String = "A", duration: TimeInterval = 200) -> TrackIdentity {
        TrackIdentity(title: title, artist: artist, album: nil, duration: duration)
    }

    func testStripsFeatSuffix() {
        XCTAssertEqual(make(title: "善良是 (feat. CHS)").normalized.title, "善良是")
        XCTAssertEqual(make(title: "Song (ft. Someone)").normalized.title, "Song")
        XCTAssertEqual(make(title: "Song [Remastered 2011]").normalized.title, "Song")
    }

    func testLeavesCleanTitleAlone() {
        XCTAssertEqual(make(title: "後來有很多人去過那裡").normalized.title, "後來有很多人去過那裡")
    }

    func testTakesPrimaryArtist() {
        XCTAssertEqual(make(title: "T", artist: "someshiit 山姆/CHS").normalized.artist, "someshiit 山姆")
        XCTAssertEqual(make(title: "T", artist: "A & B").normalized.artist, "A")
        XCTAssertEqual(make(title: "T", artist: "A, B").normalized.artist, "A")
    }

    func testDurationToleranceIsFiveSeconds() {
        let track = make(title: "T", duration: 200)
        XCTAssertTrue(track.durationMatches(200))
        XCTAssertTrue(track.durationMatches(204.9))
        XCTAssertTrue(track.durationMatches(195.1))
        XCTAssertFalse(track.durationMatches(210))
    }

    /// A provider reporting no duration at all must not be rejected outright.
    func testUnknownDurationAlwaysMatches() {
        XCTAssertTrue(make(title: "T", duration: 200).durationMatches(0))
    }

    /// The probe matched a simplified-Chinese title against a traditional one,
    /// so exact string equality would have rejected a correct result.
    func testLooseMatchIgnoresSimplifiedTraditionalDifference() {
        XCTAssertTrue(TrackIdentity.loosely("後來有很多人去過那裡", matches: "后来有很多人去过那里"))
        XCTAssertTrue(TrackIdentity.loosely("Song", matches: "song"))
        XCTAssertFalse(TrackIdentity.loosely("Song", matches: "Different"))
    }

    func testCacheKeyIgnoresDuration() {
        XCTAssertEqual(make(title: "T", duration: 200).cacheKey,
                       make(title: "T", duration: 260).cacheKey)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter TrackIdentityTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'TrackIdentity' in scope`

- [ ] **Step 3：實作**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/TrackIdentity.swift`：

```swift
import Foundation

/// What a lyrics provider is asked to look up.
///
/// Kept separate from `NowPlayingSnapshot` because lookup only cares about
/// identity, not playback state — and because the normalization rules below
/// exist purely to serve lookup.
struct TrackIdentity: Hashable {
    let title: String
    let artist: String
    let album: String?
    let duration: TimeInterval

    init(title: String, artist: String, album: String?, duration: TimeInterval) {
        self.title = title
        self.artist = artist
        self.album = album
        self.duration = duration
    }

    init(snapshot: NowPlayingSnapshot) {
        self.init(title: snapshot.title,
                  artist: snapshot.artist,
                  album: snapshot.album,
                  duration: snapshot.duration)
    }

    private static let suffixPattern = try! NSRegularExpression(
        pattern: #"\s*[\(\[](feat\.?|ft\.?|remaster(ed)?|live|explicit)[^\)\]]*[\)\]]"#,
        options: .caseInsensitive
    )

    private static let artistSeparators = CharacterSet(charactersIn: "/&,;")

    /// Falls back to a looser form of the query. A probe against LRCLIB missed
    /// on an exact lookup purely because the system reported the title without
    /// the `(feat. …)` suffix the database carried.
    var normalized: TrackIdentity {
        let range = NSRange(title.startIndex..., in: title)
        let strippedTitle = Self.suffixPattern
            .stringByReplacingMatches(in: title, range: range, withTemplate: "")
            .trimmingCharacters(in: .whitespaces)

        let primaryArtist = artist
            .components(separatedBy: Self.artistSeparators)
            .first?
            .trimmingCharacters(in: .whitespaces) ?? artist

        return TrackIdentity(
            title: strippedTitle.isEmpty ? title : strippedTitle,
            artist: primaryArtist.isEmpty ? artist : primaryArtist,
            album: album,
            duration: duration
        )
    }

    var cacheKey: String {
        let n = normalized
        return "\(n.title.lowercased())\u{1}\(n.artist.lowercased())"
    }

    /// Tolerance rather than equality: a `(feat. …)` edit of a track is often
    /// a few seconds off the original, and rejecting on that would discard
    /// correct matches. A provider reporting 0 means "unknown", not "zero".
    func durationMatches(_ other: TimeInterval) -> Bool {
        guard duration > 0, other > 0 else { return true }
        return abs(duration - other) <= 5
    }

    /// Never compare provider titles with `==`. A probe matched a
    /// simplified-Chinese title against the traditional one we sent, and exact
    /// equality would have thrown that correct result away.
    static func loosely(_ a: String, matches b: String) -> Bool {
        let fold: (String) -> String = { raw in
            let mutable = NSMutableString(string: raw) as CFMutableString
            CFStringTransform(mutable, nil, "Hans-Hant" as CFString, false)
            return (mutable as String)
                .folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive],
                         locale: nil)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return fold(a) == fold(b)
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter TrackIdentityTests 2>&1 | tail -20`
Expected: PASS。若簡繁測試失敗，確認 `CFStringTransform` 的轉換方向（`Hans-Hant` 是簡轉繁；兩邊都轉成同一種即可）。

- [ ] **Step 5：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/Lyrics Tests/NotchCatKitTests/Music
git commit -m "Add TrackIdentity with lookup normalization and loose matching"
```

---

## Task 5：Lyrics 型別與 LRC 解析

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Lyrics.swift`
- Create: `Tests/NotchCatKitTests/Music/LyricsTests.swift`

**Interfaces:**
- Consumes: 無
- Produces:
  - `struct LyricLine: Equatable { let time: TimeInterval; let text: String }`
  - `enum Lyrics: Equatable { case synced([LyricLine]); case plain(String); var isSynced: Bool }`
  - `enum LRCParser { static func parse(_ raw: String) -> Lyrics? }`
  - `extension Lyrics { func lineIndex(at position: TimeInterval) -> Int? }`

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/LyricsTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

final class LyricsTests: XCTestCase {
    func testParsesTimestampedLines() throws {
        let raw = "[00:12.34]first\n[01:05.00]second"
        guard case .synced(let lines)? = LRCParser.parse(raw) else {
            return XCTFail("expected synced lyrics")
        }
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines[0].time, 12.34, accuracy: 0.001)
        XCTAssertEqual(lines[0].text, "first")
        XCTAssertEqual(lines[1].time, 65.0, accuracy: 0.001)
    }

    // Review Focus 3: 畸形 LRC
    func testUnsortedTimestampsAreSorted() throws {
        guard case .synced(let lines)? = LRCParser.parse("[01:00.00]b\n[00:10.00]a") else {
            return XCTFail("expected synced lyrics")
        }
        XCTAssertEqual(lines.map(\.text), ["a", "b"])
    }

    func testTextWithoutAnyTimestampIsPlain() throws {
        guard case .plain(let text)? = LRCParser.parse("just words\nmore words") else {
            return XCTFail("expected plain lyrics")
        }
        XCTAssertEqual(text, "just words\nmore words")
    }

    func testMetadataOnlyTagsDoNotCountAsSynced() throws {
        guard case .plain? = LRCParser.parse("[ar:Artist]\n[ti:Title]\nwords") else {
            return XCTFail("metadata tags are not timestamps")
        }
    }

    func testEmptyInputYieldsNil() {
        XCTAssertNil(LRCParser.parse(""))
        XCTAssertNil(LRCParser.parse("   \n  "))
    }

    func testBlankLyricLinesAreDropped() throws {
        guard case .synced(let lines)? = LRCParser.parse("[00:01.00]\n[00:02.00]real") else {
            return XCTFail("expected synced lyrics")
        }
        XCTAssertEqual(lines.map(\.text), ["real"])
    }

    func testRepeatedTimestampsOnOneLine() throws {
        guard case .synced(let lines)? = LRCParser.parse("[00:10.00][00:20.00]chorus") else {
            return XCTFail("expected synced lyrics")
        }
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines.map(\.text), ["chorus", "chorus"])
    }

    func testLineIndexAtPosition() {
        let lyrics = Lyrics.synced([
            LyricLine(time: 0, text: "a"),
            LyricLine(time: 10, text: "b"),
            LyricLine(time: 20, text: "c"),
        ])
        XCTAssertEqual(lyrics.lineIndex(at: 0), 0)
        XCTAssertEqual(lyrics.lineIndex(at: 9.9), 0)
        XCTAssertEqual(lyrics.lineIndex(at: 10), 1)
        XCTAssertEqual(lyrics.lineIndex(at: 999), 2)
    }

    func testLineIndexBeforeFirstLine() {
        let lyrics = Lyrics.synced([LyricLine(time: 5, text: "a")])
        XCTAssertNil(lyrics.lineIndex(at: 1))
    }

    func testLineIndexOnPlainLyricsIsNil() {
        XCTAssertNil(Lyrics.plain("words").lineIndex(at: 10))
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter LyricsTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'LRCParser' in scope`

- [ ] **Step 3：實作**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/Lyrics.swift`：

```swift
import Foundation

struct LyricLine: Equatable {
    let time: TimeInterval
    let text: String
}

enum Lyrics: Equatable {
    case synced([LyricLine])
    case plain(String)

    var isSynced: Bool {
        if case .synced = self { return true }
        return false
    }

    /// Index of the line that should be showing at `position`, or nil when the
    /// track has not reached the first line yet.
    func lineIndex(at position: TimeInterval) -> Int? {
        guard case .synced(let lines) = self, !lines.isEmpty else { return nil }
        guard position >= lines[0].time else { return nil }
        var result = 0
        for (index, line) in lines.enumerated() where line.time <= position {
            result = index
        }
        return result
    }
}

/// Parses LRC text from whichever provider supplied it.
///
/// Community lyric databases vary wildly in quality, so this is deliberately
/// forgiving: it sorts what it finds, drops empty lines, and refuses to call
/// something synced unless it actually carries timestamps.
enum LRCParser {
    private static let timeTag = try! NSRegularExpression(
        pattern: #"\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]"#
    )

    static func parse(_ raw: String) -> Lyrics? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        var lines: [LyricLine] = []
        for rawLine in trimmed.components(separatedBy: .newlines) {
            let range = NSRange(rawLine.startIndex..., in: rawLine)
            let matches = timeTag.matches(in: rawLine, range: range)
            guard !matches.isEmpty, let last = matches.last else { continue }

            let textStart = rawLine.index(rawLine.startIndex, offsetBy: last.range.upperBound)
            let text = String(rawLine[textStart...]).trimmingCharacters(in: .whitespaces)
            guard !text.isEmpty else { continue }

            for match in matches {
                guard let time = seconds(from: match, in: rawLine) else { continue }
                lines.append(LyricLine(time: time, text: text))
            }
        }

        if lines.isEmpty {
            // No usable timestamps: metadata-only tags like [ar:…] land here
            // too, which is correct — they are not a time axis.
            return .plain(trimmed)
        }
        return .synced(lines.sorted { $0.time < $1.time })
    }

    private static func seconds(from match: NSTextCheckingResult, in line: String) -> TimeInterval? {
        func part(_ index: Int) -> String? {
            guard let r = Range(match.range(at: index), in: line) else { return nil }
            return String(line[r])
        }
        guard let minutes = part(1).flatMap(Double.init),
              let secs = part(2).flatMap(Double.init) else { return nil }
        var fraction: Double = 0
        if let raw = part(3), let value = Double(raw) {
            fraction = value / pow(10, Double(raw.count))
        }
        return minutes * 60 + secs + fraction
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter LyricsTests 2>&1 | tail -20`
Expected: PASS，10 項全過

- [ ] **Step 5：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/Lyrics Tests/NotchCatKitTests/Music
git commit -m "Add Lyrics types and a forgiving LRC parser"
```

---

## Task 6：LyricsProvider 介面與 LyricsRepository 兩段式解析

**本任務是整個歌詞設計的核心規則所在**：同步歌詞的優先級高於來源順位，且每個來源只查一次。實測證據是同一首歌在順位較前的來源只有純文字、順位較後的來源卻有 82 行同步版本。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsProvider.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsRepository.swift`
- Create: `Tests/NotchCatKitTests/Music/LyricsRepositoryTests.swift`

**Interfaces:**
- Consumes: Task 4 的 `TrackIdentity`、Task 5 的 `Lyrics`
- Produces:
  - `protocol LyricsProvider: Sendable { var id: String { get }; func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? }`
  - `actor LyricsRepository`，`init(providers: [LyricsProvider], negativeTTL: TimeInterval = 86_400, now: @escaping () -> Date = Date.init)`
  - `func lyrics(for track: TrackIdentity) async -> Lyrics?`

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/LyricsRepositoryTests.swift`：

```swift
import XCTest
@testable import NotchCatKit

private final class FakeProvider: LyricsProvider, @unchecked Sendable {
    let id: String
    private let result: Lyrics?
    private let error: Error?
    private let delay: TimeInterval
    private(set) var callCount = 0

    init(id: String, result: Lyrics? = nil, error: Error? = nil, delay: TimeInterval = 0) {
        self.id = id
        self.result = result
        self.error = error
        self.delay = delay
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        callCount += 1
        if delay > 0 {
            try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
        if let error { throw error }
        return result
    }
}

private struct ProviderFailure: Error {}

final class LyricsRepositoryTests: XCTestCase {
    private let track = TrackIdentity(title: "T", artist: "A", album: nil, duration: 200)
    private let synced = Lyrics.synced([LyricLine(time: 0, text: "line")])

    /// The rule the whole lyrics design rests on. A probe found the
    /// higher-priority source returning plain text where the lower-priority
    /// one had a fully timed version — taking the first hit would lose it.
    func testSyncedResultBeatsHigherPriorityPlain() async {
        let first = FakeProvider(id: "first", result: .plain("words"))
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result?.isSynced, true)
    }

    func testEachProviderIsQueriedAtMostOnce() async {
        let first = FakeProvider(id: "first", result: .plain("words"))
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        _ = await repo.lyrics(for: track)

        XCTAssertEqual(first.callCount, 1)
        XCTAssertEqual(second.callCount, 1)
    }

    func testStopsAtFirstSyncedResult() async {
        let first = FakeProvider(id: "first", result: synced)
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        _ = await repo.lyrics(for: track)

        XCTAssertEqual(second.callCount, 0, "找到同步歌詞後不應再查後面的來源")
    }

    func testFallsBackToPlainWhenNoSyncedAnywhere() async {
        let first = FakeProvider(id: "first", result: .plain("first words"))
        let second = FakeProvider(id: "second", result: .plain("second words"))
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result, .plain("first words"), "純文字應保留順位較前者")
    }

    func testFailingProviderDoesNotBlockLaterOnes() async {
        let first = FakeProvider(id: "first", error: ProviderFailure())
        let second = FakeProvider(id: "second", result: synced)
        let repo = LyricsRepository(providers: [first, second])

        let result = await repo.lyrics(for: track)

        XCTAssertEqual(result?.isSynced, true)
    }

    func testReturnsNilWhenNobodyHasAnything() async {
        let repo = LyricsRepository(providers: [FakeProvider(id: "only", result: nil)])
        let result = await repo.lyrics(for: track)
        XCTAssertNil(result)
    }

    func testPositiveResultIsCached() async {
        let provider = FakeProvider(id: "p", result: synced)
        let repo = LyricsRepository(providers: [provider])

        _ = await repo.lyrics(for: track)
        _ = await repo.lyrics(for: track)

        XCTAssertEqual(provider.callCount, 1)
    }

    func testNegativeResultIsCachedWithinTTL() async {
        let provider = FakeProvider(id: "p", result: nil)
        let repo = LyricsRepository(providers: [provider], negativeTTL: 86_400)

        _ = await repo.lyrics(for: track)
        _ = await repo.lyrics(for: track)

        XCTAssertEqual(provider.callCount, 1, "查無結果也該快取，避免每次切回來都重打")
    }

    func testNegativeCacheExpires() async {
        let provider = FakeProvider(id: "p", result: nil)
        var now = Date(timeIntervalSince1970: 0)
        let repo = LyricsRepository(providers: [provider], negativeTTL: 60, now: { now })

        _ = await repo.lyrics(for: track)
        now = now.addingTimeInterval(61)
        _ = await repo.lyrics(for: track)

        XCTAssertEqual(provider.callCount, 2)
    }

    /// Cache keys come from the normalized identity, so the same track at a
    /// slightly different reported duration must not re-query.
    func testCacheIsKeyedOnNormalizedIdentity() async {
        let provider = FakeProvider(id: "p", result: synced)
        let repo = LyricsRepository(providers: [provider])

        _ = await repo.lyrics(for: track)
        _ = await repo.lyrics(for: TrackIdentity(title: "T (feat. X)", artist: "A", album: nil, duration: 203))

        XCTAssertEqual(provider.callCount, 1)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter LyricsRepositoryTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'LyricsRepository' in scope`

- [ ] **Step 3：實作介面**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsProvider.swift`：

```swift
import Foundation

/// One source of lyrics. The module never names a concrete provider — adding,
/// removing or reordering sources means editing the array handed to
/// `LyricsRepository` and nothing else.
protocol LyricsProvider: Sendable {
    var id: String { get }
    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics?
}
```

- [ ] **Step 4：實作 repository**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsRepository.swift`：

```swift
import Foundation

/// Resolves lyrics across providers, and caches what it learns.
///
/// The ordering rule is the important part: a synced result outranks provider
/// priority. A probe found LRCLIB returning plain text for a track where a
/// lower-priority source had 82 timed lines — and timed lyrics are the entire
/// point of the module, so first-hit-wins would have been the wrong rule.
///
/// This is a single pass, not two. A plain result is kept aside as we go, so
/// falling back to it costs no extra network calls.
actor LyricsRepository {
    private enum CacheEntry {
        case found(Lyrics)
        case missing(at: Date)
    }

    private let providers: [LyricsProvider]
    private let negativeTTL: TimeInterval
    private let now: () -> Date
    private var cache: [String: CacheEntry] = [:]

    init(providers: [LyricsProvider],
         negativeTTL: TimeInterval = 86_400,
         now: @escaping () -> Date = Date.init) {
        self.providers = providers
        self.negativeTTL = negativeTTL
        self.now = now
    }

    func lyrics(for track: TrackIdentity) async -> Lyrics? {
        let key = track.cacheKey

        switch cache[key] {
        case .found(let lyrics):
            return lyrics
        case .missing(let at) where now().timeIntervalSince(at) < negativeTTL:
            return nil
        default:
            break
        }

        var fallback: Lyrics?

        for provider in providers {
            let result = try? await provider.fetchLyrics(for: track)
            guard let result else { continue }

            if result.isSynced {
                cache[key] = .found(result)
                return result
            }
            // Hold the highest-priority plain result in case nothing synced
            // turns up. Not overwritten, so priority still decides among equals.
            if fallback == nil { fallback = result }
        }

        if let fallback {
            cache[key] = .found(fallback)
            return fallback
        }

        cache[key] = .missing(at: now())
        return nil
    }
}
```

- [ ] **Step 5：執行測試確認通過**

Run: `swift test --filter LyricsRepositoryTests 2>&1 | tail -20`
Expected: PASS，10 項全過

- [ ] **Step 6：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/Lyrics Tests/NotchCatKitTests/Music
git commit -m "Add LyricsRepository with synced-beats-priority resolution"
```

---

## Task 7：LRCLIB 與網易雲兩個來源

兩個 provider 的結構幾乎一致（查詢、比對驗證、解析），放在同一個任務可以共用驗證邏輯並一次測完。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/LRCLIBProvider.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/NetEaseProvider.swift`
- Create: `Tests/NotchCatKitTests/Music/ProviderTests.swift`

**Interfaces:**
- Consumes: Task 4 `TrackIdentity`、Task 5 `Lyrics`/`LRCParser`、Task 6 `LyricsProvider`
- Produces:
  - `struct LRCLIBProvider: LyricsProvider`，`init(session: URLSession = .shared, timeout: TimeInterval = 5)`
  - `struct NetEaseProvider: LyricsProvider`，同樣簽章
  - `enum LyricsProviderError: Error { case badResponse, timedOut }`

- [ ] **Step 1：寫會失敗的測試**

建立 `Tests/NotchCatKitTests/Music/ProviderTests.swift`。網路測試不穩定，因此預設標記為手動執行，但**解析與驗證邏輯一律離線測試**：

```swift
import XCTest
@testable import NotchCatKit

final class ProviderTests: XCTestCase {
    private let track = TrackIdentity(title: "T", artist: "A", album: "Al", duration: 200)

    func testLRCLIBPrefersSyncedFieldOverPlain() throws {
        let json = """
        {"trackName":"T","artistName":"A","duration":200,
         "plainLyrics":"words","syncedLyrics":"[00:01.00]line"}
        """.data(using: .utf8)!
        let result = try XCTUnwrap(LRCLIBProvider.decode(json, for: track))
        XCTAssertTrue(result.isSynced)
    }

    func testLRCLIBFallsBackToPlainWhenSyncedAbsent() throws {
        let json = """
        {"trackName":"T","artistName":"A","duration":200,"plainLyrics":"words"}
        """.data(using: .utf8)!
        XCTAssertEqual(try XCTUnwrap(LRCLIBProvider.decode(json, for: track)), .plain("words"))
    }

    func testLRCLIBRejectsResultWithMismatchedDuration() throws {
        let json = """
        {"trackName":"T","artistName":"A","duration":400,"syncedLyrics":"[00:01.00]line"}
        """.data(using: .utf8)!
        XCTAssertNil(LRCLIBProvider.decode(json, for: track), "長度差太多應視為抓錯歌")
    }

    func testLRCLIBAcceptsResultWithinDurationTolerance() throws {
        let json = """
        {"trackName":"T","artistName":"A","duration":203,"syncedLyrics":"[00:01.00]line"}
        """.data(using: .utf8)!
        XCTAssertNotNil(LRCLIBProvider.decode(json, for: track))
    }

    func testNetEaseDecodesLrcField() throws {
        let json = """
        {"lrc":{"lyric":"[00:02.00]hello"}}
        """.data(using: .utf8)!
        let result = try XCTUnwrap(NetEaseProvider.decodeLyric(json))
        XCTAssertTrue(result.isSynced)
    }

    func testNetEaseEmptyLyricYieldsNil() throws {
        let json = #"{"lrc":{"lyric":""}}"#.data(using: .utf8)!
        XCTAssertNil(NetEaseProvider.decodeLyric(json))
    }

    func testNetEaseMissingLrcYieldsNil() throws {
        XCTAssertNil(NetEaseProvider.decodeLyric(#"{}"#.data(using: .utf8)!))
    }

    /// Run by hand against the live services; excluded from CI because the
    /// result depends on third-party availability and catalogue coverage.
    func testLiveLookupSmoke() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["NOTCHCAT_LIVE_TESTS"] == "1")
        let provider = LRCLIBProvider()
        let hit = TrackIdentity(title: "Bohemian Rhapsody", artist: "Queen", album: nil, duration: 0)
        _ = try await provider.fetchLyrics(for: hit)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter ProviderTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'LRCLIBProvider' in scope`

- [ ] **Step 3：實作 LRCLIB provider**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/LRCLIBProvider.swift`：

```swift
import Foundation

enum LyricsProviderError: Error {
    case badResponse
    case timedOut
}

/// Free, keyless, community-maintained LRC database.
///
/// Queried exactly first, then by search — a probe showed the exact lookup
/// missing purely because the system reported a title without the `(feat. …)`
/// suffix the database carried.
struct LRCLIBProvider: LyricsProvider {
    let id = "net.lrclib"
    private let session: URLSession
    private let timeout: TimeInterval

    init(session: URLSession = .shared, timeout: TimeInterval = 5) {
        self.session = session
        self.timeout = timeout
    }

    private struct Payload: Decodable {
        let trackName: String?
        let artistName: String?
        let duration: Double?
        let plainLyrics: String?
        let syncedLyrics: String?
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        if let exact = try? await exactLookup(track), let lyrics = Self.decode(exact, for: track) {
            return lyrics
        }
        guard let searched = try? await search(track) else { return nil }
        return Self.decodeSearch(searched, for: track)
    }

    private func request(_ url: URL) -> URLRequest {
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.setValue("NotchCat (https://github.com/Arxing/notchcat)", forHTTPHeaderField: "User-Agent")
        return request
    }

    private func exactLookup(_ track: TrackIdentity) async throws -> Data {
        let n = track.normalized
        var components = URLComponents(string: "https://lrclib.net/api/get")!
        components.queryItems = [
            .init(name: "track_name", value: n.title),
            .init(name: "artist_name", value: n.artist),
            .init(name: "album_name", value: n.album ?? ""),
            .init(name: "duration", value: String(Int(n.duration))),
        ]
        let (data, response) = try await session.data(for: request(components.url!))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw LyricsProviderError.badResponse }
        return data
    }

    private func search(_ track: TrackIdentity) async throws -> Data {
        let n = track.normalized
        var components = URLComponents(string: "https://lrclib.net/api/search")!
        components.queryItems = [.init(name: "q", value: "\(n.title) \(n.artist)")]
        let (data, response) = try await session.data(for: request(components.url!))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw LyricsProviderError.badResponse }
        return data
    }

    static func decode(_ data: Data, for track: TrackIdentity) -> Lyrics? {
        guard let payload = try? JSONDecoder().decode(Payload.self, from: data) else { return nil }
        return lyrics(from: payload, for: track)
    }

    static func decodeSearch(_ data: Data, for track: TrackIdentity) -> Lyrics? {
        guard let payloads = try? JSONDecoder().decode([Payload].self, from: data) else { return nil }
        for payload in payloads {
            if let lyrics = lyrics(from: payload, for: track) { return lyrics }
        }
        return nil
    }

    private static func lyrics(from payload: Payload, for track: TrackIdentity) -> Lyrics? {
        guard track.durationMatches(payload.duration ?? 0) else { return nil }
        if let synced = payload.syncedLyrics, let parsed = LRCParser.parse(synced), parsed.isSynced {
            return parsed
        }
        if let plain = payload.plainLyrics, !plain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return .plain(plain)
        }
        return nil
    }
}
```

- [ ] **Step 4：實作網易雲 provider**

建立 `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/NetEaseProvider.swift`：

```swift
import Foundation

/// Unofficial NetEase Cloud Music endpoints. Noticeably better coverage of
/// Chinese-language catalogue than LRCLIB in probing, at the cost of being
/// undocumented and liable to change — which is exactly what `LyricsProvider`
/// exists to contain.
struct NetEaseProvider: LyricsProvider {
    let id = "com.music163"
    private let session: URLSession
    private let timeout: TimeInterval

    init(session: URLSession = .shared, timeout: TimeInterval = 5) {
        self.session = session
        self.timeout = timeout
    }

    private struct SearchResponse: Decodable {
        struct Result: Decodable {
            struct Song: Decodable {
                struct Artist: Decodable { let name: String? }
                let id: Int
                let name: String?
                let duration: Int?
                let artists: [Artist]?
            }
            let songs: [Song]?
        }
        let result: Result?
    }

    private struct LyricResponse: Decodable {
        struct Lrc: Decodable { let lyric: String? }
        let lrc: Lrc?
    }

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        guard let songID = try? await findSongID(track) else { return nil }
        var components = URLComponents(string: "https://music.163.com/api/song/lyric")!
        components.queryItems = [
            .init(name: "id", value: String(songID)),
            .init(name: "lv", value: "1"),
            .init(name: "kv", value: "1"),
            .init(name: "tv", value: "-1"),
        ]
        let (data, _) = try await session.data(for: request(components.url!))
        return Self.decodeLyric(data)
    }

    private func request(_ url: URL) -> URLRequest {
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.setValue("https://music.163.com/", forHTTPHeaderField: "Referer")
        request.setValue("NotchCat (https://github.com/Arxing/notchcat)", forHTTPHeaderField: "User-Agent")
        return request
    }

    private func findSongID(_ track: TrackIdentity) async throws -> Int? {
        let n = track.normalized
        var components = URLComponents(string: "https://music.163.com/api/search/get/web")!
        components.queryItems = [
            .init(name: "s", value: "\(n.title) \(n.artist)"),
            .init(name: "type", value: "1"),
            .init(name: "limit", value: "5"),
            .init(name: "offset", value: "0"),
        ]
        let (data, _) = try await session.data(for: request(components.url!))
        let decoded = try JSONDecoder().decode(SearchResponse.self, from: data)
        guard let songs = decoded.result?.songs else { return nil }

        // Duration arrives in milliseconds here. Loose title matching matters:
        // this service answers simplified-Chinese titles to traditional queries.
        for song in songs {
            let seconds = Double(song.duration ?? 0) / 1000
            guard track.durationMatches(seconds) else { continue }
            if let name = song.name, TrackIdentity.loosely(name, matches: n.title) {
                return song.id
            }
        }
        return songs.first.map(\.id)
    }

    static func decodeLyric(_ data: Data) -> Lyrics? {
        guard let decoded = try? JSONDecoder().decode(LyricResponse.self, from: data),
              let raw = decoded.lrc?.lyric,
              !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { return nil }
        return LRCParser.parse(raw)
    }
}
```

- [ ] **Step 5：執行測試確認通過**

Run: `swift test --filter ProviderTests 2>&1 | tail -20`
Expected: PASS（live smoke 一項顯示 skipped）

- [ ] **Step 6：實際對外驗證一次**

Run: `NOTCHCAT_LIVE_TESTS=1 swift test --filter testLiveLookupSmoke 2>&1 | tail -10`
Expected: PASS。失敗代表外部服務不可用或介面變動，記錄下來但不阻擋後續任務。

- [ ] **Step 7：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/Lyrics Tests/NotchCatKitTests/Music
git commit -m "Add LRCLIB and NetEase lyrics providers"
```

---

## Task 8：ArtworkStore

以 `ArtworkIdentifier` 為鍵快取封面。探針顯示封面位元組三次只出現一次，所以「拿到就留著」是必要行為而非最佳化。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/ArtworkStore.swift`
- Create: `Tests/NotchCatKitTests/Music/ArtworkStoreTests.swift`

**Interfaces:**
- Consumes: 無
- Produces: `final class ArtworkStore`，`init(limit: Int = 8)`、`func store(_ data: Data, for identifier: String)`、`func artwork(for identifier: String?) -> Data?`

- [ ] **Step 1：寫會失敗的測試**

```swift
import XCTest
@testable import NotchCatKit

final class ArtworkStoreTests: XCTestCase {
    private let a = Data([1, 2, 3])
    private let b = Data([4, 5, 6])

    func testStoresAndRetrieves() {
        let store = ArtworkStore()
        store.store(a, for: "id-1")
        XCTAssertEqual(store.artwork(for: "id-1"), a)
    }

    func testUnknownIdentifierReturnsNil() {
        XCTAssertNil(ArtworkStore().artwork(for: "nope"))
    }

    func testNilIdentifierReturnsNil() {
        let store = ArtworkStore()
        store.store(a, for: "id-1")
        XCTAssertNil(store.artwork(for: nil))
    }

    /// The whole reason this cache exists: the system hands back artwork
    /// metadata far more often than the bytes, so once we have the bytes they
    /// must survive the reads that omit them.
    func testStoredArtworkSurvivesRepeatedLookups() {
        let store = ArtworkStore()
        store.store(a, for: "id-1")
        for _ in 0..<5 { XCTAssertEqual(store.artwork(for: "id-1"), a) }
    }

    func testEvictsOldestBeyondLimit() {
        let store = ArtworkStore(limit: 1)
        store.store(a, for: "id-1")
        store.store(b, for: "id-2")
        XCTAssertNil(store.artwork(for: "id-1"))
        XCTAssertEqual(store.artwork(for: "id-2"), b)
    }

    func testRestoringSameIdentifierDoesNotDuplicate() {
        let store = ArtworkStore(limit: 2)
        store.store(a, for: "id-1")
        store.store(b, for: "id-1")
        XCTAssertEqual(store.artwork(for: "id-1"), b)
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter ArtworkStoreTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'ArtworkStore' in scope`

- [ ] **Step 3：實作**

```swift
import Foundation

/// Keeps album art keyed by the system's artwork identifier.
///
/// Needed because the now-playing payload carries artwork metadata far more
/// often than the bytes themselves — probing saw bytes in only one of three
/// consecutive reads. Without this, the panel would flicker between showing a
/// cover and showing a placeholder.
final class ArtworkStore {
    private let limit: Int
    private var storage: [String: Data] = [:]
    private var order: [String] = []
    private let lock = NSLock()

    init(limit: Int = 8) {
        self.limit = max(1, limit)
    }

    func store(_ data: Data, for identifier: String) {
        lock.lock(); defer { lock.unlock() }
        if storage[identifier] == nil {
            order.append(identifier)
        }
        storage[identifier] = data
        while order.count > limit {
            let oldest = order.removeFirst()
            storage.removeValue(forKey: oldest)
        }
    }

    func artwork(for identifier: String?) -> Data? {
        guard let identifier else { return nil }
        lock.lock(); defer { lock.unlock() }
        return storage[identifier]
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter ArtworkStoreTests 2>&1 | tail -20`
Expected: PASS，6 項全過

- [ ] **Step 5：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/NowPlaying Tests/NotchCatKitTests/Music
git commit -m "Add ArtworkStore to survive payloads without artwork bytes"
```

---

## Task 9：NowPlayingMonitor

通知訂閱、兩檔保底查詢、封面補查。spec 第 5.1 節特別強調閒置檔**不可**因面板隱藏而關閉，否則漏接「開始播放」通知就永遠醒不過來。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingMonitor.swift`
- Create: `Tests/NotchCatKitTests/Music/NowPlayingMonitorTests.swift`

**Interfaces:**
- Consumes: Task 2 `MediaRemoteProviding`、Task 3 `NowPlayingSnapshot`、Task 8 `ArtworkStore`
- Produces:
  - `final class NowPlayingMonitor: ObservableObject`
  - `@Published private(set) var snapshot: NowPlayingSnapshot?`
  - `@Published private(set) var isPlaying: Bool`
  - `init(provider: MediaRemoteProviding, artworkStore: ArtworkStore = ArtworkStore())`
  - `func start()` / `func stop()` / `func refresh() async`
  - `var artwork: Data?`

- [ ] **Step 1：寫會失敗的測試**

```swift
import XCTest
@testable import NotchCatKit

private final class FakeMediaRemote: MediaRemoteProviding, @unchecked Sendable {
    var infoQueue: [[String: Any]?] = []
    var playing = true
    private(set) var sentCommands: [MediaRemoteCommand] = []
    private(set) var infoCallCount = 0

    func nowPlayingInfo() async -> [String: Any]? {
        infoCallCount += 1
        return infoQueue.isEmpty ? nil : infoQueue.removeFirst()
    }
    func isPlaying() async -> Bool { playing }
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool {
        sentCommands.append(command); return true
    }
}

final class NowPlayingMonitorTests: XCTestCase {
    private func info(artwork: Data? = nil, identifier: String? = "art-1") -> [String: Any] {
        var dict: [String: Any] = [
            "kMRMediaRemoteNowPlayingInfoTitle": "Song",
            "kMRMediaRemoteNowPlayingInfoArtist": "Artist",
            "kMRMediaRemoteNowPlayingInfoDuration": 200.0,
            "kMRMediaRemoteNowPlayingInfoElapsedTime": 1.0,
            "kMRMediaRemoteNowPlayingInfoTimestamp": Date(),
            "kMRMediaRemoteNowPlayingInfoPlaybackRate": 1.0,
        ]
        if let identifier { dict["kMRMediaRemoteNowPlayingInfoArtworkIdentifier"] = identifier }
        if let artwork { dict["kMRMediaRemoteNowPlayingInfoArtworkData"] = artwork }
        return dict
    }

    func testRefreshPublishesSnapshot() async {
        let fake = FakeMediaRemote()
        fake.infoQueue = [info()]
        let monitor = NowPlayingMonitor(provider: fake)

        await monitor.refresh()

        XCTAssertEqual(monitor.snapshot?.title, "Song")
        XCTAssertTrue(monitor.isPlaying)
    }

    func testEmptyPayloadClearsSnapshot() async {
        let fake = FakeMediaRemote()
        fake.infoQueue = [info(), [:]]
        let monitor = NowPlayingMonitor(provider: fake)

        await monitor.refresh()
        await monitor.refresh()

        XCTAssertNil(monitor.snapshot)
    }

    /// Artwork bytes arrive intermittently; once seen they must persist across
    /// later reads that omit them.
    func testArtworkPersistsAcrossPayloadsWithoutBytes() async {
        let bytes = Data([9, 9, 9])
        let fake = FakeMediaRemote()
        fake.infoQueue = [info(artwork: bytes), info(artwork: nil)]
        let monitor = NowPlayingMonitor(provider: fake)

        await monitor.refresh()
        await monitor.refresh()

        XCTAssertEqual(monitor.artwork, bytes)
    }

    func testArtworkChangesWithIdentifier() async {
        let first = Data([1]), second = Data([2])
        let fake = FakeMediaRemote()
        fake.infoQueue = [info(artwork: first, identifier: "a"), info(artwork: second, identifier: "b")]
        let monitor = NowPlayingMonitor(provider: fake)

        await monitor.refresh()
        await monitor.refresh()

        XCTAssertEqual(monitor.artwork, second)
    }

    func testPausedProviderReportsNotPlaying() async {
        let fake = FakeMediaRemote()
        fake.infoQueue = [info()]
        fake.playing = false
        let monitor = NowPlayingMonitor(provider: fake)

        await monitor.refresh()

        XCTAssertFalse(monitor.isPlaying)
    }

    func testSendForwardsCommand() {
        let fake = FakeMediaRemote()
        let monitor = NowPlayingMonitor(provider: fake)
        monitor.send(.next)
        XCTAssertEqual(fake.sentCommands, [.next])
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter NowPlayingMonitorTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'NowPlayingMonitor' in scope`

- [ ] **Step 3：實作**

```swift
import Combine
import Foundation

/// Keeps a live view of the system's now-playing state.
///
/// Updates are notification-driven with a two-tier safety poll behind them.
/// The idle tier must keep running even while the panel is hidden: it is the
/// only thing that notices playback starting if a notification is missed, and
/// without it the module could stay asleep forever.
final class NowPlayingMonitor: ObservableObject {
    @Published private(set) var snapshot: NowPlayingSnapshot?
    @Published private(set) var isPlaying: Bool = false

    private let provider: MediaRemoteProviding
    private let artworkStore: ArtworkStore
    private var timer: Timer?
    private var observer: NSObjectProtocol?
    private var artworkRetriesRemaining = 0

    private static let idleInterval: TimeInterval = 15
    private static let activeInterval: TimeInterval = 5

    init(provider: MediaRemoteProviding, artworkStore: ArtworkStore = ArtworkStore()) {
        self.provider = provider
        self.artworkStore = artworkStore
    }

    var artwork: Data? { artworkStore.artwork(for: snapshot?.artworkIdentifier) }

    func start() {
        observer = DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name("kMRMediaRemoteNowPlayingInfoDidChangeNotification"),
            object: nil, queue: .main
        ) { [weak self] _ in
            Task { await self?.refresh() }
        }
        scheduleTimer(interval: Self.idleInterval)
        Task { await refresh() }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        if let observer {
            DistributedNotificationCenter.default().removeObserver(observer)
        }
        observer = nil
    }

    @discardableResult
    func send(_ command: MediaRemoteCommand) -> Bool {
        provider.send(command)
    }

    @MainActor
    func refresh() async {
        let info = await provider.nowPlayingInfo()
        let playing = await provider.isPlaying()

        let parsed = info.flatMap(NowPlayingSnapshot.init(mediaRemoteInfo:))

        if let parsed, let identifier = parsed.artworkIdentifier {
            if let data = parsed.artworkData {
                artworkStore.store(data, for: identifier)
                artworkRetriesRemaining = 0
            } else if artworkStore.artwork(for: identifier) == nil, artworkRetriesRemaining == 0 {
                // Bytes were withheld on this read. They usually turn up on a
                // later one, so chase them rather than settling for a
                // placeholder for the whole track.
                artworkRetriesRemaining = 5
                scheduleArtworkRetry(delay: 0.5)
            }
        }

        snapshot = parsed
        isPlaying = playing && parsed != nil
        scheduleTimer(interval: isPlaying ? Self.activeInterval : Self.idleInterval)
    }

    private func scheduleArtworkRetry(delay: TimeInterval) {
        guard artworkRetriesRemaining > 0 else { return }
        artworkRetriesRemaining -= 1
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self else { return }
            Task { @MainActor in
                let info = await self.provider.nowPlayingInfo()
                guard let parsed = info.flatMap(NowPlayingSnapshot.init(mediaRemoteInfo:)),
                      let identifier = parsed.artworkIdentifier else { return }
                if let data = parsed.artworkData {
                    self.artworkStore.store(data, for: identifier)
                    self.artworkRetriesRemaining = 0
                    self.objectWillChange.send()
                } else {
                    self.scheduleArtworkRetry(delay: min(delay * 2, 8))
                }
            }
        }
    }

    private func scheduleTimer(interval: TimeInterval) {
        guard timer?.timeInterval != interval else { return }
        timer?.invalidate()
        let timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            Task { await self?.refresh() }
        }
        self.timer = timer
    }
}
```

- [ ] **Step 4：執行測試確認通過**

Run: `swift test --filter NowPlayingMonitorTests 2>&1 | tail -20`
Expected: PASS，6 項全過

- [ ] **Step 5：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/NowPlaying Tests/NotchCatKitTests/Music
git commit -m "Add NowPlayingMonitor with two-tier polling and artwork retry"
```

---

## Task 10：MusicModule

把所有元件組起來，實作 Phase 1 的 `NotchModule` protocol。**歌詞請求的競態防護在這裡。**

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/MusicModule.swift`
- Create: `Tests/NotchCatKitTests/Music/MusicModuleTests.swift`

**Interfaces:**
- Consumes: Task 2、3、4、5、6、9 的全部產出
- Produces:
  - `final class MusicModule: NotchModule, ObservableObject`
  - `init?(provider: MediaRemoteProviding? = MediaRemoteBridge(), repository: LyricsRepository? = nil)`
  - `let id = "com.notchcat.music"`、`let priority = 100`
  - `@Published private(set) var currentLyrics: Lyrics?`

- [ ] **Step 1：寫會失敗的測試**

```swift
import Combine
import XCTest
@testable import NotchCatKit

private final class StubProvider: LyricsProvider, @unchecked Sendable {
    let id = "stub"
    var byTitle: [String: Lyrics] = [:]
    var delayByTitle: [String: TimeInterval] = [:]

    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? {
        if let delay = delayByTitle[track.title], delay > 0 {
            try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
        return byTitle[track.title]
    }
}

final class MusicModuleTests: XCTestCase {
    func testModuleIdentityAndPriority() throws {
        let module = try XCTUnwrap(MusicModule(provider: FakeMediaRemoteForModule()))
        XCTAssertEqual(module.id, "com.notchcat.music")
        XCTAssertGreaterThan(module.priority, 0, "必須高於 ClockModule 的 0，才能搶到 Preview")
    }

    func testNilProviderYieldsNilModule() {
        XCTAssertNil(MusicModule(provider: nil))
    }

    /// Review Focus 4: a slow lookup for the previous track must not overwrite
    /// the lyrics of the track now playing.
    func testStaleLyricsRequestDoesNotOverwriteNewerTrack() async throws {
        let stub = StubProvider()
        stub.byTitle = [
            "Old": .synced([LyricLine(time: 0, text: "old")]),
            "New": .synced([LyricLine(time: 0, text: "new")]),
        ]
        stub.delayByTitle = ["Old": 0.4]

        let module = try XCTUnwrap(MusicModule(provider: FakeMediaRemoteForModule(),
                                               repository: LyricsRepository(providers: [stub])))

        await module.loadLyrics(for: TrackIdentity(title: "Old", artist: "A", album: nil, duration: 200))
        await module.loadLyrics(for: TrackIdentity(title: "New", artist: "A", album: nil, duration: 200))
        try await Task.sleep(nanoseconds: 700_000_000)

        guard case .synced(let lines)? = module.currentLyrics else {
            return XCTFail("expected synced lyrics")
        }
        XCTAssertEqual(lines.first?.text, "new", "較舊的請求回來時不得覆蓋較新的曲目")
    }
}

private final class FakeMediaRemoteForModule: MediaRemoteProviding, @unchecked Sendable {
    func nowPlayingInfo() async -> [String: Any]? { nil }
    func isPlaying() async -> Bool { false }
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool { true }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter MusicModuleTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'MusicModule' in scope`

- [ ] **Step 3：實作**

```swift
import Combine
import SwiftUI

/// The first real module: system now-playing plus timed lyrics.
///
/// Priority sits above ClockModule so that music owns Preview whenever it is
/// playing, while the clock remains the Expanded fallback when nothing is.
final class MusicModule: NotchModule, ObservableObject {
    let id = "com.notchcat.music"
    let priority = 100

    @Published private(set) var currentLyrics: Lyrics?

    let monitor: NowPlayingMonitor
    private let repository: LyricsRepository
    private var cancellables = Set<AnyCancellable>()
    private var lyricsTaskToken = UUID()
    private var lastTrackKey: String?

    init?(provider: MediaRemoteProviding? = MediaRemoteBridge(),
          repository: LyricsRepository? = nil) {
        guard let provider else { return nil }
        self.monitor = NowPlayingMonitor(provider: provider)
        self.repository = repository ?? LyricsRepository(providers: [
            LRCLIBProvider(),
            NetEaseProvider(),
        ])
    }

    var isActivePublisher: AnyPublisher<Bool, Never> {
        monitor.$isPlaying.eraseToAnyPublisher()
    }

    func onStart() {
        monitor.start()
        monitor.$snapshot
            .receive(on: DispatchQueue.main)
            .sink { [weak self] snapshot in
                guard let self else { return }
                guard let snapshot else {
                    lastTrackKey = nil
                    currentLyrics = nil
                    return
                }
                guard snapshot.trackKey != lastTrackKey else { return }
                lastTrackKey = snapshot.trackKey
                currentLyrics = nil
                Task { await self.loadLyrics(for: TrackIdentity(snapshot: snapshot)) }
            }
            .store(in: &cancellables)
    }

    func onStop() {
        monitor.stop()
        cancellables.removeAll()
        currentLyrics = nil
    }

    /// Guarded by a token because a slow lookup for a track the user has
    /// already skipped past would otherwise land on the new one.
    @MainActor
    func loadLyrics(for track: TrackIdentity) async {
        let token = UUID()
        lyricsTaskToken = token
        let result = await repository.lyrics(for: track)
        guard lyricsTaskToken == token else { return }
        currentLyrics = result
    }

    func previewView() -> AnyView { AnyView(MusicPreviewView(module: self)) }
    func expandedView() -> AnyView { AnyView(MusicExpandedView(module: self)) }
}
```

> 注意：`previewView()`/`expandedView()` 引用的兩個 View 在 Task 11 建立。本任務先讓它們存在為最小佔位（兩行 `Text`），Task 11 再補完整版面。

- [ ] **Step 4：建立最小佔位 View 讓專案可編譯**

`Sources/NotchCatKit/Modules/Music/Views/MusicPreviewView.swift` 與 `MusicExpandedView.swift` 各先寫：

```swift
import SwiftUI

struct MusicPreviewView: View {
    @ObservedObject var module: MusicModule
    var body: some View {
        Text(module.monitor.snapshot?.title ?? "")
            .foregroundStyle(.white)
    }
}
```

```swift
import SwiftUI

struct MusicExpandedView: View {
    @ObservedObject var module: MusicModule
    var body: some View {
        Text(module.monitor.snapshot?.title ?? "")
            .foregroundStyle(.white)
    }
}
```

- [ ] **Step 5：執行測試確認通過**

Run: `swift test --filter MusicModuleTests 2>&1 | tail -20`
Expected: PASS，3 項全過

- [ ] **Step 6：提交**

```bash
git add Sources/NotchCatKit/Modules/Music Tests/NotchCatKitTests/Music
git commit -m "Add MusicModule wiring now-playing to lyrics with stale-request guard"
```

---

## Task 11：Preview 與 Expanded 版面

**Files:**
- Modify: `Sources/NotchCatKit/Modules/Music/Views/MusicPreviewView.swift`
- Modify: `Sources/NotchCatKit/Modules/Music/Views/MusicExpandedView.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/MarqueeText.swift`
- Create: `Tests/NotchCatKitTests/Music/MarqueeTextTests.swift`

**Interfaces:**
- Consumes: Task 10 `MusicModule`
- Produces: `struct MarqueeText: View`，`init(_ text: String, font: Font)`；`enum TextFitter { static func truncate(_ text: String, max: Int) -> String }`

- [ ] **Step 1：寫會失敗的測試**

```swift
import XCTest
@testable import NotchCatKit

final class MarqueeTextTests: XCTestCase {
    // Review Focus 5: 極長歌名不得撐破版面
    func testTruncatesOverlyLongText() {
        let long = String(repeating: "長", count: 200)
        let fitted = TextFitter.truncate(long, max: 40)
        XCTAssertLessThanOrEqual(fitted.count, 41)
        XCTAssertTrue(fitted.hasSuffix("…"))
    }

    func testLeavesShortTextUntouched() {
        XCTAssertEqual(TextFitter.truncate("短標題", max: 40), "短標題")
    }

    func testExactLengthIsNotTruncated() {
        let text = String(repeating: "a", count: 40)
        XCTAssertEqual(TextFitter.truncate(text, max: 40), text)
    }

    func testEmptyTextIsSafe() {
        XCTAssertEqual(TextFitter.truncate("", max: 40), "")
    }
}
```

- [ ] **Step 2：執行測試確認失敗**

Run: `swift test --filter MarqueeTextTests 2>&1 | tail -20`
Expected: FAIL，`cannot find 'TextFitter' in scope`

- [ ] **Step 3：實作文字處理與跑馬燈**

```swift
import SwiftUI

/// Long Chinese titles with a `(feat. …)` suffix overflow the notch easily,
/// and an overflowing title pushes the transport controls off-screen.
enum TextFitter {
    static func truncate(_ text: String, max: Int) -> String {
        guard text.count > max else { return text }
        return String(text.prefix(max)) + "…"
    }
}

struct MarqueeText: View {
    let text: String
    let font: Font

    init(_ text: String, font: Font = .system(size: 12, weight: .medium)) {
        self.text = text
        self.font = font
    }

    var body: some View {
        Text(TextFitter.truncate(text, max: 40))
            .font(font)
            .lineLimit(1)
            .truncationMode(.tail)
            .foregroundStyle(.white)
    }
}
```

- [ ] **Step 4：實作 Preview 版面**

```swift
import SwiftUI

/// Ambient content while the cursor is away: the current lyric line if we have
/// timed lyrics, otherwise just what is playing. Never blank while music plays.
struct MusicPreviewView: View {
    @ObservedObject var module: MusicModule
    @State private var position: TimeInterval = 0
    private let tick = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        HStack(spacing: 8) {
            if let data = module.monitor.artwork, let image = NSImage(data: data) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 18, height: 18)
                    .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
            }
            MarqueeText(displayText)
        }
        .padding(.horizontal, 10)
        .onReceive(tick) { _ in
            guard let snapshot = module.monitor.snapshot else { return }
            position = PlaybackClock(snapshot: snapshot).position(at: Date())
        }
    }

    private var displayText: String {
        if let lyrics = module.currentLyrics,
           case .synced(let lines) = lyrics,
           let index = lyrics.lineIndex(at: position) {
            return lines[index].text
        }
        guard let snapshot = module.monitor.snapshot else { return "" }
        return "\(snapshot.title) — \(snapshot.artist)"
    }
}
```

- [ ] **Step 5：實作 Expanded 版面**

```swift
import SwiftUI

struct MusicExpandedView: View {
    @ObservedObject var module: MusicModule
    @State private var position: TimeInterval = 0
    private let tick = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    var body: some View {
        HStack(spacing: 12) {
            artwork
            VStack(alignment: .leading, spacing: 4) {
                MarqueeText(module.monitor.snapshot?.title ?? "", font: .system(size: 13, weight: .semibold))
                MarqueeText(module.monitor.snapshot?.artist ?? "", font: .system(size: 11))
                    .opacity(0.7)
                progressBar
                transport
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .onReceive(tick) { _ in
            guard let snapshot = module.monitor.snapshot else { return }
            position = PlaybackClock(snapshot: snapshot).position(at: Date())
        }
    }

    /// Placeholder keeps the same footprint as real art so the layout does not
    /// shift when artwork arrives a few seconds late.
    private var artwork: some View {
        Group {
            if let data = module.monitor.artwork, let image = NSImage(data: data) {
                Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
            } else {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(.white.opacity(0.12))
                    .overlay(Image(systemName: "music.note").foregroundStyle(.white.opacity(0.5)))
            }
        }
        .frame(width: 52, height: 52)
        .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
    }

    private var progressBar: some View {
        GeometryReader { geo in
            let duration = module.monitor.snapshot?.duration ?? 0
            let fraction = duration > 0 ? min(position / duration, 1) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.2))
                Capsule().fill(.white.opacity(0.8)).frame(width: geo.size.width * fraction)
            }
        }
        .frame(height: 3)
    }

    private var transport: some View {
        HStack(spacing: 16) {
            button("backward.fill") { module.monitor.send(.previous) }
            button("playpause.fill") { module.monitor.send(.togglePlayPause) }
            button("forward.fill") { module.monitor.send(.next) }
        }
    }

    private func button(_ symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11))
                .foregroundStyle(.white.opacity(0.85))
        }
        .buttonStyle(.plain)
    }
}
```

> **若 Task 2 Step 5 判定播放控制不可行**，刪除 `transport` 與 `button`，改為顯示唯讀播放狀態圖示，並同步修訂 spec 第 7 節。

- [ ] **Step 6：執行測試並建置**

Run: `swift test 2>&1 | tail -20 && swift build`
Expected: 全部測試通過，建置成功

- [ ] **Step 7：提交**

```bash
git add Sources/NotchCatKit/Modules/Music/Views Tests/NotchCatKitTests/Music
git commit -m "Add Music preview and expanded layouts with overflow-safe text"
```

---

## Task 12：安裝模組並做實機驗收

**Files:**
- Modify: `Sources/NotchCatKit/AppDelegate.swift`
- Modify: `README.md`
- Modify: `docs/HANDOFF.md`

**Interfaces:**
- Consumes: Task 10 `MusicModule`
- Produces: 無（本任務是整合與驗收）

- [ ] **Step 1：在 AppDelegate 安裝 MusicModule**

把 `applicationDidFinishLaunching` 裡的模組安裝段落改成：

```swift
        let moduleManager = ModuleManager()
        // Clock first so it stays the Expanded fallback when nothing is
        // active; Music's higher priority still wins Preview whenever it is
        // actually playing.
        moduleManager.install(ClockModule())
        if let music = MusicModule() {
            moduleManager.install(music)
        } else {
            NSLog("NotchCat: MediaRemote unavailable, music module disabled.")
        }
```

- [ ] **Step 2：建置並執行**

Run: `swift build && ./.build/debug/NotchCat`
Expected: 建置通過，App 啟動無錯誤輸出

- [ ] **Step 3：逐項實機驗收**

播放音樂，逐項確認並記錄結果：

- [ ] 播放音樂時，面板自動從 Hidden 變成 Preview
- [ ] Preview 顯示當前歌詞行，且隨播放前進而換行
- [ ] 暫停後面板退回 Hidden
- [ ] hover 進入 Expanded，顯示封面、歌名、演出者、進度條
- [ ] 進度條隨播放前進
- [ ] 上一首／暫停／下一首三顆按鈕實際生效
- [ ] 切到沒有歌詞的曲目時，Preview 改顯示「歌名 — 演出者」，不開天窗
- [ ] 關閉網路後切歌，音樂資訊照常顯示，只是沒有歌詞
- [ ] 封面延遲出現時版面不位移
- [ ] 連按下一首數次，歌詞不會停在舊曲目

- [ ] **Step 4：更新文件**

README 的「目前進度」加入 Phase 2 條目並註明實機驗收日期。HANDOFF 把 Phase 2 標記為完成，記錄 Step 3 中任何未通過的項目。

- [ ] **Step 5：提交**

```bash
git add -A
git commit -m "Install MusicModule and record Phase 2 acceptance"
```

---

## 自審紀錄

**Spec 覆蓋：** spec 第 4 節的檔案配置對應 Task 2 至 11；第 5.1 節兩檔查詢與封面補查對應 Task 9；第 5.2 節本地推算對應 Task 3；第 5.3 節 isActive 對應 Task 10；第 6.1 至 6.4 節對應 Task 4 至 7；第 7 節三模式版面對應 Task 11；第 8 節播放控制對應 Task 2 與 11；第 9 節錯誤處理分散於 Task 6（逾時與失敗）、Task 9（符號解析失敗）、Task 11（退化版面）；第 10 節測試策略對應各任務的測試步驟與 Task 12 的實機清單。無未覆蓋項目。

**型別一致性：** `MediaRemoteProviding` 的三個方法簽章在 Task 2、9、10 的假實作中一致；`TrackIdentity.normalized`/`cacheKey`/`durationMatches`/`loosely` 在 Task 4 定義、Task 6 與 7 使用；`Lyrics.isSynced`/`lineIndex(at:)` 在 Task 5 定義、Task 6 與 11 使用；`NowPlayingSnapshot.trackKey` 在 Task 3 定義、Task 10 使用。

**已知順序相依：** Task 10 會引用 Task 11 才完成的兩個 View，因此 Task 10 Step 4 先建立最小佔位版本以維持可編譯。這是刻意的，不是遺漏。
