# Phase 2 Music 模組實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在瀏海上顯示系統現正播放的任何來源（YouTube Music、YouTube、Spotify…），並以一份有順序的歌詞來源清單取得逐行同步歌詞，Preview 為兩翼加可折疊的歌詞條，Expanded 有封面、進度條、播放控制與歌詞。

**Architecture:** 播放來源是 `NowPlayingSource` 清單：第一順位 `MediaRemoteSource` 啟動 `/usr/bin/perl` 載入我們的 C 動態庫來讀系統現正播放（主程序自己讀不到，見 spec 第 3 節），備援 `WindowTitleSource` 讀視窗標題。歌詞是 `LyricsProvider` 清單，由 `LyricsRepository` 依序嘗試。兩邊只靠 `TrackIdentity` 溝通。Shell 只新增一個契約：模組可以要求 Preview 額外高度。

**Tech Stack:** Swift 5.9 工具鏈、SwiftUI、AppKit、Combine、XCTest、SPM（含一個 C target）。

**Spec:** [`docs/superpowers/specs/2026-10-14-music-module-design.md`](../specs/2026-10-14-music-module-design.md)（以下簡稱「spec」）。前置已完成：library 拆分（`NotchCatKit`）與 Shell／模組系統。

**舊 plan 的用途：** `git show 7112ae3:docs/superpowers/plans/2026-10-14-music-module.md` 是上一版 plan，已過時，**只能當 Task 8、9、10、11 的程式碼參考**（LRC 解析、Repository、兩個 provider）。凡與本 plan 的介面或測試不一致處，以本 plan 為準。

## Global Constraints

- 最低系統需求 macOS 14，`platforms: [.macOS(.v14)]` 不得調降；`swift-tools-version: 5.9` 不得升到 6.0（嚴格並行檢查會讓既有 Shell 大量報錯）。測試框架用 XCTest。
- 只用 Combine（`ObservableObject`／`@Published`／`AnyPublisher`），**不得使用 Observation 框架**。
- **私有 API 只准出現在兩處：** `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteControl.swift`（只送控制指令）與 `Sources/NotchCatMediaRemoteHelper/`（C，讀取，只在 perl 行程內執行）。Swift 主程序內其他檔案不得出現 `dlopen`、`dlsym` 或任何 `MRMediaRemote*` 符號名稱。
- 歌詞來源逾時一律 5 秒，逾時視同未命中。曲目長度比對採 ±5 秒容差且只在雙方都有長度時才比。歌名比對不得用字串完全相等，必須容忍簡繁體與大小寫。
- 純文字歌詞不算「可用」，只當保底；帶時間軸的才可停止走訪（spec 6.2）。
- 目前位置一律由 `elapsed + (now − timestamp) × rate` 算，且不超過總長度。起點未知時回報無法推估，**不得拿 0 充數**。
- 歌詞條底色為黑色 0.6 透明度、高度 24pt、Preview 額外高度上限 28pt，三者各寫成單一具名常數。折疊狀態存 `UserDefaults`，預設展開。
- 文件與程式碼註解只描述 Preview 與 Expanded 兩個模式，不得提及已移除的其他模式，也不得描述狀態數量改變的歷史。
- 每個 Task 結束時 `swift build` 與 `swift test` 都必須通過才能提交。提交訊息結尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 新增原始碼置於 `Sources/NotchCatKit/`（C helper 除外），測試置於 `Tests/NotchCatKitTests/`。

## Review Focus

spec 隱含、但沒有任何任務的測試會自然涵蓋的輸入類別，依「使用者實際踩到的機率」排序。每一條都已在對應任務加上測試。

1. **helper 的輸出被切斷或夾雜垃圾。** 管線讀取可能在一行中間斷開，行程也可能印出非 JSON 的雜訊。解析器不得崩潰、不得把半截字串當曲目。測試歸 Task 4。
2. **快速連續切歌造成的競態。** 前一首的歌詞請求晚於後一首回來，把舊歌詞套到新歌上。測試歸 Task 12。
3. **暫停或剛換曲時的位置。** `rate` 為 0、`elapsed` 為 0、時間戳在未來、位置超過總長度——進度條與歌詞捲動不得倒退、不得超出。測試歸 Task 3。
4. **perl 或動態庫不存在。** `/usr/bin/perl` 被移除或動態庫找不到時，來源要回報 nil 並退到備援，App 不得崩潰。測試歸 Task 5。
5. **畸形的 LRC 內容。** 未排序、重複時間標籤、空行、完全沒有時間標籤卻被當成同步。解析器不得崩潰，且沒有有效時間標籤就判為非同步。測試歸 Task 9。

---

## Task 1：Shell 支援「Preview 額外高度」

spec 第 7.1 節。這是本模組對 Shell 的唯一改動，先做，後面的畫面才有地方長出歌詞條。

**Files:**
- Modify: `Sources/NotchCatKit/ModuleKit/NotchModule.swift`（新增屬性與預設實作）
- Modify: `Sources/NotchCatKit/ModuleKit/ModuleManager.swift`
- Modify: `Sources/NotchCatKit/Shell/NotchMode.swift`
- Modify: `Sources/NotchCatKit/Shell/NotchShellController.swift`
- Modify: `Sources/NotchCatKit/UI/NotchContentView.swift`
- Test: `Tests/NotchCatKitTests/ModuleManagerPreviewHeightTests.swift`

**Interfaces:**
- Produces:
  - `NotchModule.previewExtraHeightPublisher: AnyPublisher<CGFloat, Never>`，protocol extension 提供預設值 `Just(0).eraseToAnyPublisher()`。
  - `ModuleManager.previewExtraHeight: CGFloat`（`@Published private(set)`），為 active 模組目前回報的值，夾在 `0...NotchMode.maxPreviewExtraHeight`；沒有 active 模組為 0。
  - `NotchMode.maxPreviewExtraHeight: CGFloat`（28）、`static func clampedPreviewExtra(_ value: CGFloat) -> CGFloat`。
  - `NotchMode.size(for:previewExtraHeight:)` 與 `localRect(for:previewExtraHeight:)`：`previewExtraHeight` 參數預設 0，只影響 `.preview`。
  - `NotchShellState.previewExtraHeight: CGFloat`（`@Published`）。

- [ ] **Step 1: 寫失敗的測試** `ModuleManagerPreviewHeightTests`，用測試內假模組（遵循 `NotchModule`，`previewExtraHeightPublisher` 由 `CurrentValueSubject` 驅動）：
  - `testDefaultsToZeroWithoutActiveModule`：沒有 active 模組時 `previewExtraHeight == 0`。
  - `testFollowsActiveModuleValue`：模組 active 且回報 24 → manager 為 24；改成 10 → 10。
  - `testClampsToMaximum`：回報 500 → `NotchMode.maxPreviewExtraHeight`；回報 -5 → 0。
  - `testInactiveModuleDoesNotContribute`：模組回報 24 但 `isActive == false` → 0。
  - `testSwitchingActiveModuleSwitchesSource`：兩個模組先後 active，值跟著 active 的那個。
  - `testNotchModuleDefaultIsZero`：沒覆寫該屬性的模組，發布值為 0。
  - `testClampedPreviewExtra`：純函式，涵蓋負數、0、上限內、超過。
- [ ] **Step 2: 確認失敗**：`swift test --filter ModuleManagerPreviewHeightTests`，預期編譯失敗（符號不存在）。
- [ ] **Step 3: 實作**
  - `ModuleManager`：以 `$activeModule` 的 `map` + `switchToLatest` 訂閱 active 模組的 `previewExtraHeightPublisher`，經 `NotchMode.clampedPreviewExtra` 後寫入 `previewExtraHeight`。
  - `NotchMode.size(for:previewExtraHeight:)`：`.preview` 高度 = 瀏海高 + 夾限後的額外高度，寬度不變。`containerSize` 要容得下 `preview + maxPreviewExtraHeight` 與 expanded 兩者的最大值。
  - `NotchShellController`：訂閱 `moduleManager.$previewExtraHeight` 寫入 `state.previewExtraHeight`；**collapsed 狀態下**每次額外高度變動，立即把 `hostingView.hoverRect` 重設為 `NotchMode.preview.localRect(for:previewExtraHeight:)`（歌詞條範圍也算 hover）；expanded 狀態維持整個容器。
  - `NotchContentView`：Preview 且額外高度 > 0 時，Shell 只以純黑填「瀏海高度」那一段（下緣改為直角），額外高度那一段**不填色**，交給模組的 `previewView()` 自己畫背景（半透明）；模組的 Preview 內容吃滿整個 Preview 尺寸，**Preview 不套用 Expanded 的 `padding(.top, 8)`**。額外高度的變動要跟 mode 切換用同一條動畫（`.animation(.easeInOut(duration: 0.28), value:)`）。
- [ ] **Step 4: 確認通過**：`swift build && swift test`，全部通過。
- [ ] **Step 5: 提交**：`git add Sources Tests && git commit -m "Let modules request extra Preview height"`

---

## Task 2：把 MediaRemoteBridge 瘦身成只送控制指令

spec 第 4 節邊界一：主程序不得再有任何讀取類私有呼叫。

**Files:**
- Delete: `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteBridge.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteControl.swift`
- Delete: `Tests/NotchCatKitTests/Music/MediaRemoteBridgeTests.swift`
- Create: `Tests/NotchCatKitTests/Music/MediaRemoteControlTests.swift`

**Interfaces:**
- Produces:
  - `enum MediaRemoteCommand: UInt32 { case play = 0, pause = 1, togglePlayPause = 2, next = 4, previous = 5 }`（沿用舊值）。
  - `protocol MediaRemoteControlling: AnyObject { @discardableResult func send(_ command: MediaRemoteCommand) -> Bool }`
  - `final class MediaRemoteControl: MediaRemoteControlling`，`init?(frameworkPath: String = defaultFrameworkPath, symbolName: String = "MRMediaRemoteSendCommand")`，載入失敗回 nil。

- [ ] **Step 1: 寫失敗的測試**：
  - `testCommandRawValuesMatchMediaRemote`：五個指令的 rawValue 分別為 0、1、2、4、5。
  - `testInitFailsWhenFrameworkPathIsWrong`：給不存在的路徑 → nil。
  - `testInitFailsWhenSymbolIsAbsent`：給不存在的 symbol 名稱 → nil。
  - `testConstructsAgainstRealFramework`：預設參數可建構（此機器 macOS 15.7 實測可用）。
- [ ] **Step 2: 確認失敗**（`MediaRemoteControl` 不存在）。
- [ ] **Step 3: 實作** `MediaRemoteControl`：`dlopen` 載入框架、`dlsym` 取 `MRMediaRemoteSendCommand`，簽章 `@convention(c) (UInt32, CFDictionary?) -> Bool`；`send` 轉呼叫。沿用舊檔對 `dlopen`／`dlsym` 的註解風格，並註明「只用於控制，不得在此加入任何讀取呼叫」。刪除舊檔與舊測試。
- [ ] **Step 4: 確認通過**：`swift build && swift test`；`grep -rn "MRMediaRemote\|dlopen\|dlsym" Sources/NotchCatKit` 只應命中 `MediaRemoteControl.swift`。
- [ ] **Step 5: 提交**：`git add -A Sources Tests && git commit -m "Reduce the MediaRemote bridge to control commands only"`

---

## Task 3：Snapshot 與 PositionAnchor

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingSnapshot.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingSource.swift`
- Test: `Tests/NotchCatKitTests/Music/PositionAnchorTests.swift`

**Interfaces:**
- Produces（簽章照 spec 5.1，補上 `ArtworkRef`）：
  - `struct ArtworkRef: Equatable { let identifier: String; let data: Data? }`
  - `struct PositionAnchor: Equatable { let elapsed: TimeInterval; let timestamp: Date; let rate: Double; func position(at now: Date, cappedTo duration: TimeInterval?) -> TimeInterval }`
  - `struct NowPlayingSnapshot: Equatable { let title: String; let artist: String?; let album: String?; let duration: TimeInterval?; let isPlaying: Bool; let position: PositionAnchor?; let artwork: ArtworkRef?; let sourceID: String }`
  - `protocol NowPlayingSource { var id: String { get }; var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { get } }`

- [ ] **Step 1: 寫失敗的測試** `PositionAnchorTests`（Review Focus 3）：
  - `testAdvancesWithRate`：elapsed 10、timestamp 為 T、rate 1，在 T+5 → 15。
  - `testHonorsNonUnitRate`：rate 2，T+5 → 20。
  - `testPausedFreezes`：rate 0，T+100 → 10。
  - `testCapsAtDuration`：elapsed 230、duration 233.9、T+100 → 233.9。
  - `testNoDurationMeansNoCap`：duration 為 nil 時不夾。
  - `testTimestampInFutureDoesNotRewind`：now 早於 timestamp → 不小於 elapsed，也不為負。
  - `testNegativeElapsedClampsToZero`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 型別與 `position(at:cappedTo:)`：`max(0, elapsed + max(0, now − timestamp) × rate)`，再夾總長度。`NowPlayingSource` 檔只放協定。
- [ ] **Step 4: 確認通過**：`swift test --filter PositionAnchorTests`。
- [ ] **Step 5: 提交**：`git commit -m "Add NowPlayingSnapshot and PositionAnchor"`

---

## Task 4：helper 訊息格式與解析

定義 C helper 與 Swift 之間的線上格式，並以純函式解析。這一步先定格式，Task 5 的 C 程式照它輸出。

**線上格式（一行一個 JSON 物件，UTF-8，以 `\n` 結尾）：**
- 沒有現正播放：`{"none":true}`
- 有曲目：`{"title":String,"artist":String?,"album":String?,"duration":Number?,"elapsed":Number,"timestamp":Number,"rate":Number,"artworkId":String?,"artworkData":String?}`，`timestamp` 為 Unix epoch 秒，`artworkData` 為 base64 的圖片位元組，缺席代表這次沒附。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/HelperMessageParser.swift`
- Test: `Tests/NotchCatKitTests/Music/HelperMessageParserTests.swift`

**Interfaces:**
- Produces:
  - `enum HelperMessage: Equatable { case none; case track(NowPlayingSnapshot) }`
  - `enum HelperMessageParser { static func parse(line: String) -> HelperMessage? }`：無法解析回 nil；`sourceID` 固定 `"mediaremote"`；`isPlaying == (rate > 0)`；`elapsed`／`timestamp`／`rate` 轉成 `PositionAnchor`；空字串的 `album`／`artist` 視為 nil；`title` 缺席或為空 → 整行回 nil。
  - `struct HelperLineBuffer { mutating func append(_ data: Data) -> [String] }`：累積位元組，只回傳已完整的行（Review Focus 1）。

- [ ] **Step 1: 寫失敗的測試**：
  - `testParsesFullTrack`：用 2026-11-04 實測樣本——title「蘇裔非 - 抱我!  ft. 莊岱諺」、artist「蘇裔非」、album 空字串、duration 213.081、elapsed 0、rate 1、artworkId 為 UUID 字串、附 `artworkData` base64 → 欄位正確、`album == nil`、`isPlaying == true`、`artwork?.data != nil`。
  - `testParsesNone`：`{"none":true}` → `.none`。
  - `testArtworkDataAbsentKeepsIdentifier`：只有 `artworkId` → `artwork?.identifier` 有值、`data == nil`。
  - `testRateZeroIsPaused`：`rate: 0` → `isPlaying == false`，仍回 `.track`。
  - `testMissingTitleYieldsNil`、`testEmptyTitleYieldsNil`。
  - `testGarbageYieldsNil`：`"hello"`、`"{"`、空字串、`"[]"` 全回 nil 且不崩潰。
  - `testMissingOptionalFieldsAreFine`：只有 title／elapsed／timestamp／rate。
  - `testLineBufferSplitsAcrossReads`：`append` 兩次，第一次只有半行 → 回 `[]`，第二次補齊 → 回 1 行。
  - `testLineBufferHandlesMultipleLinesInOneRead`。
  - `testLineBufferDropsInvalidUTF8Line`：夾入無效位元組的行被丟棄，後面的行仍可解析。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 以 `JSONSerialization`（容忍數字型別差異）解析；`HelperLineBuffer` 以 `\n` 切行。
- [ ] **Step 4: 確認通過**：`swift test --filter HelperMessageParserTests`。
- [ ] **Step 5: 提交**：`git commit -m "Define the helper wire format and parse it"`

---

## Task 5：C 動態庫、perl 宿主與 MediaRemoteSource

**這是最有風險的任務，Step 1 先做可行性驗證，失敗就停下來回報，不要硬做。**

**Files:**
- Create: `Sources/NotchCatMediaRemoteHelper/helper.c`
- Create: `Sources/NotchCatMediaRemoteHelper/include/NotchCatMediaRemoteHelper.h`
- Modify: `Package.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/MediaRemoteSource.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/PerlHelperLauncher.swift`
- Test: `Tests/NotchCatKitTests/Music/MediaRemoteSourceTests.swift`

**Interfaces:**
- Consumes: Task 3 `NowPlayingSnapshot`／`NowPlayingSource`、Task 4 `HelperMessageParser`／`HelperLineBuffer`。
- Produces:
  - C：`void notchcat_helper_main(void *interp, void *cv)`，perl 以 XSUB 方式呼叫（簽章刻意如此）。它在行程內每 1 秒呼叫 `MRMediaRemoteGetNowPlayingInfo`（等待上限 3 秒），**只在內容變動時**（title／artist／rate／elapsed／timestamp／artworkId／是否附圖任一變）輸出一行（格式見 Task 4），並在標準輸入 EOF 時 `_exit(0)`（父行程死掉時 helper 跟著結束）。沒有現正播放輸出 `{"none":true}`（同樣只在變動時）。
  - `Package.swift` 新增 `.target(name: "NotchCatMediaRemoteHelper", linkerSettings: [.linkedFramework("CoreFoundation")])` 與 `.library(name: "NotchCatMediaRemoteHelper", type: .dynamic, targets: ["NotchCatMediaRemoteHelper"])` product；`NotchCatKit` **不**依賴它（它只在 perl 行程內載入）。
  - `protocol HelperProcessHandle { func terminate() }`
  - `protocol HelperProcessLaunching { func launch(onLine: @escaping (String) -> Void, onExit: @escaping (Int32) -> Void) throws -> HelperProcessHandle }`
  - `final class PerlHelperLauncher: HelperProcessLaunching`，`init(perlPath: String = "/usr/bin/perl", dylibURL: URL? = PerlHelperLauncher.defaultDylibURL())`；`static func defaultDylibURL() -> URL?` 回傳可執行檔同目錄的 `libNotchCatMediaRemoteHelper.dylib`（不存在回 nil）。`launch` 在 perl 或動態庫不存在時 `throw`。perl 腳本內容固定，以 `-e` 傳入，嵌在 Swift 檔裡：以 `DynaLoader::dl_load_file`、`dl_find_symbol`、`dl_install_xsub` 載入並呼叫 `notchcat_helper_main`（參考 2026-11-04 探針，其腳本已在 spec 第 3 節描述）。stdin 用管線保持開啟。
  - `final class MediaRemoteSource: NowPlayingSource`，`init(launcher: HelperProcessLaunching, retryDelays: [TimeInterval] = [1, 2, 4, 8, 16, 30], schedule: @escaping (TimeInterval, @escaping () -> Void) -> Void = { DispatchQueue.main.asyncAfter(deadline: .now() + $0, execute: $1) })`；`func start()`、`func stop()`；`snapshots` 發布 `.track` 轉成的 snapshot，`.none` 發布 nil。
  - 行為：行程意外結束時依 `retryDelays` 重啟；**一次成功收到任何一行後，重試計數歸零**；`retryDelays` 用盡仍失敗 → 發布 nil 並放棄（不再重啟）；`launch` 直接 throw 也算一次失敗。`stop()` 終止行程且不重啟。

- [ ] **Step 1: 可行性驗證（先於所有程式碼）。**
  - 建立 `helper.c`（以 2026-11-04 探針為底，改成 Task 4 的輸出格式與迴圈）與 header，改 `Package.swift`。
  - 執行 `swift build`，確認產出 `.build/debug/libNotchCatMediaRemoteHelper.dylib`。
  - 檢查簽章：`codesign -dv .build/debug/libNotchCatMediaRemoteHelper.dylib 2>&1 | head`。若沒有簽章，在 `Package.swift` 之外以 build 後步驟 `codesign -s - -f` 處理，並把做法寫進本檔註解。
  - 手動驗證：開著任何播放中的音樂，執行 `/usr/bin/perl -e '<腳本>' .build/debug/libNotchCatMediaRemoteHelper.dylib | head -3`，預期輸出含 `"title"` 的 JSON 行；暫停或換曲後有新行出現。
  - 若動態庫載入失敗或輸出為空：**停止，把完整錯誤回報**，不要往下做。
- [ ] **Step 2: 同時驗證暫停與播放的輸出**：暫停音樂後確認輸出是 `rate 0` 還是 `{"none":true}`，把結果寫成 `helper.c` 檔頭註解（spec 5.5 列為待驗證項目）。再拖動進度一次，確認是否輸出新行（spec 5.2）；結果也寫進註解與 `docs/superpowers/specs/2026-10-14-music-module-design.md` 第 5.2、5.5 節的待驗證語句。
- [ ] **Step 3: 寫失敗的測試** `MediaRemoteSourceTests`，用假的 `HelperProcessLaunching`（可手動觸發 `onLine`／`onExit`，記錄啟動次數）與假的 `schedule`（收集排程，手動執行）：
  - `testPublishesTrackFromLine`：餵一行 track JSON → 發布對應 snapshot。
  - `testNoneLinePublishesNil`。
  - `testGarbageLineIsIgnored`：餵垃圾行 → 沒有新發布。
  - `testRestartsAfterUnexpectedExitWithBackoff`：結束三次 → 排程的延遲依序為 1、2、4。
  - `testSuccessfulLineResetsBackoff`：結束一次、收到一行資料、再結束 → 第二次延遲仍為 1。
  - `testGivesUpAfterExhaustingDelays`：全部用盡後發布 nil、不再排程。
  - `testLaunchThrowingCountsAsFailure`（Review Focus 4）：`launch` 一直 throw → 依序重試、最後放棄並發布 nil，過程不崩潰。
  - `testStopTerminatesAndDoesNotRestart`。
  - `testPerlLauncherThrowsWhenPerlMissing`、`testPerlLauncherThrowsWhenDylibMissing`：給不存在的路徑 → `launch` throw。
- [ ] **Step 4: 確認失敗**。
- [ ] **Step 5: 實作** `MediaRemoteSource` 與 `PerlHelperLauncher`（用 `Process` 與 `Pipe`，`readabilityHandler` 餵 `HelperLineBuffer`）。
- [ ] **Step 6: 確認通過**：`swift build && swift test`。再做一次端對端手動驗證：寫一個臨時的 `swift run` 腳本或測試（不提交）印出 `MediaRemoteSource` 發布的 snapshot，確認真的拿到目前播放。
- [ ] **Step 7: 提交**：`git add -A Package.swift Sources Tests && git commit -m "Read system now-playing through an Apple-signed perl host"`

---

## Task 6：視窗標題備援與位置推算

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/WindowTitleParser.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/PlaybackEstimator.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/WindowTitleSource.swift`
- Test: `Tests/NotchCatKitTests/Music/WindowTitleParserTests.swift`
- Test: `Tests/NotchCatKitTests/Music/PlaybackEstimatorTests.swift`
- Test: `Tests/NotchCatKitTests/Music/WindowTitleSourceTests.swift`

**Interfaces:**
- Consumes: Task 3 型別。
- Produces:
  - `struct WindowInfo: Equatable { let owner: String; let title: String }`
  - `struct ParsedTrack: Equatable { let title: String; let isPlaying: Bool }`
  - `enum WindowTitleParser { static func parse(_ window: WindowInfo) -> ParsedTrack? }`：擁有者須在清單內（v1：`YouTube Music`）；格式 `YouTube Music - <曲名> | YouTube Music`，尾端喇叭符號 `🔊` 表示播放中（有 → `isPlaying == true`），曲名剝除前後綴與喇叭符號與多餘空白；未知格式、空標題、擁有者不在清單 → nil。
  - `struct PlaybackEstimator { init(); mutating func observe(title: String?, isPlaying: Bool, at: Date); func anchor() -> PositionAnchor? }`：只有「模組執行期間觀察到換曲」才有錨點（elapsed 0、timestamp 為觀察時刻、rate 1）；暫停時凍結為 rate 0 的錨點；恢復時 elapsed 取凍結值。**第一次觀察到某首曲目時（起點未知）`anchor()` 回 nil。**
  - `final class WindowTitleSource: NowPlayingSource`，`init(windows: @escaping () -> [WindowInfo] = WindowTitleSource.systemWindows, now: @escaping () -> Date = Date.init, schedule…)`；輪詢 1 秒（目前有播放）／5 秒（沒有）；`static func systemWindows() -> [WindowInfo]` 用 `CGWindowListCopyWindowInfo`（`.optionOnScreenOnly` 以外還需包含背景視窗以外的視窗，以實測可讀到 YouTube Music PWA 標題為準）。

- [ ] **Step 1: 寫失敗的測試**：
  - Parser：`testParsesPlayingTitle`（`"YouTube Music - 假日而已 | YouTube Music 🔊"` → title「假日而已」、playing）、`testParsesPausedTitleWithoutSpeaker`、`testUnknownOwnerReturnsNil`、`testEmptyTitleReturnsNil`、`testUnknownFormatReturnsNil`（不得回傳半截字串）、`testTitleContainingSeparators`（曲名含 ` - ` 或 `|` 時仍取到完整曲名）。
  - Estimator：`testFirstObservationHasNoAnchor`（起點未知，不得拿 0 充數）、`testTrackChangeWhileRunningYieldsAnchorAtZero`、`testPauseFreezesPosition`（暫停 10 秒後 `position(at:)` 仍為暫停當下的值）、`testResumeContinuesFromFrozenPosition`、`testSameTrackReobservedKeepsAnchor`。
  - Source：以假視窗清單驗證 `testPublishesSnapshotForMatchingWindow`、`testPublishesNilWhenNoWindowMatches`、`testIgnoresNonMatchingOwner`、`testSnapshotCarriesAnchorOnlyAfterObservedChange`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 三個檔案。`WindowTitleSource` 的 `snapshots` 不含 `duration`／`artwork`，`artist` 為 nil。
- [ ] **Step 4: 確認通過**：`swift test --filter WindowTitle` 與 `--filter PlaybackEstimator`。
- [ ] **Step 5: 提交**：`git commit -m "Add the window-title fallback source and position estimator"`

---

## Task 7：NowPlayingMonitor 與 ArtworkCache

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/NowPlayingMonitor.swift`
- Create: `Sources/NotchCatKit/Modules/Music/NowPlaying/ArtworkCache.swift`
- Test: `Tests/NotchCatKitTests/Music/NowPlayingMonitorTests.swift`
- Test: `Tests/NotchCatKitTests/Music/ArtworkCacheTests.swift`

**Interfaces:**
- Consumes: Task 3 `NowPlayingSource`、`NowPlayingSnapshot`。
- Produces:
  - `final class NowPlayingMonitor: ObservableObject`，`init(sources: [NowPlayingSource], artwork: ArtworkCache = ArtworkCache())`；`@Published private(set) var current: NowPlayingSnapshot?`；`var isPlayingPublisher: AnyPublisher<Bool, Never>`（`current?.isPlaying ?? false`，去重）。
  - 規則：依 `sources` 陣列順序，每個來源**最新**值中第一個非 nil 的即為 `current`；尚未發布過的來源視為 nil。`current` 的 `artwork` 經 `ArtworkCache` 補齊：該 snapshot 有 `data` → 存入並使用；沒有 `data` 但快取有同識別碼的圖 → 補上；兩者皆無 → 維持 `data == nil`。
  - `final class ArtworkCache`，`init(capacity: Int = 16)`；`func resolve(_ ref: ArtworkRef?) -> ArtworkRef?`；超過容量淘汰最舊。

- [ ] **Step 1: 寫失敗的測試**（假來源由 `CurrentValueSubject` 驅動）：
  - `testFirstNonNilSourceWins`、`testFallsThroughToLaterSourceWhenFirstIsNil`、`testFirstSourceRegainingDataTakesBackOver`、`testNeverEmittedSourceCountsAsNil`。
  - `testPausedSnapshotStillWinsOverLaterSource`：高順位來源回報暫停中的 snapshot（非 nil）時，不退到備援。
  - `testIsPlayingPublisherDeduplicates`。
  - Cache：`testStoresDataAndFillsLaterMissingDataByIdentifier`（同識別碼先有圖後無圖 → 補回圖）、`testUnknownIdentifierWithoutDataStaysEmpty`、`testEvictsOldestBeyondCapacity`、`testNilRefPassesThrough`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作**：用 `Publishers.MergeMany` 搭配索引，或逐一 `sink` 到本地陣列後重算，由實作者選擇；對外契約如上。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 提交**：`git commit -m "Add NowPlayingMonitor and artwork cache"`

---

## Task 8：TrackIdentity 與 TrackNormalizer

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/TrackIdentity.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/TrackNormalizer.swift`
- Test: `Tests/NotchCatKitTests/Music/TrackIdentityTests.swift`
- Test: `Tests/NotchCatKitTests/Music/TrackNormalizerTests.swift`

**Interfaces:**
- Consumes: Task 3 `NowPlayingSnapshot`。參考舊 plan 的 Task 4 程式碼，差異是 `duration` 改為選用。
- Produces:
  - `struct TrackIdentity: Hashable { let title: String; let artist: String; let album: String?; let duration: TimeInterval? }`
  - `func durationMatches(_ other: TimeInterval?) -> Bool`：任一方為 nil 或 ≤ 0 → true，否則差 ≤ 5。
  - `static func loosely(_ a: String, matches b: String) -> Bool`：簡繁、大小寫、全半形不敏感。
  - `var cacheKey: String`：正規化後的標題＋演出者（小寫、不含長度）。
  - `enum TrackNormalizer { static func identity(from snapshot: NowPlayingSnapshot) -> TrackIdentity? }`：標題或演出者缺席 → nil（無法查）；`TrackIdentity.normalized` 負責括號後綴與主要演出者。
  - YouTube 清理（spec 6.3）：`title` 形如「A - B」且 A（忽略大小寫與空白）等於或包含於 `artist` → 取 B；再剝除 `ft.`／`feat.` 及其後內容，以及 `(MV)`、`[Official Video]`、`(Lyrics)` 類括號；連續空白收斂成一個。拆不出來就維持原標題。

- [ ] **Step 1: 寫失敗的測試**：
  - Identity：`testStripsFeatSuffix`（`善良是 (feat. CHS)`、`Song (ft. X)`、`Song [Remastered 2011]`）、`testLeavesCleanTitleAlone`、`testTakesPrimaryArtist`（`/`、`&`、`,` 分隔）、`testDurationToleranceIsFiveSeconds`、`testUnknownDurationAlwaysMatches`（nil 與 0）、`testLooseMatchIgnoresSimplifiedTraditional`（`後來有很多人去過那裡` 對 `后来有很多人去过那里`）、`testCacheKeyIgnoresDuration`。
  - Normalizer：`testYouTubeTitleSplitsOffArtist`（**實測樣本**：title `蘇裔非 - 抱我!  ft. 莊岱諺`、artist `蘇裔非` → title `抱我!`）、`testCollapsesExtraWhitespace`、`testStripsMvBrackets`、`testNoDashKeepsOriginalTitle`、`testDashButArtistDoesNotMatchKeepsOriginal`（不是頻道名就不拆）、`testMissingArtistReturnsNil`、`testMissingTitleReturnsNil`、`testCleanMusicAppTitleUntouched`（YouTube Music 風格的乾淨曲名不變）。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作**：`loosely` 以 `CFStringTransform` 的 `Hans-Hant` 兩邊轉同一方向後比較（`folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive])`）。
- [ ] **Step 4: 確認通過**：`swift test --filter TrackIdentityTests`、`--filter TrackNormalizerTests`。
- [ ] **Step 5: 提交**：`git commit -m "Add TrackIdentity and YouTube-aware normalization"`

---

## Task 9：Lyrics 型別與 LRC 解析

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Lyrics.swift`
- Test: `Tests/NotchCatKitTests/Music/LyricsTests.swift`

**Interfaces:**
- Produces（沿用舊 plan 的 Task 5，程式碼與測試可直接參考）：
  - `struct LyricLine: Equatable { let time: TimeInterval; let text: String }`
  - `enum Lyrics: Equatable { case synced([LyricLine]); case plain(String); var isSynced: Bool; func lineIndex(at position: TimeInterval) -> Int? }`
  - `enum LRCParser { static func parse(_ raw: String) -> Lyrics? }`

- [ ] **Step 1: 寫失敗的測試**（Review Focus 5），至少涵蓋：`testParsesTimestampedLines`、`testUnsortedTimestampsAreSorted`、`testTextWithoutAnyTimestampIsPlain`、`testMetadataOnlyTagsDoNotCountAsSynced`（`[ar:…]`、`[ti:…]` 不是時間軸）、`testEmptyInputYieldsNil`（空字串與純空白）、`testBlankLyricLinesAreDropped`、`testRepeatedTimestampsOnOneLine`、`testLineIndexAtPosition`（0、9.9、10、999）、`testLineIndexBeforeFirstLineIsNil`、`testLineIndexOnPlainLyricsIsNil`、`testDuplicateTimestampsKeepBothLines`、`testHugeMinutesValueDoesNotCrash`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 如簽章所列；解析器寬容、依時間排序、丟掉空文字行、沒有任何有效時間標籤就回 `.plain`。
- [ ] **Step 4: 確認通過**：`swift test --filter LyricsTests`。
- [ ] **Step 5: 提交**：`git commit -m "Add Lyrics types and a forgiving LRC parser"`

---

## Task 10：LyricsProvider 與 LyricsRepository

spec 6.2 的核心規則：依序一個一個查，**帶時間軸的才可停止**，純文字暫存當保底。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsProvider.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsRepository.swift`
- Test: `Tests/NotchCatKitTests/Music/LyricsRepositoryTests.swift`

**Interfaces:**
- Consumes: Task 8 `TrackIdentity`、Task 9 `Lyrics`。
- Produces（沿用舊 plan 的 Task 6）：
  - `protocol LyricsProvider: Sendable { var id: String { get }; func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics? }`
  - `protocol LyricsResolving: Sendable { func lyrics(for track: TrackIdentity) async -> Lyrics? }`
  - `actor LyricsRepository: LyricsResolving`，`init(providers: [LyricsProvider], negativeTTL: TimeInterval = 86_400, now: @escaping () -> Date = Date.init)`。

- [ ] **Step 1: 寫失敗的測試**（假 provider 可設結果、錯誤、延遲、記錄呼叫次數）：
  - `testSyncedResultBeatsHigherPriorityPlain`（前者純文字、後者同步 → 回同步）。
  - `testEachProviderIsQueriedAtMostOnce`。
  - `testStopsAtFirstSyncedResult`（後面的來源呼叫次數為 0）。
  - `testFallsBackToPlainKeepingEarlierProvider`（兩個都純文字 → 回順位較前者）。
  - `testFailingProviderDoesNotBlockLaterOnes`。
  - `testReturnsNilWhenNobodyHasAnything`。
  - `testPositiveResultIsCached`、`testNegativeResultIsCachedWithinTTL`、`testNegativeCacheExpires`（注入時鐘）。
  - `testCacheIsKeyedOnNormalizedIdentity`（`T` 與 `T (feat. X)` 且長度不同 → 只查一次）。
  - `testProviderTimeoutCountsAsMiss`：某 provider 延遲超過 5 秒（以可注入的逾時值縮短）→ 視同未命中，後面的來源仍被查詢。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作**：單趟走訪；逾時以 `withThrowingTaskGroup` 與 5 秒計時競賽（逾時值可注入以利測試）；其餘行為如測試。
- [ ] **Step 4: 確認通過**：`swift test --filter LyricsRepositoryTests`。
- [ ] **Step 5: 提交**：`git commit -m "Add LyricsRepository that tries providers in order"`

---

## Task 11：LRCLIB 與網易雲 provider

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/LRCLIBProvider.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Lyrics/Providers/NetEaseProvider.swift`
- Test: `Tests/NotchCatKitTests/Music/ProviderTests.swift`

**Interfaces:**
- Consumes: Task 8、9、10。程式碼參考舊 plan 的 Task 7，差異：`TrackIdentity.duration` 為選用（請求參數缺長度時省略 `duration`）。
- Produces:
  - `struct LRCLIBProvider: LyricsProvider`，`init(session: URLSession = .shared, timeout: TimeInterval = 5)`；先精確查 `https://lrclib.net/api/get`（`track_name`／`artist_name`／`album_name`／`duration`，皆用正規化後的值），失敗再 `https://lrclib.net/api/search?q=<標題 演出者>`；`static func decode(_:for:) -> Lyrics?`、`static func decodeSearch(_:for:) -> Lyrics?`。結果須通過長度容差；`syncedLyrics` 優先，沒有則 `plainLyrics`。
  - `struct NetEaseProvider: LyricsProvider`，同樣初始化簽章；先 `https://music.163.com/api/search/get/web`（`s`、`type=1`、`limit=5`）找歌，標題用 `TrackIdentity.loosely` 比對、長度（毫秒）用容差；再 `https://music.163.com/api/song/lyric?id=…&lv=1&kv=1&tv=-1` 取 `lrc.lyric`；`static func decodeLyric(_:) -> Lyrics?`。請求帶 `Referer: https://music.163.com/`。
  - 兩者的 `User-Agent` 為 `NotchCat`，不含使用者資料。

- [ ] **Step 1: 寫失敗的離線測試**：`testLRCLIBPrefersSyncedFieldOverPlain`、`testLRCLIBFallsBackToPlainWhenSyncedAbsent`、`testLRCLIBRejectsMismatchedDuration`、`testLRCLIBAcceptsDurationWithinTolerance`、`testLRCLIBAcceptsResultWhenTrackHasNoDuration`、`testLRCLIBSearchPicksFirstValidCandidate`、`testNetEaseDecodesLrcField`、`testNetEaseEmptyLyricYieldsNil`、`testNetEaseMissingLrcYieldsNil`、`testNetEaseMatchesSimplifiedTitleForTraditionalQuery`（以假的搜尋回應 JSON 驗證選歌邏輯，需將選歌邏輯抽成可離線測的靜態函式）。另一個 `testLiveLookupSmoke` 在 `NOTCHCAT_LIVE_TESTS=1` 時才跑。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 兩個 provider。
- [ ] **Step 4: 確認通過**：`swift test --filter ProviderTests`（live 項顯示 skipped）。
- [ ] **Step 5: 對外驗證一次**：`NOTCHCAT_LIVE_TESTS=1 swift test --filter testLiveLookupSmoke`。失敗代表外部服務不可用或介面變動，記錄下來但不阻擋後續任務。
- [ ] **Step 6: 提交**：`git commit -m "Add LRCLIB and NetEase lyrics providers"`

---

## Task 12：MusicModule 與呈現邏輯

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/LyricStripPreference.swift`
- Create: `Sources/NotchCatKit/Modules/Music/MusicPresentation.swift`
- Create: `Sources/NotchCatKit/Modules/Music/MusicViewModel.swift`
- Create: `Sources/NotchCatKit/Modules/Music/MusicModule.swift`
- Test: `Tests/NotchCatKitTests/Music/LyricStripPreferenceTests.swift`
- Test: `Tests/NotchCatKitTests/Music/MusicPresentationTests.swift`
- Test: `Tests/NotchCatKitTests/Music/MusicViewModelTests.swift`

**Interfaces:**
- Consumes: Task 1（額外高度契約）、2（`MediaRemoteControlling`）、7（`NowPlayingMonitor`）、8（`TrackNormalizer`）、10（`LyricsResolving`）。
- Produces:
  - `final class LyricStripPreference: ObservableObject`，`init(defaults: UserDefaults = .standard, key: String = "music.lyricStripVisible")`；`@Published var isVisible: Bool`（預設 true，寫入即存）。
  - `enum StripContent: Equatable { case line(String); case title(String); case none }`；`enum MusicPresentation { static func stripContent(snapshot: NowPlayingSnapshot?, lyrics: Lyrics?, at now: Date) -> StripContent }`：依 spec 7.3 退化階梯——同步歌詞且位置可得 → 當前行（尚未到第一行時顯示曲名）；其餘有曲目 → 曲名；無曲目 → `.none`。`static func playbackPosition(snapshot: NowPlayingSnapshot, at now: Date) -> TimeInterval?`（無錨點回 nil）。
  - `final class MusicViewModel: ObservableObject`，`init(monitor: NowPlayingMonitor, lyrics: LyricsResolving, control: MediaRemoteControlling?, preference: LyricStripPreference)`；`@Published private(set) var snapshot: NowPlayingSnapshot?`、`@Published private(set) var lyrics: Lyrics?`；`func send(_ command: MediaRemoteCommand)`（`control == nil` 時為 no-op）。換曲（`TrackNormalizer` 算出的 `cacheKey` 改變）時取消舊請求、清掉舊歌詞並重新查；**回應回來時若曲目已變就丟棄**（Review Focus 2）。
  - `final class MusicModule: NotchModule`：`id = "com.notchcat.music"`、`priority = 100`；`isActivePublisher = monitor.isPlayingPublisher`；`previewExtraHeightPublisher`：active 且 `preference.isVisible` 時為 `MusicModule.lyricStripHeight`（24），否則 0；`tabIcon()`、`previewView()`、`expandedView()` 於 Task 13 接上畫面，本任務先回傳 `EmptyView()` 包成 `AnyView`；`static func makeDefault() -> MusicModule` 組裝預設來源清單（`MediaRemoteSource(launcher: PerlHelperLauncher())`、`WindowTitleSource()`）、歌詞清單（`LRCLIBProvider()`、`NetEaseProvider()`）、`MediaRemoteControl()`，並在 `onStart` 啟動來源、`onStop` 停止。

- [ ] **Step 1: 寫失敗的測試**：
  - Preference：`testDefaultsToVisible`、`testPersistsAcrossInstances`（同一個 `UserDefaults(suiteName:)`）。
  - Presentation：`testSyncedWithPositionShowsCurrentLine`、`testSyncedBeforeFirstLineShowsTitle`、`testSyncedWithoutAnchorShowsTitle`、`testPlainLyricsShowTitle`、`testNoLyricsShowsTitle`、`testNoSnapshotIsNone`、`testPausedAnchorHoldsLine`（rate 0 時行不隨時間前進）。
  - ViewModel（假 `LyricsResolving`，可控制回應時機）：`testFetchesLyricsOnTrackChange`、`testDoesNotRefetchForSameTrack`（只有播放狀態變動）、`testStaleResponseIsDiscarded`（A 的請求未回時換成 B，A 後回來 → 不得覆蓋 B 的歌詞）、`testTrackChangeClearsPreviousLyrics`、`testMissingArtistSkipsLookup`、`testSendForwardsToControl`、`testSendWithoutControlIsNoOp`。
  - Module：`testExtraHeightIsZeroWhenInactive`、`testExtraHeightIs24WhenActiveAndVisible`、`testExtraHeightIsZeroWhenFolded`、`testFoldingUpdatesHeightImmediately`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作**。`lyricStripHeight`、`lyricStripOpacity`（0.6）定義為 `MusicModule` 的 static 常數。
- [ ] **Step 4: 確認通過**：`swift build && swift test`。
- [ ] **Step 5: 提交**：`git commit -m "Add MusicModule with presentation logic and lyric fetching"`

---

## Task 13：畫面

純 UI，單元測試只涵蓋可純函式化的部分；其餘靠建置通過與實機驗收。

**Files:**
- Create: `Sources/NotchCatKit/Modules/Music/Views/MarqueeText.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/EqualizerBars.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/LyricStripView.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/CoverView.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/MusicPreviewView.swift`
- Create: `Sources/NotchCatKit/Modules/Music/Views/MusicExpandedView.swift`
- Modify: `Sources/NotchCatKit/Modules/Music/MusicModule.swift`（接上畫面）
- Test: `Tests/NotchCatKitTests/Music/MarqueeTextTests.swift`

**Interfaces:**
- Consumes: Task 12 `MusicViewModel`、`MusicPresentation`、`LyricStripPreference`、`MusicModule` 常數；Task 1 的 Preview 契約（Preview 內容吃滿「瀏海高 + 額外高度」，額外高度那段由本模組自己畫底）。
- Produces:
  - `struct MarqueeText: View`，`init(_ text: String, font: Font)`；`enum MarqueeLayout { static func needsScroll(textWidth: CGFloat, containerWidth: CGFloat) -> Bool }`——文字比容器寬才捲動，否則靜止置中（Review 所指極長歌名不得撐破版面）。
  - `struct CoverView: View`，`init(_ artwork: ArtworkRef?)`：置中裁切成方形，`data == nil` 或解不出圖時顯示音符佔位圖。
  - `struct EqualizerBars: View`，`init(isAnimating: Bool)`。
  - `struct LyricStripView: View`：底色 `Color.black.opacity(MusicModule.lyricStripOpacity)`，下緣圓角 10，內容為 `MarqueeText`；內容來自 `TimelineView(.periodic(from: .now, by: 0.25))` 以 `MusicPresentation.stripContent` 計算。
  - `MusicPreviewView`：上段（瀏海高度）左翼 `CoverView`、中間留空（瀏海）、右翼 `EqualizerBars`；左右翼貼齊條的兩端；下段在歌詞條可見時放 `LyricStripView`。**中央寬度必須用 `geometry.notchSize.width` 留空**，所以 `MusicModule` 需持有 `NotchGeometry`（由 `AppDelegate` 傳入 `makeDefault(geometry:)`）。
  - `MusicExpandedView`：左側 `CoverView`，右側曲名、演出者（nil 則省略）、進度條（`duration` 與錨點同時存在才顯示，否則整條不畫）、上一首／播放暫停／下一首三顆鈕（`control == nil` 時禁用）、歌詞條折疊開關（SF Symbol chevron，綁 `LyricStripPreference.isVisible`）；歌詞區：同步且位置可得 → 捲動並標示當前行，其餘 → 靜態全文，無歌詞 → 不顯示該區；`snapshot == nil` → 只顯示一行「目前沒有播放」，控制列隱藏。
  - `tabIcon()`：SF Symbol `music.note`。

- [ ] **Step 1: 寫失敗的測試** `MarqueeTextTests`：`testShortTextDoesNotScroll`、`testLongTextScrolls`、`testExactFitDoesNotScroll`、`testZeroWidthContainerDoesNotCrash`。
- [ ] **Step 2: 確認失敗**。
- [ ] **Step 3: 實作** 上述檔案，並讓 `MusicModule` 的三個畫面入口回傳真正的畫面。
- [ ] **Step 4: 確認通過**：`swift build && swift test`。
- [ ] **Step 5: 提交**：`git commit -m "Add Music Preview and Expanded views"`

---

## Task 14：歌詞可信度把關

使用者決定：動態歌詞寧可不顯示也不要顯示不準的（只需要「現在是哪一行」，不需要逐字）。LRCLIB／網易雲的結果除了歌名與長度比對，再多一道對歌詞本身的合理性檢查。

**Files:**
- Modify: `Sources/NotchCatKit/Modules/Music/Lyrics/Lyrics.swift`
- Modify: `Sources/NotchCatKit/Modules/Music/Lyrics/LyricsRepository.swift`
- Test: `Tests/NotchCatKitTests/Music/LyricsTests.swift`
- Test: `Tests/NotchCatKitTests/Music/LyricsRepositoryTests.swift`

**Interfaces:**
- Consumes: Task 9 `Lyrics`／`LyricLine`、Task 10 `LyricsRepository`、`TrackIdentity.duration`（選用）。
- Produces:
  - `Lyrics.durationTolerance: TimeInterval`（static，5 秒，具名常數）。
  - `func isPlausible(forDuration duration: TimeInterval?) -> Bool`：`.plain` 一律為 true（它不用於動態顯示）；`.synced` 須同時滿足 ① 行數 ≥ 2；② 當 `duration` 非 nil 且 > 0 時，最後一行的時間 ≤ `duration + durationTolerance`。

- [ ] **Step 1: 寫失敗的測試。** `LyricsTests`：`testSyncedWithTwoLinesWithinDurationIsPlausible`；`testSingleLineSyncedIsNotPlausible`（例如只有一行的佔位）；`testLastLineBeyondDurationPlusToleranceIsNotPlausible`；邊界 `testLastLineExactlyAtDurationPlusToleranceIsPlausible`（duration 200、最後一行 205 → true）與 `testLastLineJustPastToleranceIsNotPlausible`（205.1 → false）；`testUnknownOrZeroDurationOnlyAppliesLineCountRule`（nil 與 0）；`testPlainIsAlwaysPlausible`。`LyricsRepositoryTests`：`testImplausibleSyncedResultIsSkippedAndWalkContinues`（第一個來源回不可信的 synced、第二個回可信的 synced → 回第二個）；`testImplausibleSyncedDoesNotBecomeFallback`（只有不可信 synced 的來源 → 不被當成保底）；`testImplausibleSyncedThenLaterPlainReturnsPlain`；`testImplausibleSyncedCountsAsCleanAnswerForCaching`（沒有任何來源失敗時按 `negativeTTL` 快取為查無，而不是 `failureTTL`）；`testPlausibilityUsesTrackDuration`（同一份歌詞，曲長夠長時可信、曲長太短時不可信）。
- [ ] **Step 2: 確認失敗。**
- [ ] **Step 3: 實作** `Lyrics.isPlausible(forDuration:)`；`LyricsRepository` 走訪時，來源回傳 `.synced` 但 `!isPlausible(forDuration: track.duration)` 就當作「該來源乾淨地回答了、但沒有可用結果」（等同 `.empty`，不算 `failed`，不存入保底），繼續問下一個來源。
- [ ] **Step 4: 確認通過**：`swift build && swift test`，零警告。
- [ ] **Step 5: 提交**：`git commit -m "Reject implausible synced lyrics before they are shown"`

---

## Task 15：安裝模組、更新文件、實機驗收

**Files:**
- Modify: `Sources/NotchCatKit/AppDelegate.swift`
- Modify: `docs/HANDOFF.md`（Phase 2 一節更新為「已完成／驗收結果」，並記錄 perl 宿主的現況與風險）
- Modify: `README.md`（若仍有過時的狀態描述）

- [ ] **Step 1: 安裝模組。** 在 `AppDelegate.applicationDidFinishLaunching` 建立 `ModuleManager` 之後，`moduleManager.register(MusicModule.makeDefault(geometry: geometry))`。
- [ ] **Step 2: 建置並啟動。** `swift build && swift run`，確認啟動不崩潰、`libNotchCatMediaRemoteHelper.dylib` 在可執行檔同目錄、沒在播放時 Preview 是空黑條。
- [ ] **Step 3: 實機驗收（請使用者在有瀏海的 Mac 上逐項確認，回報結果）：**
  1. YouTube Music、YouTube、Spotify 各播一首：Preview 出現封面、音柱與歌詞條；Expanded 顯示曲名、演出者、進度條、封面。
  2. 切歌、暫停、繼續、拖動進度：畫面跟上；暫停時 Preview 回到空黑條。
  3. 歌詞條：展開／折疊動畫順暢；半透明底色的觀感；折疊後 hover 範圍正確。
  4. **hover 不閃爍**（Preview 加高後重新驗收，這是 Shell 先前最難的問題）。
  5. 上一首／播放暫停／下一首真的控制得動（`next`／`previous` 先前只驗證過指令碼）。
  6. 殺掉 perl 行程（`pkill -f NotchCatMediaRemoteHelper` 之類）後自動恢復；暫時改名動態庫後 App 不崩潰且退到視窗標題備援。
  7. 無網路時歌詞靜默消失、曲目顯示不受影響。
  8. 啟動時歌曲已播到一半：視窗標題備援情境下歌詞不得從頭捲動。
- [ ] **Step 4: 修正驗收中發現的問題**（每項一個小提交），並把歌詞條透明度與高度的實機調整結果寫回 `Global Constraints` 與 spec。
- [ ] **Step 5: 提交**：`git commit -m "Install the Music module and record hardware verification"`

---

## 自審紀錄

- **Spec 覆蓋：** 目標與範圍（1）→ Task 3、5、12、13；既有基礎與 Shell 契約（2、7.1）→ Task 1；實測發現（3）→ Task 4、5 的格式與可行性驗證；架構與邊界（4）→ Task 2、5；播放來源（5.1–5.5）→ Task 3、4、5、6、7、12；歌詞（6.1–6.4）→ Task 8、9、10、11；畫面（7.1–7.3）→ Task 12、13；播放控制（8）→ Task 2、13；錯誤處理（9）→ Task 5、10、12；測試策略（10）→ 各任務；風險（11）→ Task 5 Step 1／2、Task 15 Step 3；歌詞可信度把關（使用者追加）→ Task 14。
- **型別一致：** `NowPlayingSnapshot`／`PositionAnchor`／`ArtworkRef`（Task 3）被 Task 4、5、6、7、8、12 沿用；`TrackIdentity.duration` 全程為選用；`LyricsResolving`（Task 10）被 Task 12 使用；`MediaRemoteControlling`（Task 2）被 Task 12 使用；`previewExtraHeightPublisher`（Task 1）被 Task 12 實作。
- **已知尚未釘死的決策（留給實作時以實測定）：** 暫停時 helper 輸出的形狀、拖動進度是否產生新行、SwiftPM 動態庫的簽章（皆在 Task 5 Step 1／2 驗證並回寫 spec）；歌詞條的透明度與高度（Task 15 Step 4）。
