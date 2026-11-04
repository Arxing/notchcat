# 轉交文件

最後更新：2026-11-04。接手前先讀這份，再讀 `docs/PLANNING.md`（整體架構）。

## 目前狀態

- Branch：`feature/music-module`。Shell（Preview／Expanded 兩個模式）、模組系統與 Music 模組都已實作，Music 模組已在 `AppDelegate` 安裝，`swift build`、`swift test` 通過，啟動煙霧測試正常（App 啟動不崩潰、結束後沒有殘留的 perl 行程）。
- **等待使用者在有瀏海的 Mac 上做實機驗收**，清單見下方「尚未驗證」。驗收時請在實機上實際播放音樂；發現的問題一項一個小提交。
- 專案型態：Swift Package，macOS 14+。執行方式見 `README.md`：務必 `swift build` 之後才執行，`scripts/run.sh` 已包好（`swift run` 單獨不會建出 helper 動態庫）。

## 架構與文件指標

- 整體架構：`docs/PLANNING.md`。
- Shell 與模組介面：`docs/superpowers/specs/2026-09-23-module-interface-design.md`、`docs/superpowers/plans/2026-09-30-module-interface.md`。
- Music 模組：`docs/superpowers/specs/2026-10-14-music-module-design.md`（spec，含實測依據）、`docs/superpowers/plans/2026-11-04-music-module.md`（實作計畫）。

```
Sources/NotchCat/main.swift             # 三行進入點
Sources/NotchCatKit/
  AppDelegate.swift                     # 偵測瀏海、建立 ModuleManager、註冊 MusicModule、建立 Shell、選單列 Quit
  Shell/                                # NotchGeometry、NotchMode（Preview/Expanded）、NotchPanel、NotchHostingView、NotchShellController
  ModuleKit/                            # NotchModule protocol、ModuleManager
  UI/                                   # Shell 的 SwiftUI 內容與 AppBar
  Modules/Music/
    NowPlaying/                         # 來源（MediaRemoteSource＋perl 宿主、WindowTitleSource）、NowPlayingMonitor、MediaRemoteControl
    Lyrics/                             # TrackIdentity／TrackNormalizer、LyricsRepository、LRCLIB 與網易雲 provider
    Views/                              # Preview（兩翼＋歌詞條）、Expanded、音柱、封面
Sources/NotchCatMediaRemoteHelper/      # C，編成動態庫，只在 perl 行程內執行
scripts/run.sh                          # swift build && 執行 .build/debug/NotchCat
```

私有 API 只准出現在兩處：`MediaRemoteControl.swift`（只送控制指令）與 `NotchCatMediaRemoteHelper`（讀取，只在 perl 行程內）。

## 實測發現（macOS 15.7，精簡版；完整表格見 Music spec 第 3 節）

- MediaRemote 的 now-playing **讀取**對第三方簽章的主程序封閉：ad-hoc、Apple Development、Apple Distribution 簽章的執行檔與 .app 都只拿到空字典；Apple 簽章的直譯器則讀得到。擋的是簽章授權，不是打包方式。
- 以 Apple 簽章的 `/usr/bin/perl` 當宿主、載入 ad-hoc 簽章的 C 動態庫，可以讀到完整資料（曲名、演出者、總長度、位置、速率、封面）。這是 Apple 簽章宿主的空隙，不是正式介面，系統更新隨時可能關掉；關掉時退到視窗標題備援。
- 播放**控制**（`MRMediaRemoteSendCommand`）不受讀取封閉影響，實測可用。
- helper 行程的記憶體會成長，但上限受 MediaRemote 內部約 400 秒的逾時所限制，實測在約 416 個來源處進入平穩，不是無限成長。
- `ElapsedTime` 只是 `Timestamp` 那一刻的快照，目前位置要用 `elapsed + (now − timestamp) × rate` 算；封面位元組不是每次都附，要以識別碼為鍵自行快取。
- 視窗標題不需要任何權限即可讀（YouTube Music PWA 為 `YouTube Music - 曲名 | YouTube Music 🔊` 形式），但沒有進度、長度與封面。

## 尚未驗證（請使用者在實機上逐項確認）

1. YouTube Music、YouTube、Spotify 各播一首：Preview 出現封面、音柱與歌詞條；Expanded 顯示曲名、演出者、進度條、封面。**Spotify 完全未驗證**（開發機沒安裝）。
2. 暫停時 helper 輸出的形狀（Preview 是否正確回到空黑條）。
3. 拖動進度時 helper 是否會輸出新的一行（畫面是否跟上）。
4. 視窗標題備援的格式：YouTube Music PWA 以外的情境，以及備援是否真的不需要「螢幕錄製」權限。
5. hover 不閃爍：Preview 加高後重新驗收（這是 Shell 先前最難的問題）；特別留意歌詞條出現或折疊使 Preview 高度改變時，游標靜止不動是否會產生多餘的 `mouseEntered`。
6. `.scrollIndicators(.hidden)` 在實際 App 裡是否生效。
7. 歌詞條底色透明度 0.6、高度 24pt 的觀感；展開／折疊動畫與折疊後的 hover 範圍。
8. 上一首／播放暫停／下一首（尤其 `next`、`previous`）真的控制得動。
9. 殺掉 perl 行程（`pkill -f NotchCatMediaRemoteHelper`）後自動恢復；暫時改名動態庫後 App 不崩潰並退到視窗標題備援（已知會在 log 印一行 dylib not found，只印一次）。
10. 無網路時歌詞靜默消失、曲目顯示不受影響。
11. 啟動時歌曲已播到一半（視窗標題備援情境）：歌詞不得從頭捲動。
12. helper 的 CPU 與記憶體在長時間執行下的表現。
13. 音柱（Equalizer）動畫的 CPU 成本。
14. Expanded 的 AppBar 實際高度約 15pt（`MusicExpandedView` 的註解寫約 20pt），版面預算是 149pt 而非 144pt，請目視確認沒有擠壓或裁切。
15. 歌詞條額外高度的 0.28 秒動畫期間，Preview 的封面是從零尺寸長出來的，請確認觀感可接受。
16. `.scrollIndicators(.hidden)` 也要在「系統設定 > 外觀 > 顯示捲動條：永遠」下檢查。

驗收後把歌詞條透明度與高度的調整結果寫回 Music spec 與計畫的 Global Constraints。

## 已知風險

- perl 宿主的做法脆弱、僅供個人使用，不可當成可對外發布的方案。
- 瀏海幾何算法（`NotchGeometry.swift`）用 `screen.frame.width − left.width − right.width` 算寬度；已在真機上跑過，但沒有針對不同機型量測。
- 沒有瀏海的 Mac 與多螢幕情境沒有 fallback（目前沒偵測到瀏海就結束）。
- 歌詞命中率只測過同一位演出者的兩首曲目，不足以代表整體。

## 下一輪設計的輸入（歌詞可信度與本地來源）

以下是使用者對下一個階段的決定，尚未動工：

- **本地歌詞來源排在 provider 清單的第一位**，先於 LRCLIB 與網易雲。
- **可編輯第三方結果來覆寫**：編輯後它就變成一筆本地條目。
- 每首歌可調 **±offset 微調**，並有「這份歌詞是錯的」**黑名單**。
- 準確度只要求**行級**（目前是哪一行），不做逐字。
- **準確度優先**：寧可不顯示，也不顯示不準的。
- **AI agent 整合**：用一個本地歌詞資料夾——放 `.lrc` 檔，加上一份「缺少可信同步歌詞的曲目」待辦清單；任何 agent 都能寫檔，App 監看這個資料夾。
- **遠端分享**：透過一個公開的 GitHub repo。
- 本地歌詞庫用 **git 做版本控管**。
- 公開分享前要留意**版權**問題（歌詞內容屬原權利人）。

## 待決事項

- v1 要不要做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單（目前寫死）。
- 多模組搶 Preview 的排序體驗（Phase 3）。
