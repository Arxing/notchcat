# 轉交文件（web session → 本機 Claude Code）

寫於 2026-09-22，接手前請先讀這份，再讀 `docs/PLANNING.md`（完整架構規劃）。

## 這個 session 是在什麼環境下做的

之前的工作是在**雲端 Linux sandbox** 裡做的，**沒有 Swift 工具鏈、沒有 Xcode**，AppKit/SwiftUI/`NSScreen`/`NSPanel` 這些 API 本來就只存在 macOS。

**結論：`Sources/NotchCat/` 底下所有程式碼從來沒有被編譯或執行過一次。** 這是接手後第一件事——在真的 Mac（有實體瀏海的機種）上跑起來，抓編譯錯誤跟實際行為落差。

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
2. **Hidden 狀態的 hover 觸發區域**就是瀏海本身大小（很小），滑鼠要準確移到瀏海上才會觸發展開。如果真機測起來太難 hover 到，考慮把觸發區域做得比視覺上的 Hidden 形狀大一點（padding），但視覺呈現仍維持貼合瀏海大小——這是設計取捨,目前沒做。
3. `NSMenuItem` 的 target 有手動設成 `self`（`main.swift`），照理沒問題，但沒跑過。
4. 沒有處理「機種沒有瀏海」以外的 fallback（目前直接 `NSApp.terminate`），也沒有處理外接螢幕/多螢幕情境的邊界情況（目前假設瀏海永遠在內建螢幕，邏輯上抓第一個有 `auxiliaryTopLeftArea` 的 screen，多螢幕下沒特別測試過）。

## 接手後的建議順序

1. `swift run`（或 Xcode 開起來跑），先看能不能編譯過，抓 Swift/API 用法上的錯誤。
2. 跑起來後對照真實瀏海，校正 `NotchGeometry` 的計算。
3. 手動測試 hover 進出的展開/收合動畫，調整觸發區域大小、動畫時長/曲線的手感。
4. Phase 0 驗收過關後，照 `docs/PLANNING.md` 的 Phase 1 開始：`NotchModule` protocol + `ModuleManager`，先用一個「時鐘模組」驗證安裝機制（見 PLANNING.md 第 4、8 節）。

## 待決事項（PLANNING.md 裡列過，還沒有答案）

- 最低支援的 macOS 版本（目前程式碼寫死 14，PLANNING.md 討論過 13 vs 14 的取捨）。
- v1 要不要一開始就做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單。
- Music 模組的歌詞來源怎麼拿（純顯示系統提供的資訊，或額外串接歌詞服務）——這個要到 Phase 2 才會遇到，但可以先想。
