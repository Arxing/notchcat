# 轉交文件（web session → 本機 Claude Code）

寫於 2026-09-22，接手前請先讀這份，再讀 `docs/PLANNING.md`（完整架構規劃）。

## 這個 session 是在什麼環境下做的

之前的工作是在**雲端 Linux sandbox** 裡做的，**沒有 Swift 工具鏈、沒有 Xcode**，AppKit/SwiftUI/`NSScreen`/`NSPanel` 這些 API 本來就只存在 macOS。

**結論：`Sources/NotchCat/` 底下所有程式碼從來沒有被編譯或執行過一次。** 這是接手後第一件事——在真的 Mac（有實體瀏海的機種）上跑起來，抓編譯錯誤跟實際行為落差。

> **更新（2026-09-22，本機 Claude Code）：** 已在真機上編譯 + 執行過，`swift build`/`swift run` 都過。真機測出 hover 動畫閃爍的 bug 並修正，細節見下方「已知風險」第 2 點。

## 目前狀態

- Branch：`claude/lucid-archimedes-nntl9p`，已 push 到 `origin`。
- 兩個 commit：
  1. `8aff9ab` — `docs/PLANNING.md`，完整架構規劃（三種模式定義、狀態機、Module 介面草案、技術選型、分階段路線圖）。
  2. `b0d939a` — Phase 0 骨架程式碼。
- 專案型態：Swift Package（`Package.swift`），非 `.xcodeproj`。macOS 14+，可以直接用 Xcode 開資料夾，或 `swift run`。

## 檔案地圖

```
Package.swift                              # SPM manifest, macOS 14+, 單一 executable target
README.md                                  # 執行方式 + 「尚未實機驗證」警語
docs/PLANNING.md                           # 完整架構規劃（先讀這個）
docs/HANDOFF.md                            # 本文件
Sources/NotchCat/
  main.swift                               # AppDelegate、accessory activation policy、選單列 Quit 圖示
  Shell/
    NotchGeometry.swift                    # 瀏海幾何偵測：用 auxiliaryTopLeftArea/RightArea 反推瀏海矩形
    NotchMode.swift                        # Hidden/Expanded/Preview enum + 各自的 size/frame 計算
    NotchPanel.swift                       # NSPanel 子類：borderless、non-activating、跨 Space、不進 Dock
    NotchHostingView.swift                 # NSHostingView 子類，把 hover 轉成 closure
    NotchShellController.swift             # 狀態機本體：hover 進出觸發 transition，NSAnimationContext 做 frame 動畫
  UI/
    NotchContentView.swift                 # 假內容（SwiftUI），純粹驗證三種形狀/動畫用
```

## 已知風險 / 沒把握的地方（優先驗證清單）

1. ~~**瀏海幾何算法**~~ — 已在真機驗證（2026-10-07）：正常，跟實體瀏海貼合沒有落差。
2. ~~**Hidden 狀態的 hover 觸發區域**就是瀏海本身大小（很小）~~ — 已在真機驗證並修正（2026-09-22）。原本的作法是面板本身的 frame 隨動畫縮放、同時兼任 hover 判定範圍，真機測出兩個問題：(a) 動畫進行中滑鼠移動速度一旦超過畫面還沒長到的邊界就會誤判離開，造成展開/收合狂閃；(b) Hidden 狀態只有 ~32pt 高，滑鼠「靜止不動」時的自然晃動就足以跨出邊界，造成靜止時仍會閃爍。修法：面板固定成所有模式中最大的尺寸（`NotchMode.containerFrame`），永遠不再隨動畫 resize；hover 判定範圍改由 `NotchHostingView.hoverRect` 手動控制——進入 Hidden 小膠囊才觸發展開（維持精準），一展開就立刻放大成整個固定範圍（避免抖動誤判離開）。視覺動畫則改用 SwiftUI 的 `@ObservedObject`/`.animation(value:)` 驅動（見 `NotchShellState`），不再由 AppKit 端手動 resize 視窗或覆寫 `rootView`——後者會讓 `withAnimation` 失效。細節見 `Sources/NotchCat/Shell/NotchShellController.swift` 和 `NotchMode.swift` 的註解。
3. ~~`NSMenuItem` 的 target 有手動設成 `self`~~ — 已在真機驗證（2026-10-07）：Quit 選單項目正常觸發。
4. **「機種沒有瀏海」以外的 fallback、外接螢幕/多螢幕情境**——目前無瀏海直接 `NSApp.terminate`，多螢幕下抓第一個有 `auxiliaryTopLeftArea` 的 screen，沒有特別處理過邊界情況。**決定暫緩**：等實際遇到需求再處理，不列為 Phase 0 驗收項目。

## 接手後的建議順序

1. `swift run`（或 Xcode 開起來跑），先看能不能編譯過，抓 Swift/API 用法上的錯誤。
2. 跑起來後對照真實瀏海，校正 `NotchGeometry` 的計算。
3. 手動測試 hover 進出的展開/收合動畫，調整觸發區域大小、動畫時長/曲線的手感。
4. ~~Phase 0 驗收過關後，照 `docs/PLANNING.md` 的 Phase 1 開始：`NotchModule` protocol + `ModuleManager`，先用一個「時鐘模組」驗證安裝機制（見 PLANNING.md 第 4、8 節）。~~ — 已完成並於真機驗收（2026-10-14）：hover 展開正常，Expanded 會顯示時鐘。過程中抓到一個編譯錯誤（`ModuleManager` 少 import `Foundation`），已修正。

## 下一步：Phase 2（Music 模組）—— MediaRemote 可行性已實測

原本擔心 Apple 在 **macOS 15.4** 收緊 now-playing 存取後，未取得 entitlement 的 App 會「呼叫成功但拿到空資料」。**已於 2026-10-14 在 macOS 15.7 實測，這個疑慮沒有成真**，PLANNING.md 第 5 節的 MediaRemote 路線可以照原案走。

實測結果（丟棄式探針，呼叫 `MRMediaRemoteGetNowPlayingInfo`）：

- **拿得到真實資料**，一包 13 個鍵，包含 `Title`／`Artist`／`Album`／`Duration`／`ElapsedTime`／`PlaybackRate`／`Timestamp`，足夠做 Preview 與 Expanded 的主要內容。
- `MRMediaRemoteGetNowPlayingApplicationIsPlaying` 也正常回報播放狀態，可直接拿來餵 `isActivePublisher`。
- **封面圖要特別處理：`kMRMediaRemoteNowPlayingInfoArtworkData` 不是每次都會附上。** 連續查詢三次，前兩次只有 `ArtworkDataWidth`/`Height`/`MIMEType`/`ArtworkIdentifier` 這些 metadata，第三次才出現實際位元組。實作時不能假設每次輪詢都拿得到圖，要**用 `ArtworkIdentifier` 當 key 自行快取**，拿到一次就留著，沒拿到就沿用舊的。
- **涵蓋範圍比預期好**：實測當下在播的是一個 Chrome PWA（網頁版音樂服務），MediaRemote 照樣抓得到。這是 AppleScript/ScriptingBridge 路線做不到的——那條路只能涵蓋有 scripting 介面的原生 App（大致只有 Apple Music、Spotify）。**這點是選 MediaRemote 而非 AppleScript 的決定性理由。**

仍未驗證、留到設計階段再處理的部分：歌詞來源（PLANNING.md 第 9 節的待決事項，MediaRemote 本身不提供歌詞）。

## 待決事項（PLANNING.md 裡列過，還沒有答案）

- 最低支援的 macOS 版本（目前程式碼寫死 14，PLANNING.md 討論過 13 vs 14 的取捨）。
- v1 要不要一開始就做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單。
- Music 模組的歌詞來源怎麼拿（純顯示系統提供的資訊，或額外串接歌詞服務）——這個要到 Phase 2 才會遇到，但可以先想。
