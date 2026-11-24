# 歌詞來源連結（LyricsHit）實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 歌詞命中時，Expanded 的控制列左側出現一個小標籤（網址的主機名稱加 `↗`），點下去用預設瀏覽器開啟該歌詞的來源網址；沒命中就沒有標籤。

**Architecture:** 在 `Lyrics`（不改）旁新增 `LyricsHit { lyrics, url }`。提供者回傳 `LyricsHit`（各自組網址），查詢總管與快取存整個 hit，控制器是唯一拆開它的地方，在 `MusicState` 新增並列的 `lyrics_url`；畫面讀 `lyrics_url`，點擊透過注入的 `open_url` 回呼，真正開瀏覽器的函式放在 `platform-macos`。

**Tech Stack:** Rust（工作區 `rust/`）、GPUI `=0.2.2`、`objc2`（只在 `platform-macos`）。

**Spec:** `docs/superpowers/specs/2026-11-24-lyrics-source-link-design.md`（已核准）。專案背景見 `docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md` 與 `rust/docs/HANDOFF-rust.md`。

## Global Constraints

- 在分支 `feature/lyrics-source-link` 工作（已建立，設計文件已在上面）。不得修改任何 Swift 檔案。
- `musicat-core` 不得有 `cfg(target_os)`，不得依賴平台、非同步執行環境或 HTTP 函式庫（`sh rust/scripts/check-core-purity.sh` 擋住）。
- 私有 API 只准出現在既有的兩處（`platform-macos/helper/`、`mediaremote_control.rs`）；`sh rust/scripts/check-private-api.sh` 必須維持通過。ObjC（`objc2`）只准出現在 `platform-macos/src/` 之下；本功能新增一個檔案 `links.rs`，使用公開的 `NSWorkspace`／`NSURL` API。`shell`、`app` 不得出現 ObjC。
- 網址一律 `https://` 開頭；`open_url` 拒絕其他任何字串。不得新增任何網路請求；User-Agent 維持 `MusiCat`。
- `Lyrics`、`LyricLine` 與 `lyrics_display`／`strip_content` 等呈現邏輯**不得改動**（既有測試的語意不變）。
- 版面：來源標籤放在 Expanded 控制列的**左側空位**（現在是 `square()` 加左側 `flex_1` 的空白），不得蓋住歌詞區，也不得進入下方圓角 24 pt 的淨空以外的地方造成裁切。（設計文件寫「歌詞區右下角」；實作改放控制列左側，因為歌詞區整個高度都是歌詞，右下角會蓋到長歌詞行。）
- 每個任務結束時 `cd rust && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && sh scripts/check-core-purity.sh && sh scripts/check-private-api.sh && sh scripts/parity.sh` 都必須通過，且 `cargo check -p musicat-platform-macos --target x86_64-pc-windows-msvc` 通過。建置可能需要 `--offline`。
- 文件用繁體中文。提交訊息結尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

1. LRCLIB 回應的 `id` 型別怪異（字串、浮點、0、負數、超大、`null`）：不得讓整筆回應變成壞回應，也不得 panic；一律退用請求位址（Task 1）。
2. 純文字保底帶的是「保底那一個來源」的網址，不是後面某個來源的；可信度不過的結果不得洩漏網址（Task 1）。
3. 換曲、過期（世代／鍵不符）的舊查詢結果不得讓上一首的網址殘留在新曲目上（Task 2）。
4. `open_url` 對 `javascript:`、`file:`、`http:`、含使用者資訊（`https://a@b`）、含空白或控制字元、無主機、超長字串一律拒絕（Task 3）。
5. 標籤文字函式對奇怪網址（埠號、`www.`、IDN／非 ASCII 主機、超長主機、空字串）不得 panic、不得讓標籤撐破版面（Task 4）。

---

## 檔案結構

- 改：`rust/core/src/lyrics/{mod.rs,provider.rs,lrclib.rs,netease.rs,repository.rs}`、`rust/core/src/music/controller.rs`
- 改：`rust/app/src/{switchable.rs,composition.rs,main.rs}`、`rust/app/examples/lyrics_demo.rs`
- 新：`rust/platform-macos/src/links.rs`；改 `rust/platform-macos/{Cargo.toml,src/lib.rs}`
- 改：`rust/shell/src/{music_views.rs,music_layout.rs}`、`rust/shell/examples/music_demo.rs`
- 改文件：`rust/README.md`、`rust/docs/HANDOFF-rust.md`、`CONTRIBUTING.md`

---

## Task 1：`LyricsHit`、兩個提供者、查詢總管與快取

提供者介面一改，下游（查詢總管、可切換查詢器、控制器、範例）都會編不過，所以這些要在同一個任務內一起回到綠燈；控制器在這個任務只做最小的機械式調整（暫時丟掉網址），Task 2 才把網址放進狀態。

**Files:**
- Modify: `rust/core/src/lyrics/{mod.rs,provider.rs,lrclib.rs,netease.rs,repository.rs}`、`rust/core/src/music/controller.rs`（只做最小調整）、`rust/app/src/{switchable.rs,composition.rs}`、`rust/app/examples/lyrics_demo.rs`、`rust/shell/examples/music_demo.rs`（假解析器的型別調整）

**Interfaces:**
- Produces（後續任務依賴）：
  ```rust
  // rust/core/src/lyrics/mod.rs
  #[derive(Clone, PartialEq, Debug)]
  pub struct LyricsHit { pub lyrics: Lyrics, pub url: String }   // url 一定是 https
  impl LyricsHit { pub fn new(lyrics: Lyrics, url: impl Into<String>) -> Self }
  // provider.rs
  pub trait LyricsProvider: Send + Sync {
      fn id(&self) -> &str;
      fn fetch<'a>(&'a self, track: &'a TrackIdentity)
          -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>>;
  }
  // repository.rs
  pub trait LyricsResolving: Send + Sync {
      fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<LyricsHit>>;
  }
  ```
  失敗／乾淨沒有／命中三態不變；`Lyrics`、`LyricLine` 不變；`CacheEntry::Found { hit: LyricsHit, until: Option<f64> }`；`Coalescer` 的值型別為 `Option<LyricsHit>`。

- [ ] **Step 1: 寫失敗的測試**
  - `lrclib.rs`：`lrclib_exact_hit_uses_the_record_id_url`（回應 `{"id":3396226,...,"syncedLyrics":"[00:01.00]a\n[00:02.00]b"}` → `hit.url == "https://lrclib.net/api/get/3396226"`）；`lrclib_hit_without_a_usable_id_falls_back_to_the_request_url`（`id` 缺漏／`null`／`0`／`-5`／`"abc"`／`7.5`／`1e30` 逐一：回應仍是有效命中，不是 `BadResponse`，`hit.url == LrclibProvider::exact_url(&track)`）；`lrclib_search_fallback_hit_uses_the_candidate_id_or_the_search_url`（精確查詢 404，搜尋第一筆歌名不符、第二筆符合且 `id: 42` → `https://lrclib.net/api/get/42`；候選沒有有效 id → `LrclibProvider::search_url(&track)`）。
  - `netease.rs`：`netease_hit_url_points_at_the_song_page`（選到 `song_id 7`、歌詞有效 → `https://music.163.com/song?id=7`）。
  - `repository.rs`：`a_hit_carries_the_providers_url`、`cached_hit_still_carries_its_url`、`plain_fallback_carries_the_fallback_providers_url_not_a_later_one`（A 純文字＋`https://a.example/1`、B 沒有 → 結果 url 是 A 的；A 純文字、B 帶時間軸 → 用 B 的 hit）、`implausible_synced_result_does_not_leak_its_url`（A 回可信度不過的 hit、B 沒有 → `None`）。
  - `switchable.rs`：`swap_changes_the_url_source_for_later_lookups`。
  - 既有測試（提供者 48 個、查詢總管 30 個、可切換查詢器、控制器）語意不變：假提供者與假解析器改持有 `Option<LyricsHit>`，加小幫手 `hit(lyrics)`（url 固定如 `https://test.example/<id>`）；提供者測試的斷言改取 `hit.lyrics`。
- [ ] **Step 2: Run** `cd rust && cargo test -p musicat-core lyrics::`；Expected: FAIL / 編譯失敗（`LyricsHit` 未定義）。
- [ ] **Step 3: 型別與介面**：在 `lyrics/mod.rs` 新增 `LyricsHit` 並匯出；`LyricsProvider::fetch`、`LyricsResolving::lyrics` 改成上面的簽章。
- [ ] **Step 4: 實作 LRCLIB**：`Payload` 新增 `id`，用寬鬆的反序列化函式（`fn lenient_positive_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error>`：整數且大於 0 才是 `Some`，其餘任何型別或值一律 `Ok(None)`，**不得讓整筆回應失敗**）；新增 `fn record_url(id: Option<i64>, request_url: &str) -> String`（有 id → `https://lrclib.net/api/get/<id>`，否則 `request_url`）；精確查詢路徑傳 `exact_url(track)`，搜尋路徑傳候選自己的 id 或 `search_url(track)`。
- [ ] **Step 5: 實作網易雲**：`pub(crate) fn song_url(song_id: i64) -> String`；`fetch_lyrics` 在歌詞有效時回 `LyricsHit`。
- [ ] **Step 6: 實作查詢總管**：`resolve` 的 `found`／`fallback` 改為 `Option<LyricsHit>`；可信度檢查只看 `hit.lyrics`；快取與 `Coalescer` 型別跟著改。`switchable.rs`、`composition.rs`、控制器（暫時以 `result.map(|h| h.lyrics)` 取歌詞並加註解「Task 2 會一併保存網址」）、兩個範例只做簽章與型別調整；`lyrics_demo` 順便多印一行 `url: <hit.url>`。
- [ ] **Step 7: Run** `cd rust && cargo test --workspace` 與完整檢查；Expected: 全部 PASS（既有測試語意不變）。
- [ ] **Step 8: Commit** `feat(core): LyricsHit carries the source URL from providers through repository and cache`

## Task 2：控制器與 `MusicState.lyrics_url`

**Files:**
- Modify: `rust/core/src/music/controller.rs`、`rust/shell/examples/music_demo.rs`（假解析器改回傳帶網址的 hit）

**Interfaces:**
- Consumes: Task 1 的 `LyricsResolving`。
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Debug, Default)]
  pub struct MusicState {
      pub snapshot: Option<TrackSnapshot>,
      pub lyrics: Option<Lyrics>,         // 不變
      pub lyrics_url: Option<String>,     // 新增；沒命中為 None
  }
  ```

- [ ] **Step 1: 寫失敗的測試（`controller.rs`）**
  - `hit_sets_lyrics_and_url_together`、`track_change_clears_lyrics_and_url_together`、`no_artist_or_no_snapshot_clears_the_url`。
  - `slow_lookup_for_the_previous_track_never_leaves_its_url_on_the_new_track`（沿用既有「A 卡住、切到 B 完成、再放行 A」情境，斷言 `lyrics_url` 仍是 B 的）、`stale_generation_result_does_not_write_the_url`（沿用既有的不取消的 spawner 與 A→B→A 情境）。
  - `subscribers_are_notified_when_only_the_url_changes`（同一份歌詞但網址不同 → 通知一次；完全相同的重複結果不通知）。
- [ ] **Step 2: Run** `cargo test -p musicat-core --features testing controller`；Expected: FAIL。
- [ ] **Step 3: 實作**：控制器是唯一拆開 `LyricsHit` 的地方；寫入、清除、世代與鍵的檢查對 `lyrics` 與 `lyrics_url` 在同一個鎖內一起做；「沒變就不通知」的判斷（原本 `core.state.lyrics == result`）改為兩個欄位都相同才算沒變。移除 Task 1 的暫時註解。
- [ ] **Step 4: `music_demo`**：假的歌詞解析器回傳帶 `https://lrclib.net/api/get/1` 的 hit；`none`／`nolyrics` 場景維持沒有 hit。
- [ ] **Step 5: Run** `cargo test -p musicat-core --features testing controller && cargo build -p musicat-shell --examples --offline` 與完整檢查；Expected: PASS。
- [ ] **Step 6: Commit** `feat(core): MusicState.lyrics_url set and cleared together with the lyrics`

## Task 3：`open_url`（platform-macos）

**Files:**
- Create: `rust/platform-macos/src/links.rs`
- Modify: `rust/platform-macos/src/lib.rs`（`pub mod links;`）、`rust/platform-macos/Cargo.toml`（`objc2-foundation` 加 `NSURL` 特性；`objc2-app-kit` 已有 `NSWorkspace`）、`CONTRIBUTING.md`（把 `links.rs` 加進 objc2 允許的檔案清單）

**Interfaces:**
- Produces:
  ```rust
  /// 純函式（所有平台都編譯）：通過回傳原字串，否則 None。
  pub fn validate_open_url(url: &str) -> Option<&str>;
  /// 僅 macOS、僅主執行緒；通過驗證才用 NSWorkspace 開啟。回傳是否已交給系統開啟。
  #[cfg(target_os = "macos")] pub fn open_url(url: &str) -> bool;
  ```
  規則：必須以小寫 `https://` 開頭；主機名稱非空且不含 `@`（拒絕使用者資訊）；整串不含空白或控制字元（`char::is_control`／`is_whitespace`）；長度 ≤ 2048；不可解析為有效 `NSURL` 時 `open_url` 回 `false`。

- [ ] **Step 1: 寫失敗的測試（純函式，所有平台）**：`validate_open_url` 接受 `https://lrclib.net/api/get/1`、`https://music.163.com/song?id=7`、含埠號與 `#` 的位址；拒絕 `http://x.com`、`HTTPS://x.com`（大小寫不符亦拒絕）、`javascript:alert(1)`、`file:///etc/passwd`、`https://`、`https:///path`、`https://a@b.com/`、`https://user:pw@b.com/`、含空白、含換行與 NUL、空字串、2049 字元、`https://exa mple.com`。
- [ ] **Step 2: Run** `cd rust && cargo test -p musicat-platform-macos links`；Expected: FAIL。
- [ ] **Step 3: 實作** `validate_open_url`；macOS 的 `open_url`：`MainThreadMarker::new()`（非主執行緒 → 印一行警告、回 `false`），`NSURL::URLWithString`（回 `None` → `false`），`NSWorkspace::sharedWorkspace().openURL(&url)`；檔頭註解沿用 `power.rs` 的風格（說明只用公開 API、執行緒要求、為何只放行 https）。不在單元測試中實際開啟任何東西。
- [ ] **Step 4: Run** 同 Step 2，再跑完整檢查與 Windows 檢查（純函式與常數須在 Windows 目標上編譯；ObjC 部分 `cfg(target_os = "macos")`）；Expected: PASS。
- [ ] **Step 5: Commit** `feat(platform-macos): open_url with https-only validation`

## Task 4：畫面標籤、注入開啟回呼與文件

**Files:**
- Modify: `rust/shell/src/music_views.rs`、`rust/shell/src/music_layout.rs`、`rust/app/src/main.rs`、`rust/shell/examples/music_demo.rs`、`rust/README.md`、`rust/docs/HANDOFF-rust.md`

**Interfaces:**
- Consumes: Task 2 的 `MusicState.lyrics_url`、Task 3 的 `platform_macos::links::open_url`。
- Produces:
  ```rust
  // music_layout.rs（純函式與常數，有測試）
  pub fn source_label(url: &str) -> Option<String>;   // "lrclib.net ↗"；主機取自網址，去掉開頭的 "www."，保留埠號之外的主機；解析不出主機 → None；主機超過 SOURCE_LABEL_MAX_CHARS 時以 "…" 截斷
  pub const SOURCE_LABEL_FONT_SIZE: f64;   // 9.0（建議）
  pub const SOURCE_LABEL_OPACITY: f64;     // 0.45（建議，低對比）
  pub const SOURCE_LABEL_MAX_CHARS: usize; // 24（建議）
  // music_views.rs
  MusicShellContent::new(cx, geometry, controller, module_state, modules, clock,
                         open_url: Arc<dyn Fn(&str) + Send + Sync>) -> Self
  ```

- [ ] **Step 1: 寫失敗的測試（`music_layout.rs`）**：`source_label_uses_the_host_and_an_arrow`（`https://lrclib.net/api/get/1` → `lrclib.net ↗`；`https://music.163.com/song?id=7` → `music.163.com ↗`）、`www_prefix_is_dropped`、`port_userinfo_query_and_fragment_are_ignored`、`very_long_host_is_truncated_with_an_ellipsis`（結果字元數 ≤ `SOURCE_LABEL_MAX_CHARS + 2`，含 ` ↗`）、`non_ascii_host_does_not_panic_or_split_a_char`、`garbage_returns_none`（空字串、`https://`、`not a url`、`https:///x`）。再加一個版面測試：標籤列高度 ≤ `CONTROL_ROW_HEIGHT`，且標籤位於控制列的左側空位內（不與按鈕群重疊：以現有常數計算左側可用寬度 ≥ 標籤最大寬度的估算，或直接斷言標籤容器使用 `flex_1` 的左側空位而不增加列高）。
- [ ] **Step 2: Run** `cargo test -p musicat-shell music_layout`；Expected: FAIL。
- [ ] **Step 3: 實作純函式與常數**（不用 `url` crate：手動取 `https://` 之後到第一個 `/`、`?`、`#` 之前的部分，去掉 `userinfo@` 與 `:埠號`，以字元（非位元組）截斷）。
- [ ] **Step 4: 畫面**：`MusicShellContent::new` 多收 `open_url`，存進 `Shared`；Expanded 控制列左側（取代左邊的 `square()` 與左 `flex_1`）放一個左對齊的容器：`state.lyrics_url` 為 `Some` 且 `source_label` 為 `Some` 時顯示小標籤（字級、不透明度用常數，`id("music-source-link")`），`on_click` 呼叫 `open_url(&url)`（url 取自點擊當下的狀態，不要在閉包裡抓過期值）；游標在標籤上用手形游標（若 GPUI 0.2.2 支援；否則略過並在報告說明）；`None` 時只留空白，列高與按鈕位置**完全不變**。
- [ ] **Step 5: 組裝**：`app/src/main.rs` 傳入 `Arc::new(|url| { musicat_platform_macos::links::open_url(url); })`；`music_demo` 傳入「印出 `open <url>`、不真的開瀏覽器」的回呼，讓示範可以在不開網頁的情況下驗證點擊。其他呼叫端（若有）一併更新。
- [ ] **Step 6: 文件**：`rust/README.md` 加一句（來源標籤與「點擊會用預設瀏覽器開啟該網站」，隱私一節補一句：點擊連結由瀏覽器自己連線，MusiCat 不經手）；`rust/docs/HANDOFF-rust.md` 新增「歌詞來源連結」手動驗收項：(a) 有命中時標籤顯示且點擊會開瀏覽器、面板不搶焦點；(b) 沒命中（`--scene none`、`nolyrics`）沒有標籤、控制列版面不變；(c) 純文字歌詞（`--scene plain`）也有標籤；(d) 超長歌名或窄螢幕時標籤不蓋到按鈕；(e) LRCLIB 沒有有效 id 時點開的是請求位址（JSON 或搜尋結果清單）屬預期。
- [ ] **Step 7: Run** `cd rust && cargo test -p musicat-shell && cargo build -p musicat-shell --examples --offline && cargo build --release -p musicat-app --offline` 與完整檢查；若螢幕醒著且已解鎖，執行一次 `music_demo`（預設場景，約 8 秒內）確認標籤出現並用滑鼠點一次（會看到 `open <url>` 輸出）——**移動游標最多一次**，否則把視覺與點擊檢查標為待作者確認。
- [ ] **Step 8: Commit** `feat(shell): clickable lyrics source label in the Expanded control row`

---

## 自我檢查

- **規格覆蓋：** 命中一定有網址／沒命中為空（Task 1、2）；`LyricsHit` 與解耦（Task 1、2，`Lyrics` 不動）；開啟只允許 https（Task 3）；標籤只在 Expanded、無網址不顯示（Task 4）；不新增網路請求（無任務新增請求）；README、HANDOFF、CONTRIBUTING 更新（Task 3、4）。
- **型別一致：** `LyricsHit { lyrics, url: String }` 在 Task 1 定義，Task 1 的 `LyricsResolving` 與快取、Task 2 的拆開都使用同名；`MusicState.lyrics_url: Option<String>` 與 Task 4 的讀取一致；`open_url` 的兩個簽章（純驗證函式回傳 `Option<&str>`、動作函式回傳 `bool`）與 Task 4 的注入型別 `Arc<dyn Fn(&str) + Send + Sync>` 一致。
- **已知取捨：** Task 1 的介面變更會讓下游同時編不過，所以查詢總管、可切換查詢器、控制器的最小調整都放在 Task 1 內一起回到綠燈（控制器暫時丟掉網址，Task 2 補上）。標籤放在控制列左側，與設計文件的「歌詞區右下角」不同，原因寫在 Global Constraints（設計文件也需要同步更新一行）。
