# Phase 2：Music 模組設計

寫於 2026-10-14。上游文件：[`docs/PLANNING.md`](../../PLANNING.md)（整體架構）、[`docs/HANDOFF.md`](../../HANDOFF.md)（階段進度與實測紀錄）。

## 1. 目標與範圍

在 NotchCat 的瀏海面板上顯示「現正播放」與「動態歌詞」，作為 Phase 1 模組系統之後的第一個真實模組。

**範圍內：**

- 讀取目前播放的曲名與播放狀態。**v1 的來源是視窗標題，只支援 YouTube Music。**
- 依播放狀態決定 Music 是否接管 Preview：播放中顯示歌詞或曲名，沒在播放時 Preview 回到 Shell 預設的空黑條。
- 取得逐行同步歌詞，在播放位置可推估時依其捲動。
- Expanded 下的播放控制（上一首／播放暫停／下一首）。

**已於 2026-10-28 對齊兩態 Shell。** Shell 只有 Preview 與 Expanded 兩個模式，Expanded 上方有模組切換 AppBar，因此本 spec 中依舊版狀態機寫的段落（第 1、2、5.5、7、9 節）已改寫。

**範圍已於 2026-10-21 縮減。** 原案要讀系統層級的 now-playing（含演出者、專輯、總長度、播放進度、封面圖），但實測證明該管道對第三方 App 封閉（第 3 節第 1 點）。改用視窗標題後，**總長度、播放進度與封面圖都拿不到**，因此進度條與封面一併移出範圍（第 7 節）。

**範圍外：** 多模組搶 Preview 的排序體驗（Phase 3）、模組開關設定 UI（Phase 3）、簽章與發布（Phase 4）。

**明確的決策：Music 模組與歌詞不拆成兩個階段**，兩者在同一份 spec、同一輪實作內完成。此決定已知的風險是歌詞的外部不確定性會直接落在主線上；接受這個風險的理由是畫面佈局可以一次定案，不必為了之後塞歌詞回頭重改 Preview 與 Expanded。

## 2. 既有基礎

兩態 Shell 與模組系統已完成並通過真機驗收，本模組直接架在上面，不改動 Shell：

- 實作 `NotchModule` protocol（`ModuleKit/NotchModule.swift`），向 `ModuleManager` 以 `register(_:)` 註冊。
- `isActivePublisher` 回報「是否正在播放」，`ModuleManager` 據此算出 `activeModule`：Music active 時接管 Preview；不 active 時 Preview 顯示 Shell 預設的空黑條。這條線已經打通並驗證過，本模組只需要正確餵值。
- 三個畫面入口：`tabIcon()`（Expanded 的 AppBar 上的圖示）、`previewView()`、`expandedView()`。
- Expanded 每次展開，AppBar 的選取會重設成當下的 `activeModule`；Music 沒在播放時沒有 active 模組，使用者可以自己點 AppBar 上的 Music 圖示進入它的 Expanded。

## 3. 設計所依據的實測結果

> **本節已於 2026-10-21 大幅改寫。** 原本的第 1 與第 3 點宣稱 MediaRemote 的 now-playing 讀取可用，那是錯的，整個第 5 節的資料層因此重做。改寫理由見下方第 1 點。

以下全部來自 macOS 15.7 上的實測，不是推測。設計中幾個不直覺的決定都源自這裡。

1. **MediaRemote 的 now-playing 讀取對第三方 App 不可用（2026-10-21 更正）。** 先前結論說可用，那是測試方法造成的偽陽性：所有探針都以 `swift <檔名>.swift` 執行，跑在 Apple 簽章的直譯器底下並繼承其存取權。改以六種宿主組態重測，同一時刻、同一段程式碼：

   | 宿主行程 | 取得的鍵數 |
   |---|---|
   | Apple 簽章的 `swift` 直譯器 | 13 |
   | ad-hoc 簽章的裸執行檔 | 0 |
   | ad-hoc 簽章、含 bundle id 的 .app | 0 |
   | Apple Development 憑證簽章的 .app | 0 |
   | Apple Distribution 憑證簽章的 .app | 0 |
   | 同上，經 LaunchServices 正常啟動 | 0 |

   關卡擋的是簽章授權，不是打包方式，任何第三方身分都打不開。

2. **MediaRemote 的播放控制不受此限制，實測可用。** `MRMediaRemoteSendCommand` 送出的暫停與播放確實生效。因此本模組「控制得了，但讀不到在播什麼」。

3. **視窗標題可讀，且不需要任何權限。** 以 `CGWindowListCopyWindowInfo` 從編譯後的執行檔即可取得各視窗標題，無須螢幕錄製或輔助使用授權。開發機上 YouTube Music 這個 Chrome PWA 的視窗標題形如：

   ```
   擁有者：YouTube Music
   標題：  YouTube Music - 假日而已 | YouTube Music 🔊
   ```

   曲名在標題內，且瀏覽器會在正在發聲的視窗標題附加喇叭符號，可作為播放中的判斷依據。**這是目前唯一能涵蓋瀏覽器播放的讀取管道。**

4. **視窗標題給不出播放位置。** 標題只在換曲時變動，不含已播秒數、總長度或播放速率。**這直接砍掉進度條，也讓逐行同步歌詞無法用原設計實現。** 第 5.4 節的推算機制是對此的補償，不是完整替代。

5. **開發機未安裝 Apple Music 與 Spotify。** AppleScript／ScriptingBridge 路線在這台機器上無從發揮，且本來就涵蓋不到瀏覽器播放。

6. **歌詞來源的精確比對很脆。** 以一位中文獨立演出者的兩首曲目實測 LRCLIB：其中一首用「歌名＋演出者＋專輯＋長度」精確查詢**沒中**，退回模糊搜尋才找到，原因是系統回報的歌名不含資料庫裡的 `(feat. …)` 後綴。

7. **同一首曲目在不同來源的品質不同。** 同樣那首歌，LRCLIB 只有純文字，網易雲有 82 行逐行時間軸。**這是第 6 節採兩段式查詢的直接證據。**

8. **簡繁體不一致。** 網易雲是以簡體曲名比對到我們送出的繁體歌名。比對驗證邏輯因此不得使用字串完全相等。

9. **封面圖不是每次都附**（在直譯器下觀察）。連續查詢三次只有一次帶實際位元組。此發現對 MediaRemote 已無意義，但對任何未來的資料來源仍有參考價值。

**樣本侷限：** 歌詞命中率只測了同一位演出者、同一張專輯的兩首曲目，不足以代表整體命中率，僅足以證明上述機制性問題存在。視窗標題格式只在 YouTube Music 這一個 PWA 上驗證過。

## 4. 架構總覽

```
Sources/NotchCatKit/Modules/Music/
  MusicModule.swift             # 實作 NotchModule，組裝下列元件
  NowPlaying/
    NowPlayingSource.swift      # 「什麼在播」的來源抽象介面
    NowPlayingSnapshot.swift    # 一次讀取的正規化結果（值型別）
    WindowTitleSource.swift     # CGWindowList 讀視窗標題，涵蓋瀏覽器播放
    WindowTitleParser.swift     # 標題字串 → 曲目（純函式，完全可測）
    NowPlayingMonitor.swift     # 依序詢問來源、偵測換曲、驅動 @Published
    PlaybackEstimator.swift     # 由「觀察到換曲的時刻」推算播放位置
    MediaRemoteControl.swift    # 私有 API 只在這個檔案出現，且只用於送控制指令
  Lyrics/
    TrackIdentity.swift         # 查詢用的正規化曲目識別
    Lyrics.swift                # synced / plain 兩種歌詞的表示
    LyricsProvider.swift        # 歌詞來源抽象介面
    LyricsRepository.swift      # 兩段式解析 + 快取
    Providers/
      LRCLIBProvider.swift
      NetEaseProvider.swift
  Views/
    MusicPreviewView.swift
    MusicExpandedView.swift
```

**關鍵邊界一：私有 API 只准出現在 `MediaRemoteControl.swift`，而且只用於送出控制指令。** 讀取 now-playing 的私有呼叫一律不得出現，因為實測證明它對出貨的 App 回傳空值，留著只會讓人誤以為有資料。

**關鍵邊界二：`NowPlayingSource` 是抽象介面，不是只為了好看。** 視窗標題是目前唯一可行的來源，但它明顯是權宜之計：格式由第三方 App 決定、隨時可能變。把它放在介面後面，將來要加 AppleScript 來源、要換成瀏覽器擴充套件回報、或是 Apple 哪天重新開放 MediaRemote，都只動一個檔案與一個陣列。這與第 6 節的 `LyricsProvider` 是同一套思路。

## 5. 現正播放資料層

### 5.1 來源介面

```swift
struct NowPlayingSnapshot: Equatable {
    let title: String          // 來源能提供的最佳曲名
    let artist: String?        // 視窗標題多半沒有，故為選用
    let isPlaying: Bool
    let sourceID: String       // 哪個來源提供的，供除錯與優先序判斷
}

protocol NowPlayingSource {
    var id: String { get }
    func currentTrack() -> NowPlayingSnapshot?
}
```

`NowPlayingMonitor` 持有一個依優先序排列的來源陣列，每次輪詢依序詢問，**第一個回傳非 nil 的即採用**。v1 只有 `WindowTitleSource` 一個來源。

### 5.2 更新策略：輪詢，因為沒有通知可用

MediaRemote 的通知機制與其讀取一併被擋，視窗標題也不會主動通知變更。**因此本模組只能輪詢，這是外部限制造成的，不是設計偏好。**

- **活躍檔（1 秒）。** 已偵測到播放中時使用。換曲要能很快反映在面板上，而讀視窗清單是純本機呼叫、成本低。
- **閒置檔（5 秒）。** 沒有偵測到播放時使用。
- **閒置檔不可因為面板隱藏而關閉**，否則一旦使用者開始播放音樂，模組將永遠不會醒來。

### 5.3 視窗標題的解析

`WindowTitleParser` 是純函式，不碰系統 API，必須完整單元測試。

- 以視窗**擁有者名稱**比對已知的播放器清單（v1：`YouTube Music`），再解析其標題。擁有者比對先行，可避免把任意視窗的標題誤判成歌曲。
- 已知格式：`YouTube Music - <曲名> | YouTube Music`，必要時附加喇叭符號。解析時需剝除前後綴的服務名稱。
- **播放中的判斷依據是標題尾端的喇叭符號**，瀏覽器會在正在發聲的視窗加上它。符號消失視為暫停。
- 解析失敗時回 nil，由 `NowPlayingMonitor` 當作「此來源沒有資料」，絕不可回傳半截字串當曲名。
- 此格式由第三方決定、隨時可能變，所以解析器必須對未知格式安全退場，而不是硬塞。

### 5.4 播放位置：只能推算，而且會退化

**視窗標題給不出已播秒數、總長度或播放速率（實測結果 4）。** 本節的機制是補償，不是完整替代，其限制必須如實反映在畫面上。

機制：`PlaybackEstimator` 記錄「觀察到曲名變成目前這首的時刻」，之後以 `現在時間 − 該時刻` 作為推估位置。

能用與不能用的情形：

| 情形 | 結果 |
|---|---|
| 模組執行中、使用者切到下一首 | 推估可用，誤差約一個輪詢週期（1 秒內） |
| 暫停再繼續 | 可偵測（喇叭符號），暫停期間凍結推估 |
| 使用者拖動進度 | **無法偵測，推估失準** |
| 模組啟動時歌曲已播到一半 | **起點未知，推估不可用** |

因此 `PlaybackEstimator` 必須能回報「目前無法推估」，而且**起點未知時就回報無法推估，不得拿 0 充數**。拿 0 充數會讓歌詞從頭開始捲動，比完全不捲更糟。

**結論：總長度未知，所以沒有進度條**（見第 7 節）。逐行同步歌詞只在推估可用時捲動，否則退化為靜態全文。

### 5.5 isActive 的定義

`isActivePublisher` 在「有曲目且喇叭符號存在」時為 true。暫停時為 false，Music 不再接管 Preview，瀏海回到 Shell 預設的空黑條，而不是停著顯示曲名佔著版面。此行為若實際使用覺得突兀，屬於手感調整，留待實機驗收後再議。

## 6. 歌詞層

### 6.1 來源抽象

```swift
struct TrackIdentity {
    let title: String
    let artist: String
    let album: String?
    let duration: TimeInterval
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

模組本身只認識 `LyricsRepository`，不認識任何具體來源。增刪來源或調整順位只動 `LyricsRepository` 持有的陣列。這與 Phase 1 中 `ModuleManager` 對 `NotchModule` 的關係是同一套思路。

**v1 的來源清單（依序）：** LRCLIB、網易雲。

**明確排除 YouTube Music。** 排除理由記錄於此以免日後重複討論：MediaRemote 回傳的欄位中沒有 YouTube 影片 ID 也沒有 URL，`ContentItemIdentifier` 只是本機 UUID。因此即使走 YouTube Music 的非官方歌詞介面，仍然必須先用歌名與演出者模糊搜尋比對曲目，在比對難度上並未優於 LRCLIB，卻額外承擔非官方介面的不穩定性。

### 6.2 兩段式解析

**同步歌詞的優先級高於來源順位。**

**單趟走訪即可，每個來源只查一次：**

1. 依序詢問各來源。拿到**帶時間軸**的結果就立即停止並採用，後面的來源不再查詢。
2. 途中若拿到的是純文字，**先暫存不採用**，繼續問下一個來源。同一輪內已有暫存時不覆蓋，保留順位較前者。
3. 全部問完仍無同步結果時，採用步驟 2 暫存的純文字；若連暫存都沒有，視為此曲無歌詞。

注意這是**一趟**走訪，不是兩輪。純文字結果在第一次查詢時就已經拿到，無須為了降級而重打一次網路。

理由見實測結果 5：若採單純的「先命中就用誰」，實測中那首歌會停在 LRCLIB 的純文字，丟掉網易雲的 82 行同步版本，而動態歌詞正是本模組的主要目的。

### 6.3 比對與正規化

查詢前對 `TrackIdentity` 正規化，並採「精確查詢失敗則退回模糊搜尋」的兩層策略（依據實測結果 4）：

- 移除歌名中的 `(feat. …)`、`(ft. …)`、`[Remastered …]` 等括號後綴後重試。
- 演出者欄位可能含多位演出者，以分隔符切出主要演出者再試。
- 長度比對採**容差**而非相等，建議 ±5 秒。feat. 版本與原版長度常有差異，容差過嚴會砍掉正確結果（依據實測結果 4）。
- **驗證不得使用字串完全相等**，必須容忍簡繁體差異（依據實測結果 6）。建議比對前統一轉換，或改用寬鬆的相似度判斷。

### 6.4 快取

- **正向快取：** 以正規化後的 `TrackIdentity` 為鍵存放歌詞結果，重播同一首不重打網路。
- **反向快取：** 查無結果者同樣記錄，附存活時間（建議 24 小時），避免同一首沒歌詞的曲目每次切回來都重打一輪所有來源。
- **封面快取：** v1 不適用。視窗標題不提供封面，Expanded 一律顯示佔位圖（見第 7 節）。此項保留供未來有封面的來源使用。

## 7. 兩種模式的畫面

> **已於 2026-10-21 依新的資料層縮減。** 原案的進度條與封面圖都移除了，因為視窗標題給不出總長度，也給不出圖。

| 模式 | 內容 |
|---|---|
| **Preview，Music 不 active** | Shell 預設的空黑條（Music 完全不畫東西）。 |
| **Preview，Music active** | 當前歌詞行（跑馬燈）。推估不可用或無同步歌詞時，改顯示曲名跑馬燈。 |
| **Expanded，有曲目** | 曲名、來源名稱、上一首／播放暫停／下一首，以及（若有）歌詞。 |
| **Expanded，沒有曲目** | 使用者從 AppBar 手動點進來才會看到：一行「目前沒有播放」的文字，控制列隱藏。 |

**明確不做的項目與原因：**

- **沒有進度條。** 總長度未知（實測結果 4），畫一條比例不明的條只會誤導。
- **沒有封面圖。** 視窗標題不含封面，也沒有可用的識別碼去查。Expanded 以一個音符佔位圖維持版面穩定。
- **演出者欄位多半是空的。** 視窗標題通常只有曲名。欄位存在但允許為 nil，畫面不得因為缺它而破版。

**退化階梯**（任一層失敗時往下退，畫面不得開天窗）：

1. 同步歌詞且推估可用 → 跟著推估位置捲動。
2. 同步歌詞但推估不可用 → Expanded 顯示靜態全文，Preview 顯示曲名。
3. 僅純文字歌詞 → Expanded 顯示可捲動的靜態全文，Preview 顯示曲名。
4. 無歌詞 → 兩個畫面都只顯示曲目資訊。
5. 無任何來源有資料 → 模組不 active，Preview 回到空黑條。

## 8. 播放控制

Expanded 的上一首／播放暫停／下一首預計透過 `MRMediaRemoteSendCommand` 送出。

> **已於 2026-10-14 在 macOS 15.7（arm64）實測：播放控制可行。** 從一個 ad-hoc 簽署、非 Apple 簽署的獨立編譯程式呼叫 `MRMediaRemoteSendCommand`，指令確實生效，且與讀取端無關。驗證方式是用另一個獨立程序讀取播放狀態來交叉確認，避免被送指令的程序自己的讀取結果誤導：
>
> - 送出 `pause`（回傳 `true`）後，獨立程序讀到 `playing: false`。
> - 送出 `play`（回傳 `true`）復原後，獨立程序讀到 `playing: true`。
> - 另在 Apple 簽署的直譯器內，以 `togglePlayPause`（回傳 `true`）送出兩次，`isPlaying` 依序為 `true → false → true`，與 spec 原案的探針一致。
>
> 因此 Expanded 的上一首／播放暫停／下一首控制列照原案實作，Task 10 不需跳過、第 7 節版面不需修訂。`next`／`previous` 只實測了指令碼與回傳值，尚未在真機上逐一確認曲目切換，列入實機驗收。
>
> **這次探針同時揭穿了讀取端的偽陽性**，後續以六種宿主組態完整重測並確認讀取封閉，詳見第 3 節第 1 點。**控制端不受影響，結論維持可行。**
>
> 因此本模組的形態是「控制得了，但讀不到在播什麼」：控制指令走 `MediaRemoteControl.swift` 的私有 API，曲目資訊則完全改由第 5 節的視窗標題來源提供。兩者必須互相獨立（第 9 節）。`next`／`previous` 只實測了指令碼與回傳值，尚未逐一確認曲目切換，列入實機驗收。

## 9. 錯誤處理

- **歌詞來源逾時：** 每個來源設獨立逾時（建議 5 秒），逾時視同未命中，繼續問下一個，不得阻塞畫面。
- **網路不可用：** 歌詞靜默退化為無歌詞，不顯示錯誤訊息。曲目顯示不受影響。
- **視窗標題格式改變：** 解析器回 nil，模組視為沒有曲目，不 active，Preview 回到空黑條。**不得顯示解析了一半的字串。** 這是最可能發生的故障，因為格式由第三方決定。
- **播放控制符號解析失敗：** 控制列隱藏或禁用，曲目顯示照常運作。**控制與顯示必須互相獨立，任一邊壞掉不得拖垮另一邊。**
- **歌詞與實際曲目不符：** 比對驗證失敗時寧可不顯示，也不顯示錯的歌詞。

## 10. 測試策略

- **`WindowTitleParser` 是測試重點。** 純函式，輸入字串輸出曲目，必須涵蓋：已知格式、附喇叭符號與否、擁有者不在清單內、空標題、未知格式、以及曲名本身含有分隔符號的情形。
- `PlaybackEstimator`：純邏輯，可注入時鐘。必須涵蓋「起點未知時回報無法推估」這條規則。
- `TrackIdentity` 正規化、`Lyrics` 的 LRC 解析、`LyricsRepository` 的兩段式解析順序：純邏輯，全部可單元測試。
- `LyricsProvider` 以假實作注入，驗證兩段式解析確實讓同步結果勝過來源順位。這是第 6.2 節的核心規則，必須有測試保護。
- `WindowTitleSource` 與各網路來源：碰系統 API 或外部服務，列為實機手動驗收項目。
- 實機驗收清單：切歌、暫停、關閉播放器、歌詞對不上時的退化、無網路、以及**啟動時歌曲已播到一半（推估應回報不可用，歌詞不得從頭捲）**。

## 11. 未決事項與風險

- **視窗標題格式只在 YouTube Music 驗證過，而且由第三方決定。** 這是本設計最脆弱的一環，Google 改版就會壞。`NowPlayingSource` 抽象的存在就是為了讓替換成本可控。
- **本模組只涵蓋已知清單內的播放器。** 使用者改用其他播放器就沒有任何顯示。v1 只支援 YouTube Music。
- **歌詞著作權。** 社群歌詞資料庫與非官方介面皆處於灰色地帶。自用無虞，但 Phase 4 若要公開發布需重新評估。
- **歌詞命中率的真實分布。** 現有樣本僅兩首同專輯曲目，實作後應以較廣的歌單重新評估來源順位。
- **逐字歌詞與拖動進度的偵測**：目前皆無解，若日後改用瀏覽器擴充套件回報資料，兩者可一併解決。
- **網易雲非官方介面的穩定性。** 介面可能變動或限制外部請求，`LyricsProvider` 抽象的存在正是為了讓替換成本可控。
