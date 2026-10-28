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

1. **瀏海幾何算法**（`NotchGeometry.swift`）：用 `screen.frame.width - left.width - right.width` 算瀏海寬度、用 `left.height` 當瀏海高度。這是社群常見手法，但沒有在真機上量過，實際瀏海形狀（圓角、精確 padding）跟算出來的矩形可能有落差，需要拿真機數字校正。
2. ~~**Hidden 狀態的 hover 觸發區域**就是瀏海本身大小（很小）~~ — 已在真機驗證並修正（2026-09-22）。原本的作法是面板本身的 frame 隨動畫縮放、同時兼任 hover 判定範圍，真機測出兩個問題：(a) 動畫進行中滑鼠移動速度一旦超過畫面還沒長到的邊界就會誤判離開，造成展開/收合狂閃；(b) Hidden 狀態只有 ~32pt 高，滑鼠「靜止不動」時的自然晃動就足以跨出邊界，造成靜止時仍會閃爍。修法：面板固定成所有模式中最大的尺寸（`NotchMode.containerFrame`），永遠不再隨動畫 resize；hover 判定範圍改由 `NotchHostingView.hoverRect` 手動控制——進入 Hidden 小膠囊才觸發展開（維持精準），一展開就立刻放大成整個固定範圍（避免抖動誤判離開）。視覺動畫則改用 SwiftUI 的 `@ObservedObject`/`.animation(value:)` 驅動（見 `NotchShellState`），不再由 AppKit 端手動 resize 視窗或覆寫 `rootView`——後者會讓 `withAnimation` 失效。細節見 `Sources/NotchCat/Shell/NotchShellController.swift` 和 `NotchMode.swift` 的註解。
3. `NSMenuItem` 的 target 有手動設成 `self`（`main.swift`），照理沒問題，但沒跑過。
4. 沒有處理「機種沒有瀏海」以外的 fallback（目前直接 `NSApp.terminate`），也沒有處理外接螢幕/多螢幕情境的邊界情況（目前假設瀏海永遠在內建螢幕，邏輯上抓第一個有 `auxiliaryTopLeftArea` 的 screen，多螢幕下沒特別測試過）。

## 接手後的建議順序

1. `swift run`（或 Xcode 開起來跑），先看能不能編譯過，抓 Swift/API 用法上的錯誤。
2. 跑起來後對照真實瀏海，校正 `NotchGeometry` 的計算。
3. 手動測試 hover 進出的展開/收合動畫，調整觸發區域大小、動畫時長/曲線的手感。
4. Phase 0 驗收過關後，照 `docs/PLANNING.md` 的 Phase 1 開始：`NotchModule` protocol + `ModuleManager`，先用一個「時鐘模組」驗證安裝機制（見 PLANNING.md 第 4、8 節）。

## 下一步：Phase 2（Music 模組）—— MediaRemote 可行性已實測

> **更正（2026-10-14 當日稍晚）：本節原先寫著「疑慮沒有成真、MediaRemote 可以照原案走」，那個結論是錯的，已於同日推翻。詳見下方「實測更正」。**

### 實測更正：MediaRemote 的 now-playing 讀取對第三方 App 不可用

原本擔心 Apple 在 **macOS 15.4** 收緊 now-playing 存取後，未取得 entitlement 的 App 會「呼叫成功但拿到空資料」。**這個疑慮確實成真了。**

**先前誤判的原因：** 當天所有可行性探針都是用 `swift <檔名>.swift` 執行的，而那會跑在 Apple 簽章的直譯器底下並繼承它的存取權限。探針從來沒有在「App 實際出貨的形態」下測過。

**更正後的實測（macOS 15.7，同一台機器、同一段程式碼、同一時刻）：**

| 宿主行程 | 拿到的 now-playing 鍵數 |
|---|---|
| Apple 簽章的 `swift` 直譯器 | 13 |
| ad-hoc 簽章的裸執行檔 | 0 |
| ad-hoc 簽章、含 bundle id 的 .app | 0 |
| 以 Apple Development 憑證簽章的 .app | 0 |
| 以 Apple Distribution 憑證簽章的 .app | 0 |

**結論：這道關卡擋的是簽章授權，不是打包方式，而且任何第三方簽章身分都打不開。** 正式的開發者憑證與發布憑證一樣拿到空字典。因此 PLANNING.md 第 5 節以 MediaRemote 為資料來源的 Music 模組，在 macOS 15.4 以後的系統上**對出貨的 App 無效**。

`MRMediaRemoteSendCommand`（播放控制）則**不受此限制，實測可用**——上一首／暫停／下一首送得出去也會生效。換句話說「控制得了，但讀不到在播什麼」。

**附帶仍然成立的發現**（在直譯器下觀察到，對任何未來的資料來源設計仍有參考價值）：封面圖的實際位元組不是每次查詢都會附上，三次中只出現一次，所以任何實作都需要以識別碼為鍵自行快取。

**尚待決定：** Phase 2 要改走哪條路。AppleScript／ScriptingBridge 只涵蓋有 scripting 介面的原生 App（大致只有 Apple Music、Spotify），**涵蓋不到瀏覽器播放**，而瀏覽器正是本專案使用者的實際使用方式。

**後續決定：** Phase 2 的資料來源已改為讀取視窗標題（`CGWindowList`），播放控制仍走 `MRMediaRemoteSendCommand`。細節見 `docs/superpowers/specs/2026-10-14-music-module-design.md`。

## 待決事項（PLANNING.md 裡列過，還沒有答案）

- 最低支援的 macOS 版本（目前程式碼寫死 14，PLANNING.md 討論過 13 vs 14 的取捨）。
- v1 要不要一開始就做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單。
- Music 模組的歌詞來源怎麼拿（純顯示系統提供的資訊，或額外串接歌詞服務）——這個要到 Phase 2 才會遇到，但可以先想。
