# Phase 2：Music 模組設計

寫於 2026-10-14。上游文件：[`docs/PLANNING.md`](../../PLANNING.md)（整體架構）、[`docs/HANDOFF.md`](../../HANDOFF.md)（階段進度與實測紀錄）。

## 1. 目標與範圍

在 NotchCat 的瀏海面板上顯示「現正播放」與「動態歌詞」，作為 Phase 1 模組系統之後的第一個真實模組。

本模組由兩個互相獨立的部分組成：

- **播放來源：** 讀取**系統目前正在播放的內容**，不論是 YouTube Music、YouTube、Spotify 或其他 App，只要系統有在播就要抓得到。它回答「現在在播什麼、播到哪」。
- **歌詞來源：** 一份有順序的歌詞提供者清單，一個一個查，查到可用的歌詞就顯示。它回答「這首歌的歌詞是什麼」。

兩部分之間只靠一個值型別（`TrackIdentity`）溝通，任一邊的增減與替換都不影響另一邊。

**範圍內：**

- 讀取系統現正播放的曲名、演出者、專輯、總長度、播放位置、播放狀態與封面（來源提供什麼就用什麼，沒有的欄位一律允許為 nil，畫面不得因此破版）。
- 依播放狀態決定 Music 是否接管 Preview：播放中左右兩翼顯示封面與音柱，瀏海下方多一條歌詞條（使用者可折疊）；沒在播放時 Preview 回到 Shell 預設的空黑條。
- 取得逐行同步歌詞，依播放位置捲動。
- Expanded 下的播放控制（上一首／播放暫停／下一首）。

**已於 2026-10-28 對齊兩態 Shell。** Shell 只有 Preview 與 Expanded 兩個模式，Expanded 上方有模組切換 AppBar。

**播放來源已於 2026-11-04 重新定案。** 先前（2026-10-21）因為系統現正播放的讀取對第三方 App 封閉，退而只讀視窗標題、只支援 YouTube Music，進度條與封面也因此移出範圍。2026-11-04 實測發現以系統內建的 `/usr/bin/perl` 當宿主可以讀到完整資料（第 3 節第 2 點），因此改為「系統現正播放」為主要來源，視窗標題降為備援，進度條與封面回到範圍內。

**範圍外：** 多模組搶 Preview 的排序體驗（Phase 3）、模組開關設定 UI（Phase 3）、簽章與發布（Phase 4）。

**明確的決策：Music 模組與歌詞不拆成兩個階段**，兩者在同一份 spec、同一輪實作內完成。此決定已知的風險是歌詞的外部不確定性會直接落在主線上；接受這個風險的理由是畫面佈局可以一次定案，不必為了之後塞歌詞回頭重改 Preview 與 Expanded。

## 2. 既有基礎

兩態 Shell 與模組系統已完成並通過真機驗收，本模組架在上面。**唯一需要動 Shell 的地方是第 7.1 節的「Preview 額外高度」**，其餘不改：

- 實作 `NotchModule` protocol（`ModuleKit/NotchModule.swift`），向 `ModuleManager` 以 `register(_:)` 註冊。
- `isActivePublisher` 回報「是否正在播放」，`ModuleManager` 據此算出 `activeModule`：Music active 時接管 Preview；不 active 時 Preview 顯示 Shell 預設的空黑條。這條線已經打通並驗證過，本模組只需要正確餵值。
- 三個畫面入口：`tabIcon()`（Expanded 的 AppBar 上的圖示）、`previewView()`、`expandedView()`。
- Expanded 每次展開，AppBar 的選取會重設成當下的 `activeModule`；Music 沒在播放時沒有 active 模組，使用者可以自己點 AppBar 上的 Music 圖示進入它的 Expanded。

## 3. 設計所依據的實測結果

以下全部來自 macOS 15.7 上的實測，不是推測。設計中幾個不直覺的決定都源自這裡。

1. **系統現正播放的讀取（MediaRemote）對第三方簽章的 App 封閉。** 以六種宿主組態同時重測，同一台機器、同一段程式碼：

   | 宿主行程 | 取得的鍵數 |
   |---|---|
   | Apple 簽章的 `swift` 直譯器 | 13 |
   | ad-hoc 簽章的裸執行檔 | 0 |
   | ad-hoc 簽章、含 bundle id 的 .app | 0 |
   | Apple Development 憑證簽章的 .app | 0 |
   | Apple Distribution 憑證簽章的 .app | 0 |
   | 同上，經 LaunchServices 正常啟動 | 0 |

   關卡擋的是簽章授權，不是打包方式，任何第三方身分都打不開。所以 NotchCat 的主程序本身**不能**直接讀。

2. **以 Apple 簽章的 `/usr/bin/perl` 當宿主可以讀到完整資料（2026-11-04）。** 做法：寫一個很小的 C 動態庫，只呼叫 `MRMediaRemoteGetNowPlayingInfo`，用 ad-hoc 簽章，再由 `/usr/bin/perl` 以 `DynaLoader` 載入並呼叫。同一時刻對照：

   | 宿主 | 鍵數 |
   |---|---|
   | `/usr/bin/perl` 載入 ad-hoc 簽章的 dylib | 13～14 |
   | 同一段程式編成一般執行檔 | 0 |

   已驗證的來源：一首歌（播放器未記錄）與瀏覽器裡播放的 YouTube 影片，皆讀到曲名、演出者、總長度、播放位置、播放速率與封面資訊。**Spotify 尚未驗證**，視為預期可行但未證實。dylib 只需 ad-hoc 簽章，不需要開發者憑證。探針留在 session 暫存目錄，不屬於專案。

   這靠的是 Apple 簽章宿主的空隙，不是正式開放的介面，系統更新隨時可能關掉（第 11 節）。

3. **此管道回傳的欄位形狀（兩次實測，均在 `/usr/bin/perl` 宿主下）：**

   - `Title`、`Artist`、`Album`、`Duration`、`ElapsedTime`、`Timestamp`、`PlaybackRate`、`ArtworkIdentifier`、`ArtworkMIMEType`、`ContentItemIdentifier`、`ArtworkData*`。
   - **`ElapsedTime` 是「在 `Timestamp` 那一刻的位置」的快照，不會自己往前走。** 目前位置必須算：`ElapsedTime + (現在 − Timestamp) × PlaybackRate`。
   - **封面位元組不是每次都附。** 同一首歌連續兩次查詢，一次只有識別碼，一次附了 7751 bytes 的 JPEG。必須以 `ArtworkIdentifier` 為鍵自行快取。識別碼格式不固定（一次是雜湊字串，一次是 UUID），只能當鍵、不可解析。
   - **封面尺寸與比例不固定。** 同一首歌的宣告尺寸是 768×768、實際位元組是 120×120；YouTube 影片給的是 150×83 的 16:9 縮圖。畫面一律置中裁切成方形顯示。
   - **YouTube 的欄位語意不同於音樂 App。** `Title` 是整個影片標題（例如「蘇裔非 - 抱我!  ft. 莊岱諺」），`Artist` 是頻道名稱，`Album` 是空字串。拿去查歌詞前要先清理（第 6.3 節）。

4. **播放控制（`MRMediaRemoteSendCommand`）不受讀取封閉的影響，實測可用。** 見第 8 節。

5. **視窗標題在開發機上可讀。** 以 `CGWindowListCopyWindowInfo` 從編譯後的執行檔即可取得各視窗標題。這次量測的環境可能已經有「螢幕錄製」權限（macOS 10.15 起其他 App 的視窗標題通常要有該權限），「不需要權限」未經確認；沒有權限時讀不到標題，備援就什麼都不顯示。開發機上 YouTube Music 這個 Chrome PWA 的視窗標題形如：

   ```
   擁有者：YouTube Music
   標題：  YouTube Music - 假日而已 | YouTube Music 🔊
   ```

   瀏覽器會在正在發聲的視窗標題附加喇叭符號，可作為播放中的判斷依據。標題只在換曲時變動，**不含已播秒數、總長度或播放速率**，也沒有封面。它現在是備援來源，用於第 2 點的管道失效時。

6. **開發機未安裝 Apple Music 與 Spotify。** Spotify 的相容性只能靠使用者在自己機器上驗收。

7. **歌詞來源的精確比對很脆。** 以一位中文獨立演出者的兩首曲目實測 LRCLIB：其中一首用「歌名＋演出者＋專輯＋長度」精確查詢**沒中**，退回模糊搜尋才找到，原因是系統回報的歌名不含資料庫裡的 `(feat. …)` 後綴。

8. **同一首曲目在不同歌詞來源的品質不同。** 同樣那首歌，LRCLIB 只有純文字，網易雲有 82 行逐行時間軸。**這是第 6.2 節「純文字不算數，繼續找同步版」的直接證據。**

9. **簡繁體不一致。** 網易雲是以簡體曲名比對到我們送出的繁體歌名。比對驗證邏輯因此不得使用字串完全相等。

**樣本侷限：** 歌詞命中率只測了同一位演出者、同一張專輯的兩首曲目，不足以代表整體命中率，僅足以證明上述機制性問題存在。現正播放管道只在兩種來源上驗證過（見第 2 點）。視窗標題格式只在 YouTube Music 這一個 PWA 上驗證過。

## 4. 架構總覽

```
Sources/NotchCatMediaRemoteHelper/   # C target，編成動態庫，只在 perl 宿主內執行
  helper.c                           # 呼叫私有的讀取 API，把變化以 JSON 行輸出

Sources/NotchCatKit/Modules/Music/
  MusicModule.swift             # 實作 NotchModule，組裝下列元件
  NowPlaying/
    NowPlayingSource.swift      # 「什麼在播」的來源抽象介面
    NowPlayingSnapshot.swift    # 一次讀取的正規化結果（值型別）
    MediaRemoteSource.swift     # 啟動並監管 perl 宿主行程，讀它輸出的 JSON 行
    HelperMessageParser.swift   # JSON 行 → Snapshot（純函式，完全可測）
    WindowTitleSource.swift     # 備援：CGWindowList 讀視窗標題
    WindowTitleParser.swift     # 標題字串 → 曲目（純函式，完全可測）
    NowPlayingMonitor.swift     # 依序詢問來源、偵測換曲、驅動 @Published
    PlaybackEstimator.swift     # 僅供沒有位置資訊的來源：由「觀察到換曲的時刻」推算
    ArtworkCache.swift          # 以 ArtworkIdentifier 為鍵的記憶體快取
    MediaRemoteControl.swift    # 私有 API 只在這個檔案出現，且只用於送控制指令
  Lyrics/
    TrackIdentity.swift         # 查詢用的正規化曲目識別
    TrackNormalizer.swift       # 標題／演出者清理（含 YouTube 影片標題）
    Lyrics.swift                # synced / plain 兩種歌詞的表示
    LyricsProvider.swift        # 歌詞來源抽象介面
    LyricsRepository.swift      # 依序嘗試 + 快取
    Providers/
      LRCLIBProvider.swift
      NetEaseProvider.swift
  LyricStripPreference.swift    # 歌詞條折疊狀態（UserDefaults，可注入）
  Views/
    MusicPreviewView.swift      # 兩翼 + 歌詞條
    LyricStripView.swift        # 歌詞條本體（半透明底、跑馬燈）
    EqualizerBars.swift         # 右翼的動態音柱
    MusicExpandedView.swift
```

Shell 側的對應改動（第 7.1 節）在 `ModuleKit/NotchModule.swift`、`ModuleKit/ModuleManager.swift`、`Shell/NotchMode.swift`、`Shell/NotchShellController.swift`、`UI/NotchContentView.swift`。

**關鍵邊界一：私有 API 只准出現在兩處。** 一是 `MediaRemoteControl.swift`，只用於送出控制指令；二是 `NotchCatMediaRemoteHelper` 這個 C target，用於讀取，而且只會在 Apple 簽章的 perl 行程內執行。**NotchCat 的 Swift 主程序自己不得呼叫讀取類私有 API**，因為實測證明它在主程序內只會回傳空值，留著只會讓人誤以為有資料。

**關鍵邊界二：`NowPlayingSource` 是抽象介面，不是只為了好看。** perl 宿主是個空隙而不是正式介面，格式、可用性都不受我們控制。把它放在介面後面，系統關掉它時退到視窗標題，將來要加 AppleScript 來源、要換成瀏覽器擴充套件、或是 Apple 哪天正式開放，都只動一個檔案與一個陣列。這與第 6 節的 `LyricsProvider` 是同一套思路。

## 5. 現正播放資料層

### 5.1 來源介面

```swift
struct NowPlayingSnapshot: Equatable {
    let title: String
    let artist: String?
    let album: String?
    let duration: TimeInterval?      // 視窗標題來源沒有
    let isPlaying: Bool
    let position: PositionAnchor?    // 沒有就代表「無法得知播放位置」
    let artwork: ArtworkRef?         // 識別碼＋（可能缺席的）位元組
    let sourceID: String             // 哪個來源提供的，供除錯用
}

struct PositionAnchor: Equatable {
    let elapsed: TimeInterval        // 在 timestamp 那一刻的位置
    let timestamp: Date
    let rate: Double                 // 播放速率，暫停為 0
    func position(at now: Date) -> TimeInterval   // elapsed + (now − timestamp) × rate，不超過 duration
}

protocol NowPlayingSource {
    var id: String { get }
    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { get }
}
```

`NowPlayingMonitor` 持有一個依優先序排列的來源陣列，**第一個回傳非 nil 的即採用**。順序：

1. `MediaRemoteSource`（系統現正播放，涵蓋所有 App）。
2. `WindowTitleSource`（備援）。

來源以 Publisher 對外，因為 `MediaRemoteSource` 是推送式的，視窗標題來源在自己內部輪詢後也轉成 Publisher，介面統一。

### 5.2 `MediaRemoteSource`：perl 宿主行程

運作方式：

- 模組 `onStart` 時，以 `Process` 啟動 `/usr/bin/perl`，腳本以 `DynaLoader` 載入 `NotchCatMediaRemoteHelper` 動態庫並呼叫其入口函式。腳本內容是固定的幾行，嵌在 Swift 原始碼裡。
- 動態庫在宿主內持續查詢現正播放，**只在內容變化時**（曲目、播放速率、`ElapsedTime`／`Timestamp` 任一改變）往標準輸出寫一行 JSON。`ElapsedTime` 錨點變動也包含使用者拖動進度的情形，因此**拖動進度預期可被偵測到**（仍待實機確認：實作驗證時不得操作使用者的播放，故未實測拖動）。
- Swift 端逐行讀取，交給 `HelperMessageParser`，轉成 `NowPlayingSnapshot`。
- 模組 `onStop` 時終止行程。
- **行程意外結束時：** 以退避間隔重啟（建議 1、2、4…最多 30 秒）。連續失敗超過上限即放棄，`MediaRemoteSource` 回報 nil，`NowPlayingMonitor` 自動退到視窗標題備援。主程序不因此崩潰。放棄後每隔一段時間（預設 300 秒）重新嘗試一次；從睡眠喚醒時也會換掉舊的 helper 並重新開始。helper 本身若連續 10 次查詢都沒有回覆，以結束碼 4 離開，交給上述重啟機制處理。
- **啟動即讀不到任何欄位**（宿主被系統關掉的徵兆）與「真的沒在播」要能區分：前者是行程啟動後立刻回報鍵數為 0 且連續多次，後者是正常的空狀態。具體判定留待實作時以實測定。

動態庫的建置與載入細節（SwiftPM 的動態庫 target 輸出位置、執行時如何定位、arm64 簽章是否由連結器自動帶上）是**實作時第一個要驗證的項目**，已於 2026-11-04 驗證：`swift build` 產出 `.build/debug/libNotchCatMediaRemoteHelper.dylib`，連結器自動帶上 ad-hoc 簽章（`flags=0x20002(adhoc,linker-signed)`），不需額外 `codesign`；`/usr/bin/perl` 載入後讀到完整資料。perl 的 hardened runtime 拒絕相對路徑，動態庫必須以絕對路徑傳入。執行時以「可執行檔同目錄」定位。

### 5.3 播放位置

有 `PositionAnchor` 的來源（現正播放管道）：位置 = `anchor.position(at: 現在)`，由 UI 的計時器驅動更新，**不需要輪詢系統**。進度條與歌詞捲動都取這個值。

沒有 `PositionAnchor` 的來源（視窗標題）：沿用觀察式推算。`PlaybackEstimator` 記錄「觀察到曲名變成目前這首的時刻」，之後以 `現在時間 − 該時刻` 作為推估位置。

| 情形（僅視窗標題來源） | 結果 |
|---|---|
| 模組執行中、使用者切到下一首 | 推估可用，誤差約一個輪詢週期（1 秒內） |
| 暫停再繼續 | 可偵測（喇叭符號），暫停期間凍結推估 |
| 使用者拖動進度 | **無法偵測，推估失準** |
| 模組啟動時歌曲已播到一半 | **起點未知，推估不可用** |

`PlaybackEstimator` 必須能回報「目前無法推估」，而且**起點未知時就回報無法推估，不得拿 0 充數**。拿 0 充數會讓歌詞從頭開始捲動，比完全不捲更糟。

### 5.4 視窗標題備援的解析

`WindowTitleParser` 是純函式，不碰系統 API，必須完整單元測試。

- 以視窗**擁有者名稱**比對已知的播放器清單（v1：`YouTube Music`），再解析其標題。擁有者比對先行，可避免把任意視窗的標題誤判成歌曲。
- 已知格式：`YouTube Music - <曲名> | YouTube Music`，必要時附加喇叭符號。解析時需剝除前後綴的服務名稱。
- **播放中的判斷依據是標題尾端的喇叭符號**。符號消失視為暫停。
- 解析失敗時回 nil，由 `NowPlayingMonitor` 當作「此來源沒有資料」，絕不可回傳半截字串當曲名。
- 此格式由第三方決定、隨時可能變，所以解析器必須對未知格式安全退場，而不是硬塞。
- 備援只涵蓋清單內的播放器，且背景分頁的瀏覽器看不到。這是備援，不追求涵蓋面。

### 5.5 isActive 的定義

`isActivePublisher` 在「有曲目且正在播放」時為 true（現正播放管道：`rate > 0`；視窗標題：喇叭符號存在）。暫停時為 false，Music 不再接管 Preview，瀏海回到 Shell 預設的空黑條，而不是停著顯示曲名佔著版面。此行為若實際使用覺得突兀，屬於手感調整，留待實機驗收後再議。

暫停時管道實際回傳什麼（rate 為 0，還是整份資料消失）：2026-11-04 有一次被動取樣恰逢播放器處於暫停，回傳完整資料且 `PlaybackRate` 為 0（即 `"rate":0` 的曲目行，而非 `{"none":true}`）。實際按下暫停的轉換過程仍待實機確認（驗證時不得操作使用者的播放）。兩種情形都要處理。

## 6. 歌詞層

### 6.1 來源抽象與清單

```swift
struct TrackIdentity {
    let title: String
    let artist: String
    let album: String?
    let duration: TimeInterval?     // 備援來源沒有；有的時候才做長度比對
}

enum Lyrics {
    case synced([LyricLine])   // 逐行時間軸
    case plain(String)         // 無時間軸
}

struct LyricLine {
    let time: TimeInterval
    let text: String
}

protocol LyricsProvider {
    var id: String { get }
    func fetchLyrics(for track: TrackIdentity) async throws -> Lyrics?
}
```

模組本身只認識 `LyricsRepository`，不認識任何具體來源。**增刪來源或調整順序只動 `LyricsRepository` 持有的那個陣列。**

**v1 的清單（依序）：** LRCLIB、網易雲。

**明確排除 YouTube Music 的歌詞介面。** 排除理由記錄於此以免日後重複討論：現正播放管道回傳的欄位中沒有 YouTube 影片 ID 也沒有 URL，`ContentItemIdentifier` 只是本機 UUID。因此即使走 YouTube Music 的非官方歌詞介面，仍然必須先用歌名與演出者模糊搜尋比對曲目，在比對難度上並未優於 LRCLIB，卻額外承擔非官方介面的不穩定性。

### 6.2 依序嘗試，取第一個可用的

**一個一個來源查，查到可用的歌詞就停，顯示它。**

「可用」指**帶時間軸的歌詞**。純文字算作保底，不算可用。單趟走訪，每個來源只查一次：

1. 依序詢問各來源。拿到帶時間軸的結果就立即停止並採用，後面的來源不再查詢。
2. 途中若拿到純文字，先暫存，繼續問下一個來源。同一輪內已有暫存時不覆蓋，保留順位較前者。
3. 全部問完仍無同步結果時，採用步驟 2 暫存的純文字；若連暫存都沒有，視為此曲無歌詞。

純文字結果在第一次查詢時就已經拿到，無須為了降級而重打一次網路。

理由見實測結果 8：若純文字也算「第一個命中」，那首歌會停在 LRCLIB 的純文字，丟掉網易雲的 82 行同步版本，而動態歌詞正是本模組的主要目的。

> **待使用者確認：** 這個「純文字不算可用」是我的建議，尚未得到明確回覆。若要改成嚴格的「第一個有東西就停」，只需拿掉步驟 2、3 的暫存邏輯，其餘不受影響。

### 6.2.1 歌詞可信度把關

動態歌詞只需要做到「此刻是哪一行」，不需要逐字，但**寧可不顯示，也不顯示不準的**。因此帶時間軸的結果除了歌名與長度比對，還要通過合理性檢查才算可用：

- 行數至少 2 行（單行通常是「純音樂」之類的佔位）。
- 已知曲長（大於 0）時，最後一行的時間不得超過曲長加 5 秒（超過通常表示抓到不同版本）。

沒通過的結果視同該來源「乾淨地回答了，但沒有可用結果」：不算失敗、不當保底，繼續問下一個來源。純文字不受此檢查影響（它不用於動態顯示）。不再另設更嚴的規則（例如歌詞佔曲長比例），因為前奏、間奏、尾奏很長的歌很常見，規則過嚴會擋掉正確的歌詞。

### 6.3 比對與正規化

`TrackNormalizer` 負責把播放來源給的欄位整理成查詢用的 `TrackIdentity`。查詢前採「精確查詢失敗則退回模糊搜尋」的兩層策略（依據實測結果 7）：

- 移除歌名中的 `(feat. …)`、`(ft. …)`、`[Remastered …]` 等括號後綴後重試。
- 演出者欄位可能含多位演出者，以分隔符切出主要演出者再試。
- **YouTube 影片標題的清理：** 標題形如「頻道名 - 曲名 ft. 其他人」且頻道名等於（或包含於）`Artist` 時，取「 - 」之後作為曲名，再剝除 `ft.`／`feat.` 及其後內容、以及「MV」「Official Video」「Lyrics」等常見括號字樣；多餘空白一併收斂（實測樣本中「抱我!」與「ft.」之間有兩個空白）。拆不出來就維持原標題，交給模糊搜尋。
- 長度比對採**容差**而非相等，建議 ±5 秒，且只在雙方都有長度時才比。feat. 版本與原版長度常有差異，容差過嚴會砍掉正確結果。
- **驗證不得使用字串完全相等**，必須容忍簡繁體差異（依據實測結果 9）。建議比對前統一轉換，或改用寬鬆的相似度判斷。

### 6.4 快取

- **正向快取：** 以正規化後的 `TrackIdentity` 為鍵存放歌詞結果，重播同一首不重打網路。
- **反向快取：** 查無結果者同樣記錄，附存活時間（建議 24 小時），避免同一首沒歌詞的曲目每次切回來都重打一輪所有來源。
- **封面快取：** 在 `ArtworkCache`，以 `ArtworkIdentifier` 為鍵、記憶體即可。位元組缺席時沿用該鍵已有的圖；從未有過就顯示佔位圖。

## 7. 兩種模式的畫面

### 7.0 為什麼 Preview 不能直接顯示一行歌詞

Preview 的條寬是「瀏海寬 + 160pt」，瀏海本體是螢幕硬體的挖洞，畫在上面的東西看不到。實際可用的只有瀏海左右各約 80pt 的「翼」，高度等於瀏海高度。一行歌詞放不進去，也會被瀏海從中間切開。因此 Preview 採兩翼加下方歌詞條的做法（類似 boring.notch）。

### 7.1 Preview：兩翼加歌詞條

```
[封面] ░░░ 瀏海 ░░░ [音柱]      ← 兩翼：與瀏海同高，純黑，和瀏海連成一體
┗━━━━ 目前這句歌詞 ━━━━┛        ← 歌詞條：在瀏海正下方，半透明黑底
```

- **左翼：** 封面縮圖（方形置中裁切）。沒有封面時用音符佔位圖。
- **右翼：** 播放中的動態音柱。
- **歌詞條：** 顯示完整寬度的一行，內容是當前歌詞行；位置未知或沒有同步歌詞時改顯示曲名；文字過長用跑馬燈。底色為黑色加透明度（預設 0.6，寫成單一常數便於實機調整），下緣圓角與 Preview 的圓角一致。
- **可折疊：** 使用者可以折疊歌詞條，折疊後 Preview 回到只有兩翼。開關放在 Music 的 Expanded 畫面裡（一顆小按鈕），狀態存在 `UserDefaults`，預設為展開。折疊只影響 Preview，不影響 Expanded 內的歌詞。

**Shell 的配合（本模組唯一對 Shell 的改動）：** Preview 的高度要能隨模組需求變動。

- `NotchModule` 新增 `var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never>`，預設實作回傳固定的 0，既有模組不受影響。
- `ModuleManager` 對外提供「目前 active 模組要求的額外高度」，Shell 據此加高 Preview，上限夾在一個常數（建議 28pt）內。
- Preview 實際高度 = 瀏海高度 + 額外高度。額外高度變動時要有動畫，且 Preview 的 hover 判定範圍必須跟著更新，歌詞條範圍內也算 hover。
- 固定大小的容器必須容得下 Preview 加上額外高度的最大值（目前 Expanded 的 180pt 遠大於此，所以容器大小不變）。
- Music 在「不 active」、「已折疊」兩種情形下回報 0。

### 7.2 Expanded

| 情形 | 內容 |
|---|---|
| **Expanded，有曲目** | 封面、曲名、演出者、進度條（有總長度與位置時）、上一首／播放暫停／下一首、歌詞條折疊開關，以及（若有）歌詞。 |
| **Expanded，沒有曲目** | 使用者從 AppBar 手動點進來才會看到：一行「目前沒有播放」的文字，控制列隱藏。 |

> 「目前沒有播放」這行字是我加的，尚未經使用者確認。

**欄位缺席時的處理**（任何欄位都可能是 nil，畫面不得破版）：

- **封面：** 置中裁切成方形。沒有圖時以音符佔位圖維持版面穩定。
- **進度條：** 需要 `duration` 與 `position` 同時存在才顯示，否則整條隱藏，不畫比例不明的條。
- **演出者：** nil 時整行省略。
- **來源 App 名稱：** 現正播放管道的欄位不含 App 名稱，因此不顯示。

### 7.3 退化階梯

任一層失敗時往下退，畫面不得開天窗：

1. 同步歌詞且位置可得 → 歌詞條跟著位置顯示當前行，Expanded 捲動。
2. 同步歌詞但位置未知 → 歌詞條顯示曲名，Expanded 顯示靜態全文。
3. 僅純文字歌詞 → 歌詞條顯示曲名，Expanded 顯示可捲動的靜態全文。
4. 無歌詞 → 歌詞條顯示曲名，Expanded 只顯示曲目資訊。
5. 無任何來源有資料 → 模組不 active，Preview 回到空黑條（額外高度為 0）。

## 8. 播放控制

Expanded 的上一首／播放暫停／下一首透過 `MRMediaRemoteSendCommand` 送出，由 NotchCat 主程序自己呼叫（不經 perl 宿主）。

> **已於 2026-10-14 在 macOS 15.7（arm64）實測：播放控制可行。** 從一個 ad-hoc 簽署、非 Apple 簽署的獨立編譯程式呼叫 `MRMediaRemoteSendCommand`，指令確實生效，且與讀取端無關。驗證方式是用另一個獨立程序讀取播放狀態來交叉確認，避免被送指令的程序自己的讀取結果誤導：
>
> - 送出 `pause`（回傳 `true`）後，獨立程序讀到 `playing: false`。
> - 送出 `play`（回傳 `true`）復原後，獨立程序讀到 `playing: true`。
> - 另在 Apple 簽署的直譯器內，以 `togglePlayPause`（回傳 `true`）送出兩次，`isPlaying` 依序為 `true → false → true`。
>
> `next`／`previous` 只實測了指令碼與回傳值，尚未在真機上逐一確認曲目切換，列入實機驗收。

控制走 `MediaRemoteControl.swift`，顯示走第 5 節的資料層，兩者必須互相獨立（第 9 節）。

## 9. 錯誤處理

- **perl 宿主行程失敗：** 退避重啟；超過上限就退到視窗標題備援。不顯示錯誤訊息，也不得影響控制列與已顯示的歌詞。
- **歌詞來源逾時：** 每個來源設獨立逾時（建議 5 秒），逾時視同未命中，繼續問下一個，不得阻塞畫面。
- **網路不可用：** 歌詞靜默退化為無歌詞，不顯示錯誤訊息。曲目顯示不受影響。
- **視窗標題格式改變（備援）：** 解析器回 nil，視為此來源沒有曲目。**不得顯示解析了一半的字串。**
- **播放控制失敗：** 控制列隱藏或禁用，曲目顯示照常運作。**控制與顯示必須互相獨立，任一邊壞掉不得拖垮另一邊。**
- **歌詞與實際曲目不符：** 比對驗證失敗時寧可不顯示，也不顯示錯的歌詞。

## 10. 測試策略

- **`HelperMessageParser` 與 `WindowTitleParser` 是測試重點。** 純函式，輸入字串輸出 Snapshot。前者涵蓋：完整欄位、缺欄位、封面位元組缺席、鍵數為 0、格式損壞的 JSON；後者涵蓋已知格式、附喇叭符號與否、擁有者不在清單內、空標題、未知格式、曲名本身含有分隔符號。
- `PositionAnchor.position(at:)`：純數學，涵蓋播放中、暫停（rate 0）、超過總長度時封頂、錨點在未來。
- `PlaybackEstimator`：純邏輯，可注入時鐘。必須涵蓋「起點未知時回報無法推估」這條規則。
- `TrackNormalizer`：以實測樣本「蘇裔非 - 抱我!  ft. 莊岱諺／頻道 蘇裔非」為測資，涵蓋拆得出與拆不出、`ft.` 剝除、空白收斂。
- `Lyrics` 的 LRC 解析、`LyricsRepository` 的走訪順序：純邏輯，全部可單元測試。
- `LyricsProvider` 以假實作注入，驗證「純文字不算可用、同步結果勝過來源順位」。這是第 6.2 節的核心規則，必須有測試保護。
- `MediaRemoteSource` 的行程監管（重啟、退避、放棄後切換）：以可注入的行程啟動器做單元測試；實際載入 perl 與動態庫列為實機驗證項目。
- `WindowTitleSource` 與各網路來源：碰系統 API 或外部服務，列為實機手動驗收項目。
- Shell 的額外高度：`ModuleManager` 在 active 模組切換、額外高度變動時發布正確的值並夾在上限內；`NotchMode` 在給定額外高度時算出的 Preview 尺寸與 hover 範圍。
- 歌詞條折疊狀態的讀寫（`UserDefaults` 可注入），以及折疊時額外高度回報 0。
- 實機驗收清單：歌詞條的展開與折疊動畫、半透明底色的觀感、hover 範圍是否包含歌詞條；YouTube Music、YouTube、Spotify 各播一次；切歌、暫停、拖動進度、關閉播放器、歌詞對不上時的退化、無網路；**perl 行程被手動殺掉後能自動恢復**；以及視窗標題備援的**啟動時歌曲已播到一半（推估應回報不可用，歌詞不得從頭捲）**。

## 11. 未決事項與風險

- **整個播放來源建立在 Apple 簽章宿主的空隙上。** 不是正式介面，系統更新隨時可能關掉。`/usr/bin/perl` 本身也可能在日後的 macOS 被移除（Apple 已公告腳本語言執行環境會被移除）。`NowPlayingSource` 的備援機制與視窗標題來源就是為此存在，但備援只涵蓋 YouTube Music。這是本設計最脆弱的一環。
- **這個管道只適合自用與 side-loading 的發布方式。** 若 Phase 4 要走 App Store 或公證發布，需要重新評估是否可以接受。
- **Spotify 與其他 App 未實測。** 目前只驗證過兩種來源。
- **perl 宿主的穩定性細節未驗：** SwiftPM 動態庫的產出位置與簽章、暫停時回傳什麼、拖動進度是否會更新錨點、長時間執行的記憶體與 CPU。皆列入實作與驗收。
- **歌詞著作權。** 社群歌詞資料庫與非官方介面皆處於灰色地帶。自用無虞，但 Phase 4 若要公開發布需重新評估。
- **歌詞命中率的真實分布。** 現有樣本僅兩首同專輯曲目，實作後應以較廣的歌單重新評估清單順序。
- **逐字歌詞**目前無解。
- **Preview 加高要動 Shell 的狀態機與 hover 判定。** Shell 先前花了很多力氣才解掉 hover 閃爍，這次改動必須保持「固定大小容器、只有繪製內容動畫」的原則，並在實機上重新驗收 hover 不閃爍。
- **歌詞條的透明度與高度是憑印象給的值**（0.6、24pt），需要實機調整。
- **網易雲非官方介面的穩定性。** 介面可能變動或限制外部請求，`LyricsProvider` 抽象的存在正是為了讓替換成本可控。
