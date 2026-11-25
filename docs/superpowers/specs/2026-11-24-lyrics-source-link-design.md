# 歌詞來源連結（LyricsHit）設計

寫於 2026-11-24，依使用者的簡化指示更新。這是 MusiCat（`rust/`）v0.1.0 之後的第一個小功能。讀者是要規劃並實作的 agent，沒有參與過先前的對話；請先讀完本文，再讀「參考」一節的檔案。

## 1. 意圖

歌詞命中時，使用者想在 Expanded 裡有一個地方可以點，直接用預設瀏覽器開啟這份歌詞的來源頁面。**使用者明確的簡化要求：只要命中歌詞就給網址，沒命中就留空；不在意 LRCLIB 的 id 之類的細節。**

## 2. 目標、非目標、反目標

**目標**
- 歌詞命中（`Synced` 或 `Plain`）時，每一次命中都**一定帶有一個 https 網址**；沒命中時沒有網址（空）。
- Expanded 的歌詞區角落顯示一個小標籤（網站名稱加 `↗`），點下去用預設瀏覽器開啟該網址。沒命中就完全不顯示。
- 網址跟著歌詞走過 提供者 → 查詢總管（含快取）→ 控制器 → 畫面，每一層都不認得具體來源（解耦不變）。

**非目標**
- Preview 不顯示（它為了點擊穿透不接收滑鼠）。
- 不做「複製連結」、不做多個候選來源切換、不做歌詞編輯與黑名單（R5）。
- 不為了組網址多發任何網路請求。

**反目標**
- 把特定來源的細節（LRCLIB 或網易雲的 id）放進 `Lyrics`，或讓控制器與畫面認得它們。
- 開啟非 https 的位址，或開啟不是我們自己的提供者組出來的位址。
- 讓來源標籤蓋住歌詞、被下方圓角切到，或擠壞 Expanded 版面。

## 3. 做法（Approaches）

1. **在 `Lyrics` 裡加欄位。** 要動 64 處既有用法（11 個檔案），且 enum 沒有共用的地方放。
2. **另外包一層 `LyricsHit { lyrics, url }`。** `Lyrics` 不動；只有提供者、查詢總管、控制器、畫面四處的簽章多帶一個欄位。
3. **提供者另開一個「查網址」的方法。** 可能多一次請求，或與查詢結果不一致。

**已選：2（使用者決定）。**

## 4. 設計

### 4.1 型別（`rust/core/src/lyrics/`）

```rust
/// 一次命中：歌詞加上這份歌詞的網頁位址。命中一定有網址。
pub struct LyricsHit {
    pub lyrics: Lyrics,
    /// 一定是 https 開頭的位址（各提供者自己組；見 4.2）。
    pub url: String,
}
```
（衍生 `Clone, PartialEq, Debug`。）`Lyrics` 與 `LyricLine` 不變。不另設「來源名稱」欄位：畫面上的標籤文字由網址的主機名稱導出（純函式，只用於顯示；網址全都來自我們自己的提供者）。

### 4.2 各層

- **`LyricsProvider::fetch`** 回傳 `Result<Option<LyricsHit>, ProviderError>`（失敗／乾淨沒有／命中三態不變）。各提供者自己組網址，且保證命中時一定組得出：
  - LRCLIB：回應有有效的 `id`（正整數）時 `https://lrclib.net/api/get/<id>`（我實測這個位址確定回傳該筆的 JSON）；沒有有效 id 時退而用**我們剛剛發出去的那個請求位址**（精確查詢位址或搜尋位址，它就是產生這份歌詞的來源，必然可用）。提供者不需要向使用者解釋這個差別。
  - 網易雲：`https://music.163.com/song?id=<song_id>`（`song_id` 本來就有）。
- **`LyricsResolving::lyrics`** 回傳 `Option<LyricsHit>`。可信度把關只看 `hit.lyrics`。`CacheEntry::Found` 存整個 `LyricsHit`。快取鍵與 TTL 規則不變。純文字保底帶的是「保底那一個來源」的網址。
- **`SwitchableResolver`** 轉手，簽章跟著改。
- **`MusicController`／`MusicState`**：`MusicState.lyrics: Option<Lyrics>` **保持不變**，新增 `MusicState.lyrics_url: Option<String>`（沒命中 `None`）。控制器是唯一拆開 `LyricsHit` 的地方；換曲清空、過期結果丟棄、世代防護的規則對兩個欄位一起套用（永遠一起設定、一起清除）。
- **開啟連結**：新增 `rust/platform-macos/src/links.rs`（唯一的新 ObjC 檔）：`pub fn open_url(url: &str) -> bool`，只接受 `https://` 且可解析、有主機名稱、不含空白或控制字元、長度合理的位址，其餘回傳 false 並不開啟；用 `NSWorkspace.sharedWorkspace().openURL`；僅限主執行緒（`MainThreadMarker`）。
- **畫面（`shell`）**：`MusicShellContent::new` 多一個注入的 `open_url: Arc<dyn Fn(&str) + Send + Sync>`（app 傳入 `platform_macos::open_url`），shell 不碰 ObjC。Expanded 的**控制列左側空位**（現在是 `square()` 加左側 `flex_1` 的空白；放在歌詞區右下角會蓋到長歌詞行，所以改放這裡）顯示小標籤「<主機名稱> ↗」（例如 `lrclib.net ↗`、`music.163.com ↗`）：`lyrics_url` 為 `Some` 才顯示，點擊呼叫注入的 `open_url`；`None` 完全不顯示。字小、低對比，列高與按鈕位置不變。純文字歌詞同樣顯示。

### 4.3 隱私與安全

- 不新增任何網路請求；連結只在使用者點擊時由系統瀏覽器開啟。
- 網址只含提供者自己回傳的紀錄 id；唯一的例外是「LRCLIB 沒有有效 id 時退用我們發出去的請求位址」，該位址含曲目的標題與演出者，這些資料本來就已經送給了 lrclib.net，而且是在使用者自己的瀏覽器中開啟，不經過我們。
- 只開 https；網址只由我們的提供者組出，不接受其他來源的字串。
- README 的隱私說明補一句：「點擊歌詞來源連結會用預設瀏覽器開啟該網站」。

### 4.4 測試

- 提供者：LRCLIB 有 id → `api/get/<id>`；id 缺漏、為零、為負、非整數 → 退用請求位址；搜尋備援路徑同理；網易雲位址；既有的 48 個提供者測試語意不變（斷言改取 `hit.lyrics`）。
- 查詢總管：命中帶出網址；快取命中仍帶網址；純文字保底帶的是保底來源的網址；可信度不過的帶時間軸結果不會洩漏網址；既有 30 個測試語意不變。
- 控制器：歌詞與網址一起設定與清除；換曲時舊網址不殘留；過期（世代／鍵不符）結果不寫入網址；`MusicState` 預設兩者皆 `None`。
- `open_url`：純函式的網址驗證表（https 通過；http、file、javascript、空字串、含空白或控制字元、無主機、超長都拒絕）；不在單元測試中實際開啟瀏覽器。
- 畫面：由網址導出標籤文字的純函式（一般網址、含埠號、含 `www.`、含使用者資訊或奇怪輸入 → 退回完整主機或不顯示，不 panic）；版面常數放在 `music_layout.rs`（標籤高度、字級、與圓角淨空）並有測試確認落在淨空內。

### 4.5 文件

- `rust/README.md`：一行說明來源標籤與點擊行為，並補隱私一句。
- `rust/docs/HANDOFF-rust.md`：新增手動驗收項：點擊會開啟瀏覽器且面板不搶焦點、沒命中時沒有標籤、純文字歌詞也有標籤、版面不被切。

## 5. 我替使用者做的決定（請檢查）

- 命中一定有網址：LRCLIB 沒有有效 id 時退用「我們剛發出去的請求位址」。點開會是該請求回應的 JSON（精確查詢）或一份搜尋結果清單（搜尋備援），不是漂亮的頁面。
- LRCLIB 有 id 時用 `https://lrclib.net/api/get/<id>`（確定可用，點開是 JSON），而不是 `https://lrclib.net/db/<id>`：我實測後者對不存在的 id 也回傳 200 的通用外殼，無法確認它會顯示該筆曲目。若使用者確認 `/db/<id>` 在瀏覽器中確實顯示該筆，只需改提供者裡的一個函式。
- 不另存來源名稱，標籤用網址的主機名稱；位置在控制列左側空位（不蓋歌詞）、小字低對比；Preview 不顯示；只開 https。
- `MusicState` 新增並列欄位 `lyrics_url`，而非改 `lyrics` 的型別，讓既有的畫面與呈現邏輯零改動。

## 6. 留給實作者

- 標籤的確切字級、顏色、與歌詞區的間距（遵守下方圓角淨空）。
- 提供者回傳型別改動造成的測試調整方式。
- 約 4–5 個任務（型別與提供者、查詢總管與快取、控制器與狀態、`open_url` 與畫面、文件），沿用 subagent-driven 流程。

## 參考

- `rust/core/src/lyrics/{mod.rs,provider.rs,lrclib.rs,netease.rs,repository.rs}`、`rust/core/src/music/controller.rs`、`rust/app/src/switchable.rs`
- `rust/shell/src/{music_views.rs,music_layout.rs,lib.rs}`、`rust/platform-macos/src/{lib.rs,power.rs}`（ObjC 與主執行緒慣例）、`rust/app/src/main.rs`（組裝）
- `docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`（第 5.3、8 節）、`rust/docs/HANDOFF-rust.md`
