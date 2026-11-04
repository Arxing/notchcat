# NotchCat

一個把 macOS 瀏海變成可互動面板的工具。架構規劃見 [`docs/PLANNING.md`](docs/PLANNING.md)，目前進度與待驗收項目見 [`docs/HANDOFF.md`](docs/HANDOFF.md)。

## 目前狀態

- **Shell**：兩個模式——**Preview**（平時常駐的收合狀態，貼著瀏海）與 **Expanded**（滑鼠 hover 瀏海時展開）。Expanded 上方有模組切換用的 AppBar。
- **模組系統**：模組實作 `NotchModule` protocol，啟動時向 `ModuleManager` 註冊；Shell 只透過這個介面和模組溝通。
- **Music 模組**：已實作並安裝，等待使用者在實機上驗收（清單見 HANDOFF）。

### Music 模組

- **Preview**：播放中時，瀏海左右兩翼顯示封面與動態音柱，瀏海下方多一條半透明的歌詞條（可折疊，折疊狀態會記住）。沒在播放時 Preview 回到空黑條。
- **Expanded**：曲名、演出者、進度條、封面，以及上一首／播放暫停／下一首控制。
- **現正播放的來源**：讀取系統目前正在播放的內容，不限 App（YouTube Music、YouTube、Spotify 等）。
  - macOS 的 MediaRemote 讀取對第三方簽章的 App 是封閉的（NotchCat 主程序自己讀只會拿到空資料），所以改用 Apple 簽章的 `/usr/bin/perl` 當宿主，由它載入一個很小的 C 動態庫（`libNotchCatMediaRemoteHelper.dylib`，ad-hoc 簽章即可）來讀，再把變化以 JSON 行傳回主程序。主程序只用 MediaRemote 送播放控制指令（這部分不受限制）。
  - 若 perl 宿主不可用（動態庫不在、系統關掉這個空隙），退到**視窗標題備援**：用 `CGWindowList` 讀視窗標題。它目前只認得 YouTube Music（視窗擁有者為 `YouTube Music`，標題形如 `YouTube Music - 曲名 | YouTube Music`），只能得到曲名，沒有演出者、進度與封面，因此備援模式只顯示曲名，不會顯示歌詞。macOS 10.15 起，其他 App 的視窗標題通常要有「螢幕錄製」權限才讀得到，所以這個備援很可能需要該權限；沒有權限時讀不到標題，就什麼都不顯示。此點尚未實機確認。
- **歌詞**：依序向 LRCLIB、網易雲查詢，取第一個「可用」的結果。純文字歌詞只當保底；帶時間軸的結果還要通過合理性把關（至少 2 行、最後一行時間不得超過曲長加 5 秒），寧可不顯示也不顯示不準的。無網路或查不到時歌詞條靜默消失，不影響曲目顯示。

### 隱私

查歌詞時，正在播放曲目的曲名與演出者（包含 YouTube 影片或頻道的任意標題）會送到 lrclib.net 與 music.163.com；除此之外不會傳送任何資料。本專案僅供個人使用。

## 執行環境需求

macOS 14+、有實體瀏海的 Mac（MacBook Pro 14"/16", 2021+）。用到 `NSScreen.auxiliaryTopLeftArea/auxiliaryTopRightArea`、`NSPanel` 等 macOS 專屬 API。偵測不到瀏海時 App 會直接結束。

## 如何執行

這是一個 Swift Package。**請先 `swift build`，再執行建好的執行檔**——`swift run` 本身不會建出 `libNotchCatMediaRemoteHelper.dylib`，只有完整的 `swift build` 會。沒有這個動態庫時 App 仍能啟動，但只剩視窗標題備援（並在 log 印出一行說明動態庫找不到）。

最簡單的做法：

```bash
scripts/run.sh
```

它等同於 `swift build && exec .build/debug/NotchCat`。也可以用 Xcode 開啟資料夾，選 `NotchCat` scheme 執行（同樣要確保動態庫已建出且與執行檔在同一目錄）。

執行後選單列會出現一顆貓咪圖示，可以從那裡 Quit；滑鼠移到瀏海上會展開成黑色圓角面板，移開後收合回 Preview。測試用 `swift test`。

## 注意事項

- **perl 宿主這個做法很脆弱。** 它靠的是 Apple 簽章行程的空隙，不是正式開放的介面，任何一次系統更新都可能把它關掉；關掉時 App 會退到視窗標題備援。
- **僅供個人使用。** 這個專案使用私有 API，不適合上架 Mac App Store，也不建議當作對外發布的產品。
- 歌詞來自第三方服務，內容的版權屬於原權利人。
