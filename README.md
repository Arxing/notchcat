# NotchCat

一個把 macOS 瀏海變成可互動面板的工具。架構規劃見 [`docs/PLANNING.md`](docs/PLANNING.md)。

## 目前進度：Phase 0（純骨架）

- 瀏海幾何偵測（`Shell/NotchGeometry.swift`）
- `NSPanel` 骨架、Hidden ⇄ Expanded ⇄ Preview 三態狀態機、hover 展開/收合動畫（`Shell/`）
- 內容目前是假資料（`UI/NotchContentView.swift`），還沒有接任何模組（Phase 1 才會有 `NotchModule`／`ModuleManager`）

## 執行環境需求

這個專案只能在 **macOS 14+、有實體瀏海的 Mac**（MacBook Pro 14"/16", 2021+）上執行與測試，因為用到 `NSScreen.auxiliaryTopLeftArea/auxiliaryTopRightArea`、`NSPanel` 等 macOS 專屬 API。

> **這份 Phase 0 程式碼是在沒有 macOS/Xcode 的環境下寫的，尚未實機編譯與執行過**，開起來後如果有編譯錯誤或行為不如預期（例如瀏海幾何算法有誤差），請回報，我再依實機結果修正。

## 如何執行（在 Mac 上）

這是一個 Swift Package，兩種方式都可以：

```bash
swift run
```

或用 Xcode 直接開啟資料夾（`File > Open...` 選這個資料夾，Xcode 會辨識 `Package.swift`），選 `NotchCat` scheme 執行。

執行後選單列會出現一顆貓咪圖示，可以從那裡 Quit；瀏海本身在滑鼠移過去時應該會展開成一個黑色圓角面板，移開後收合回瀏海大小。
