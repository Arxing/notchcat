# 歌詞來源連結（LyricsHit）設計

寫於 2026-11-24。這是 MusiCat（`rust/`）v0.1.0 之後的第一個小功能。讀者是要規劃並實作的 agent，沒有參與過先前的對話；請先讀完本文，再讀「參考」一節的檔案。

## 1. 意圖

歌詞命中時，使用者想在 Expanded 裡有一個地方可以點，直接用預設瀏覽器開啟這份歌詞的來源頁面。（使用者沒有說明目的；最可能是想確認歌詞對不對，或知道它從哪個網站來。設計因此同時顯示來源名稱與可點的連結。）

## 2. 目標、非目標、反目標

**目標**
- 歌詞命中（`Synced` 或 `Plain`）時，Expanded 的歌詞區角落顯示一個小標籤「來源：<名稱> ↗」；有連結時可點，點下去用預設瀏覽器開啟。
- 來源資訊跟著歌詞走過 提供者 → 查詢總管（含快取）→ 控制器 → 畫面，每一層都不認得具體來源（解耦不變）。
- 沒有命中、或命中但來源沒有連結時，不顯示可點的東西（沒有連結時只顯示名稱，不可點）。

**非目標**
- Preview 不顯示來源（Preview 為了點擊穿透，不接收滑鼠）。
- 不做「複製連結」、不做多個候選來源的切換、不做歌詞編輯與「這份不對」黑名單（那是 R5）。
- 不為了組連結多發任何網路請求。

**反目標（即使技術上成功也算失敗）**
- 把特定來源的細節（LRCLIB 的 id、網易雲的 id）塞進 `Lyrics` 或讓 UI／控制器認得它們。
- 讓點擊連結開啟非 https 的位址，或開啟不是我們自己的提供者組出來的位址。
- 讓 Expanded 的版面被擠壞（來源標籤必須落在下方圓角的 24 pt 淨空之內不會被切到）。

## 3. 做法（Approaches）

1. **在 `Lyrics` 裡加欄位。** 最簡單的概念，但 `Lyrics` 是兩個變體的 enum，沒有共用的地方放；兩種改法（每個變體各加、或改成 struct 包住內容）都要動 64 處既有用法（11 個檔案）。且 `PartialEq` 會把來源算進相等比較。
2. **另外包一層 `LyricsHit { lyrics, source }`。** `Lyrics` 完全不動；只有提供者、查詢總管、控制器、畫面這四處的簽章多帶一個欄位。來源特有的細節（網址怎麼組）留在各提供者裡。
3. **提供者另開一個「查來源連結」的方法。** 會讓查詢與取連結分開，之後可能多一次請求或讓兩者不一致。

**已選：2（使用者決定）。** 理由：`Lyrics` 保持乾淨，改動只經過必要的四層，且與 spec 第 8 節「來源清單最前面可以插入本地來源而不改其他程式碼」一致（本地來源之後自己決定給不給連結）。

## 4. 設計

### 4.1 型別（`rust/core/src/lyrics/`）

```rust
/// 歌詞的出處。只含通用資料；沒有任何特定來源的欄位。
pub struct LyricsSource {
    /// 給人看的名稱，例如 "LRCLIB"、"網易雲音樂"。
    pub name: String,
    /// 該筆資料的網頁位址；來源沒有或組不出來時為 None。必須是 https。
    pub url: Option<String>,
}

/// 一次命中：歌詞加出處。
pub struct LyricsHit {
    pub lyrics: Lyrics,
    pub source: LyricsSource,
}
```
（衍生 `Clone, PartialEq, Debug`。）`Lyrics` 與 `LyricLine` 不變。

### 4.2 各層

- **`LyricsProvider::fetch`** 改為回傳 `Result<Option<LyricsHit>, ProviderError>`（失敗／乾淨沒有／命中三態不變）。各提供者自己組 `LyricsSource`：
  - LRCLIB：解析回應裡的 `id`（整數，缺漏或非正數則 `url = None`），`name = "LRCLIB"`，`url = https://lrclib.net/api/get/<id>`。搜尋備援路徑命中時同樣用候選的 `id`。
  - 網易雲：已經有 `song_id`，`name = "網易雲音樂"`，`url = https://music.163.com/song?id=<id>`。
- **`LyricsResolving::lyrics`** 改為回傳 `Option<LyricsHit>`。可信度把關仍只看 `hit.lyrics`。`CacheEntry::Found` 存整個 `LyricsHit`。快取鍵與 TTL 規則不變。
- **`SwitchableResolver`** 只是轉手，簽章跟著改。
- **`MusicController`／`MusicState`**：`MusicState.lyrics: Option<Lyrics>` **保持不變**，新增 `MusicState.lyrics_source: Option<LyricsSource>`。控制器是唯一拆開 `LyricsHit` 的地方；換曲清空、過期結果丟棄、世代防護的規則對兩個欄位一起套用（永遠一起設定、一起清除）。這樣 `presentation.rs` 與大部分畫面程式碼完全不用動。
- **開啟連結**：新增 `rust/platform-macos/src/links.rs`（唯一的新 ObjC 檔），`pub fn open_url(url: &str) -> bool`：只接受 `https://` 開頭、可解析的位址（其他一律回傳 false 並不開啟）；用 `NSWorkspace.sharedWorkspace().openURL`；僅限主執行緒（`MainThreadMarker`）。
- **畫面（`shell`）**：`MusicShellContent::new` 多一個注入的 `open_url: Arc<dyn Fn(&str) + Send + Sync>`（app 傳入 `platform_macos::open_url`），讓 shell 不碰 ObjC、也方便測試。Expanded 的歌詞區右下角（落在下方圓角 24 pt 淨空之內）顯示小標籤：有 `lyrics_source` 才顯示；有 `url` → 可點、帶 `↗`，點擊呼叫注入的 `open_url`；沒有 `url` → 只顯示名稱、不可點、不帶 `↗`。字小、低對比，不搶歌詞。純文字歌詞（`Plain`）同樣顯示來源。

### 4.3 隱私與安全

- 不新增任何網路請求；連結只在使用者點擊時由系統瀏覽器開啟。
- 網址只含提供者自己回傳的紀錄 id，不含使用者的曲目資訊。
- 只開 https；網址只由我們的提供者組出，不接受來自其他地方的字串。
- README 的隱私說明不需要改（沒有新的外送資料）；但要補一句「點歌詞來源連結會用預設瀏覽器開啟該網站」。

### 4.4 測試

- 提供者：LRCLIB 的 id 存在／缺漏／零／負／非整數時的 `url`；搜尋備援路徑也帶 id；網易雲的 `url`；既有的 48 個提供者測試語意不變（只調整斷言取 `hit.lyrics`）。
- 查詢總管：命中帶出 `LyricsSource`；快取命中仍帶來源；純文字保底帶的是「保底那一個來源」的來源（不是後面某個來源的）；可信度不過的帶時間軸結果不會洩漏來源；既有 30 個測試語意不變。
- 控制器：歌詞與來源一起設定與清除；換曲時舊來源不殘留；過期（世代／鍵不符）結果不會寫入來源；`MusicState` 預設兩者皆 None。
- `open_url`：純函式的網址驗證（https 通過；http、file、javascript、空字串、含空白或控制字元、超長、無主機的都拒絕）；不在單元測試中實際開啟瀏覽器。
- 畫面：標籤文字與可點狀態的純函式（名稱加有無連結 → 顯示文字、是否可點）；版面常數放在 `music_layout.rs`（標籤高度、字級、與下方圓角淨空的關係）並有測試確認落在淨空內。

### 4.5 文件

- `rust/README.md`：一行說明來源標籤與「點擊會用預設瀏覽器開啟」。
- `rust/docs/HANDOFF-rust.md`：新增手動驗收項：有連結的點擊會開啟瀏覽器且面板不搶焦點、無連結只顯示名稱、純文字歌詞也顯示來源、版面不被切。

## 5. 我替使用者做的決定（請檢查）

- **LRCLIB 的連結用 API 位址 `https://lrclib.net/api/get/<id>`**，而不是看起來比較友善的 `https://lrclib.net/db/<id>`。原因：我實測 `/db/<任何 id>` 一律回傳 200 的通用應用程式外殼（連不存在的 id 也是），無法確認它真的會顯示該筆曲目；`/api/get/<id>` 則確定回傳該筆完整的 JSON 紀錄（含歌詞與中繼資料）。代價：點開是 JSON 而不是漂亮的頁面。若使用者實測 `/db/<id>` 在瀏覽器裡確實顯示該筆，只需改提供者裡的一個函式。
- **網易雲用 `https://music.163.com/song?id=<id>`**（實測回傳 200 的 HTML）。
- 標籤文字「來源：<名稱> ↗」、位置在歌詞區右下角、小字低對比；Preview 不顯示。
- 只開 https。
- `MusicState` 新增並列欄位而非把 `lyrics` 改型別，是為了讓既有的畫面與呈現邏輯零改動。

## 6. 留給實作者

- 標籤的確切字級、顏色、與歌詞區的間距（遵守下方圓角淨空）。
- 提供者回傳型別改動造成的測試調整方式。
- `LyricsSource` 要不要實作 `Default`／輔助建構函式。
- 工作量與流程：約 4–5 個任務（型別與提供者、查詢總管與快取、控制器與狀態、`open_url` 與畫面、文件），沿用 subagent-driven 流程。

## 參考

- `rust/core/src/lyrics/{mod.rs,provider.rs,lrclib.rs,netease.rs,repository.rs}`、`rust/core/src/music/controller.rs`、`rust/app/src/switchable.rs`
- `rust/shell/src/{music_views.rs,music_layout.rs,lib.rs}`、`rust/platform-macos/src/{lib.rs,power.rs}`（ObjC 與主執行緒慣例）、`rust/app/src/main.rs`（組裝）
- `docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`（第 5.3、8 節）、`rust/docs/HANDOFF-rust.md`
