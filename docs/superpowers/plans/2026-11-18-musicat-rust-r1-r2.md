# MusiCat Rust 重寫 R1 + R2 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `rust/` 以 Cargo workspace 重寫 NotchCat 為 MusiCat：R1 完成與平台無關的 `musicat-core`（含逐案移植的測試與開源骨架），R2 完成 macOS 平台層與 GPUI 瀏海外殼，行為與 Swift 版一致並可交給作者跑實機驗收。

**Architecture:** `core` 只放邏輯與抽象介面（`Clock`、`HttpClient`、`Spawner`、`NowPlayingSource`、`PlaybackControl`、`LyricsProvider`），不依賴任何作業系統 API、非同步執行環境或 HTTP 函式庫；非同步以 `Future` 加注入的 `Clock` 表達。連瀏海幾何、螢幕選擇、hover 狀態機這些「外殼邏輯」也放在 `core`（純函式、可單元測試，且 Windows CI 只跑 `core` 測試，這樣它們在兩個平台都被驗證）。`platform-macos` 實作現正播放（perl 宿主＋C 動態庫）、播放控制、視窗標題備援、螢幕幾何與 `NSPanel` 繞路；`shell` 用 GPUI 畫外殼與 Music 內容；`app` 組裝、讀 TOML、放選單列圖示。

**Tech Stack:** Rust stable（本機 1.95，edition 2021）、GPUI `=0.2.2`（`runtime_shaders`）、`objc2` 0.6、`futures-util`（只開 `default-features = false`）、`serde`／`serde_json`、`toml`、`zhconv`（簡繁折疊，須驗證，見 Task 5）、`ureq`（只在 `app`）、`cc`（build.rs 編 C 動態庫）。

**Spec:** [`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`](../specs/2026-11-06-cross-platform-rust-design.md)（以下簡稱「spec」）。行為依據是 `docs/superpowers/specs/2026-10-14-music-module-design.md` 與 `Tests/NotchCatKitTests/`；Swift 原始碼（`Sources/NotchCatKit/`）是讀不懂規格時的對照。

**範圍：** R1、R2 逐任務；R3、R4、R5 只寫到里程碑（見文末），等 R2 通過實機驗收後再細化（spec 第 13 節）。

## Global Constraints

- 在新分支 `feature/musicat-rust`（由目前的 `feature/music-module` 切出）工作。只新增 `rust/` 與 `.github/` 下的檔案，並在 `docs/` 新增文件；**不得修改或刪除任何 Swift 檔案、`Package.swift`、`scripts/run.sh`**。Swift 版移除由作者實機驗收後下令，builder 不得自行刪除。
- 名稱一律 `MusiCat`／`musicat`；crate 使用 `musicat-` 前綴：`musicat-core`、`musicat-platform-macos`、`musicat-platform-windows`、`musicat-shell`、`musicat-app`（可執行檔名 `musicat`）。目錄為 `rust/core`、`rust/platform-macos`、`rust/platform-windows`、`rust/shell`、`rust/app`。
- **`musicat-core` 硬規則：** 不得有 `cfg(target_os)`；不得直接或間接依賴 `tokio`、`async-std`、`smol`、`reqwest`、`hyper`、`ureq`、`gpui`、`objc2*`、`windows`、`core-foundation`；時間、網路、排程、背景工作一律透過注入的 `Clock`／`HttpClient`／`Spawner`。由 `rust/scripts/check-core-purity.sh` 在 CI 擋住。
- 授權 `MIT OR Apache-2.0`；所有 crate 的 `license` 欄位照此填，根目錄放 `LICENSE-MIT` 與 `LICENSE-APACHE`。User-Agent 固定為 `MusiCat`，不得帶任何使用者資料。
- 歌詞：每個來源 5 秒逾時，逾時視同失敗；曲長比對 ±5 秒且僅雙方都有且大於 0 時才比；歌名比對須容忍簡繁、大小寫、全半形；帶時間軸的結果至少 2 行，已知曲長（大於 0）時最後一行 ≤ 曲長 + 5 秒，否則視同該來源沒有；快取：成功永久、全部乾淨回報「沒有」24 小時（86400 秒）、有來源失敗 120 秒；純文字只當保底；提供者必須區分「失敗（回傳 `Err`）」與「乾淨地沒有（回傳 `Ok(None)`）」。
- 播放位置：`position = max(0, elapsed + max(0, now − timestamp) × rate)`，已知總長度時不超過總長度；位置未知時**絕不拿 0 充數**，沒有位置就不捲動歌詞、不畫進度條。
- 外殼數值（照抄 spec 第 5.3 節）：Preview 與 Expanded 同寬 = 瀏海寬 + 160；Preview 高 = 瀏海高 + 模組額外高度（clamp 到 0…28）；Expanded 高 180；上方兩角直角，下方圓角 Preview 10、Expanded 24；動畫 0.28 秒 ease-in-out；離開延遲 150 毫秒才收合；展開後 hover 範圍立刻放大到整個固定容器；每次新展開把模組切換列選取重設為 active 模組；歌詞條高 24、黑底不透明度 0.6（需實機調整，各寫成單一具名常數）；視窗外框終身不變，只有畫出來的內容做動畫。虛擬瀏海預設 200 × 32（可在 TOML 調整）。
- 私有 API 只准出現在兩處：`rust/platform-macos/src/mediaremote_control.rs`（只送控制指令）與 `rust/platform-macos/helper/`（C，讀取，只在 perl 行程內跑）。`objc2` 視窗繞路只准出現在 `rust/platform-macos/src/panel.rs`，每個繞路附註解寫明「為什麼需要」與「對應 GPUI 0.2.2」。`shell` 與 `app` 不碰 Objective-C。
- GPUI 以 `=0.2.2` 鎖死，且啟用 `runtime_shaders`；升級是獨立任務，不在本計畫。
- Windows 程式碼一律在檔頭與文件標示「未在實機驗證」；Windows CI 只要求編譯通過加 `musicat-core` 測試。
- 移植測試的規則：Swift 測試的案例與期望值照抄，Rust 測試名稱為 Swift 測試名的 `snake_case`；每個任務完成時更新 `rust/parity.tsv`（Swift 檔、Rust 位置、Swift 案例數、豁免理由），`rust/scripts/parity.sh` 在 Rust `#[test]` 數少於 Swift 案例數且無豁免理由時失敗。
- 每個任務結束時 `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 必須通過才能提交（Task 1 之前的骨架除外）。提交訊息結尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

spec 隱含、但沒有任務的主要測試會自然涵蓋的輸入，依「使用者實際踩到的機率」排序；每條都已加進對應任務。

1. **前一首的歌詞晚於後一首回來。** 快速連切歌時，舊請求完成後不得覆蓋新歌的歌詞，被放棄的請求不得被快取成「查無」。測試歸 Task 7、Task 10。
2. **歌詞檔畸形或惡意。** 未排序、重複時間標籤、空歌詞行、超大數字、百萬字元單行、完全沒有時間標籤卻被當成同步，都不得崩潰且須線性時間；回應本體過大（超過 4 MiB）視為失敗而非吃光記憶體。測試歸 Task 4、Task 6。
3. **helper 輸出被截斷或夾雜垃圾。** 一行中途被切開、非 UTF-8 位元組、非 JSON 雜訊、超過 4 MiB 的單行，不得崩潰、不得把半截字串當曲目，且不得影響前後的合法行。測試歸 Task 14。
4. **選定的螢幕被拔掉或闔上。** 設定裡的螢幕 id 已不存在、螢幕列表暫時為空、螢幕回來時，面板移到預設螢幕、程式不結束，回來時再移回。測試歸 Task 19。
5. **TOML 設定檔損毀、缺欄位或有未知欄位。** 一律以預設值啟動並在日誌印警告，不得崩潰、不得覆寫使用者的檔案。測試歸 Task 25。

---

## 檔案結構

```
rust/
  Cargo.toml  rust-toolchain.toml  parity.tsv
  scripts/ check-core-purity.sh  parity.sh  run.sh  measure.sh
  docs/ adr-0001-async-model.md  HANDOFF-rust.md
  core/src/
    lib.rs  types.rs  artwork_cache.rs  estimator.rs
    runtime.rs      # Clock, Spawner, TaskHandle, BoxFuture, Notifier, Subscription
    timing.rs       # with_timeout, Coalescer
    http.rs         # HttpClient, HttpRequest, HttpResponse, HttpError
    lyrics/ mod.rs(Lyrics,LRC 解析,plausible) identity.rs normalizer.rs provider.rs
            lrclib.rs netease.rs repository.rs
    nowplaying/ mod.rs(NowPlayingSource,PlaybackControl) monitor.rs window_title.rs
    music/ presentation.rs controller.rs module.rs
    shell_logic/ geometry.rs screens.rs hover.rs
    testing.rs      # FakeClock, FakeHttp, LocalSpawner（feature "testing"）
  platform-macos/ build.rs  helper/(helper.c, helper.h)
    src/ lib.rs helper_wire.rs supervisor.rs perl_launcher.rs
         mediaremote_control.rs window_title_source.rs screens.rs panel.rs tray.rs
  platform-windows/ src/lib.rs        # 只有可編譯的骨架
  shell/ src/ lib.rs window.rs notch_view.rs music_views.rs marquee.rs
  app/ src/ main.rs config.rs http_ureq.rs
.github/workflows/rust.yml
LICENSE-MIT  LICENSE-APACHE  README.md  CONTRIBUTING.md
```

---

# R1：核心

## Task 1：Workspace、授權、CI、核心純淨檢查

**Files:**
- Create: `rust/Cargo.toml`、`rust/rust-toolchain.toml`、各 crate 的 `Cargo.toml` 與空的 `lib.rs`（`app` 為空的 `main.rs`）、`rust/scripts/check-core-purity.sh`、`rust/scripts/parity.sh`、`rust/parity.tsv`、`.github/workflows/rust.yml`、`LICENSE-MIT`、`LICENSE-APACHE`、`README.md`、`CONTRIBUTING.md`
- Modify: `.gitignore`（加 `rust/target/`）

**Interfaces:**
- Produces: 五個 crate 名稱與路徑（見 Global Constraints）；`musicat-core` 有 feature `testing`（公開 `testing` 模組給其他 crate 的測試用）；`parity.tsv` 欄位以 tab 分隔：`swift_file  rust_location  swift_case_count  phase(r1|r2)  waiver_reason`。

- [ ] **Step 1: 建立 workspace。** `rust/Cargo.toml` 為 `resolver = "2"` 的 workspace，`[workspace.package]` 放 `edition = "2021"`、`license = "MIT OR Apache-2.0"`、`version = "0.1.0"`；`rust-toolchain.toml` 用 `stable` 並含 `rustfmt`、`clippy`。`platform-macos`、`shell`、`app` 目前只含空殼。`platform-windows` 的 `lib.rs` 檔頭註解標示「未在實機驗證」，內容只有一個 `pub fn crate_name() -> &'static str`。
- [ ] **Step 2: 寫失敗的純淨檢查。** `check-core-purity.sh`：(a) `grep -rn 'target_os' rust/core/src` 有輸出即失敗；(b) `cargo tree -p musicat-core --edges normal --prefix none` 的輸出含禁用清單（Global Constraints）任一 crate 即失敗。先在 `core/Cargo.toml` 暫時加 `ureq` 依賴確認腳本失敗，再移除。
- [ ] **Step 3: 寫 `parity.sh`。** 讀 `parity.tsv`，對每一列計算 `rust_location`（檔案或目錄）中 `#[test]` 的數量，少於 `swift_case_count` 且 `waiver_reason` 為空則印出該列並以非零離開；可用參數 `r1` 只檢查 R1 的列。把 Swift 的 21 個檔填進去（案例數見下表，`rust_location` 先填各任務規劃的路徑），`PackageSmokeTests` 豁免理由寫「Swift 套件結構測試」。此時預期 `parity.sh` 失敗；這是 Task 12 的收尾條件。

  | Swift 測試檔 | 案例數 | Rust 位置（任務） |
  |---|---|---|
  | PositionAnchorTests／PlaybackEstimatorTests／ArtworkCacheTests | 7／7／6 | `core/src/types.rs`、`estimator.rs`、`artwork_cache.rs`（T3） |
  | LyricsTests | 41 | `core/src/lyrics/mod.rs`（T4） |
  | TrackIdentityTests／TrackNormalizerTests | 10／15 | `lyrics/identity.rs`、`normalizer.rs`（T5） |
  | ProviderTests | 48 | `lyrics/lrclib.rs`、`netease.rs`、`provider.rs`（T6） |
  | LyricsRepositoryTests | 30 | `lyrics/repository.rs`（T7） |
  | NowPlayingMonitorTests／WindowTitleParserTests | 12／7 | `nowplaying/monitor.rs`、`window_title.rs`（T8） |
  | MusicPresentationTests | 12 | `music/presentation.rs`（T9） |
  | MusicViewModelTests | 10 | `music/controller.rs`（T10） |
  | ModuleManagerPreviewHeightTests／MusicModuleTests | 9／8 | 前者拆在 `shell_logic/geometry.rs` 與 `music/module.rs`（T11、T19，`rust_location` 以逗號列出多處，腳本加總），後者 `music/module.rs`（T11） |
  | HelperMessageParserTests | 15 | `platform-macos/src/helper_wire.rs`（T14） |
  | MediaRemoteSourceTests | 26 | `platform-macos/src/supervisor.rs`（T15） |
  | MediaRemoteControlTests | 4 | `platform-macos/src/mediaremote_control.rs`（T17） |
  | WindowTitleSourceTests | 11 | `platform-macos/src/window_title_source.rs`（T18） |
  | MarqueeTextTests／LyricStripPreferenceTests | 4／2 | `shell/src/marquee.rs`、`app/src/config.rs`（T24、T25） |

- [ ] **Step 4: 寫 CI。** `rust.yml` 在 `push` 與 `pull_request` 觸發，矩陣 `macos-latest`、`windows-latest`。兩者都跑 `cargo fmt --check`、`rust/scripts/check-core-purity.sh`（Windows 用 `shell: bash`）、`cargo test -p musicat-core --features testing`、`cargo build -p musicat-platform-windows`。僅 macOS 另跑 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace`、`cargo test --workspace`。有螢幕才能跑的測試以 `#[ignore]` 標記，不進 CI。
- [ ] **Step 5: 文件骨架。** `README.md` 含：專案簡介、狀態（macOS 開發中、Windows 未驗證）、從原始碼建置（Metal Toolchain 說明留待 Task 21 補）、**隱私**（曲名與演出者會送到 lrclib.net 與 music.163.com，來源可個別關閉）、**macOS 風險**（perl 宿主是借 Apple 簽章宿主的空隙，不是正式介面，系統更新可能失效，不適合上架，可在選單與 TOML 關閉）、授權、歌詞資料不在授權範圍。`CONTRIBUTING.md` 寫 core 純淨規則、parity 規則、測試怎麼跑。
- [ ] **Step 6: 驗證並提交。** Run: `cd rust && cargo fmt --check && cargo test --workspace && sh scripts/check-core-purity.sh`；Expected: 全部通過（`parity.sh` 先不跑）。Commit: `chore(rust): scaffold MusiCat workspace, licenses and CI`。

## Task 2：非同步模型 ADR 與基礎元件

這是 `core` 最貴的決定（spec 第 13 節），要在寫任何移植測試之前定案。

**Files:**
- Create: `rust/docs/adr-0001-async-model.md`、`rust/core/src/runtime.rs`、`timing.rs`、`http.rs`、`testing.rs`
- Test: `rust/core/src/timing.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces（其他任務依賴的確切簽章）：
  ```rust
  pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
  pub trait Clock: Send + Sync {
      fn now(&self) -> f64;                                  // Unix epoch 秒
      fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
  }
  pub struct TaskHandle;                                      // drop 即取消
  pub trait Spawner: Send + Sync { fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle; }
  pub struct Notifier<T>;  pub struct Subscription;           // drop 即取消訂閱
  impl<T: Send + 'static> Notifier<T> {
      pub fn new() -> Self;
      pub fn subscribe(&self, f: Box<dyn Fn(&T) + Send + Sync>) -> Subscription;
      pub fn notify(&self, v: &T);
  }
  pub struct TimedOut;
  pub async fn with_timeout<T>(clock: &dyn Clock, limit: Duration, fut: impl Future<Output = T>) -> Result<T, TimedOut>;
  pub struct Coalescer<K, V: Clone>;
  impl<K: Hash + Eq + Clone, V: Clone + Send + 'static> Coalescer<K, V> {
      pub fn new() -> Self;
      pub fn run(&self, key: K, make: impl FnOnce() -> BoxFuture<'static, V>) -> impl Future<Output = V> + Send;
      pub fn waiters(&self, key: &K) -> usize;               // 測試用
  }
  pub struct HttpRequest { pub url: String, pub headers: Vec<(String, String)>, pub timeout: Duration }
  pub struct HttpResponse { pub status: u16, pub body: Vec<u8> }
  pub enum HttpError { Transport(String), TooLarge }
  pub trait HttpClient: Send + Sync { fn get(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, HttpError>>; }
  // testing.rs（feature "testing"）
  pub struct FakeClock;   // now 可設定；advance(Duration) 喚醒到期的 sleep
  pub struct FakeHttp;    // 依 URL 排程回應／錯誤／永不回應；記錄請求次數
  pub struct LocalSpawner; // 以 futures::executor::LocalPool 驅動，run_until_stalled()
  ```
- 設計定案（寫進 ADR，每項附一句理由）：`Future` 加注入 `Clock`；逾時用 `futures_util::future::select`（輸家被 drop 即取消）；併發合併用 `futures_util::future::Shared` 存成 `WeakShared`，最後一個等待者被 drop 時內部 future 一併被 drop（取消）；取消的查詢因為快取寫入在 future 內部且 future 沒跑完，自然不會被快取成「查無」；`Spawner` 回傳的 `TaskHandle` drop 即取消，對應 GPUI 的 `Task`。若 Step 1 的三個證明測試寫不出來，停下來回報並提出狀態機替案，不要硬做。

- [ ] **Step 1: 寫三個證明測試（失敗）。** 在 `timing.rs` 測試模組以 `FakeClock` 加 `LocalSpawner` 寫：
  - `with_timeout_does_not_wait_for_a_future_that_ignores_cancellation`：被包的 future 永不完成，`advance(5s)` 後 `with_timeout` 回 `Err(TimedOut)`，且不需要被包的 future 配合。
  - `coalescer_shares_one_run_between_concurrent_callers`：同一 key 兩個呼叫者，`make` 只被呼叫一次，兩者拿到相同值，`waiters(&key) == 2`；完成後同 key 再呼叫會重新跑 `make`。
  - `coalescer_cancels_the_run_when_every_waiter_is_dropped_and_never_reports_a_result`：兩個呼叫者都 drop 後，內部 future 被 drop（以 drop guard 旗標驗證），下一次呼叫重新執行。只剩一個呼叫者被 drop 時內部 future 繼續跑。
- [ ] **Step 2: Run:** `cargo test -p musicat-core --features testing timing`；Expected: FAIL（符號未定義）。
- [ ] **Step 3: 實作上述介面。** `Notifier` 用 `Mutex<Vec<(id, Box<dyn Fn>)>>`，`Subscription` 的 drop 移除該 id；回呼不得在持鎖狀態下執行（先複製出來再呼叫，避免重入死鎖）。
- [ ] **Step 4: 寫 ADR。** `adr-0001-async-model.md`：背景、定案、被否決的替案（以事件加計時驅動的狀態機：每個移植測試都要手動推進狀態，改寫成本高）、三個證明測試的名稱。
- [ ] **Step 5: Run:** `cargo test -p musicat-core --features testing && sh rust/scripts/check-core-purity.sh`；Expected: PASS。Commit: `feat(core): async model ADR, Clock/Spawner/HttpClient, timeout and coalescer`。

## Task 3：型別、播放位置、播放估算、封面快取

**Files:**
- Create: `rust/core/src/types.rs`、`estimator.rs`、`artwork_cache.rs`
- Test: 各檔 `#[cfg(test)]`；移植 `PositionAnchorTests`（7）、`PlaybackEstimatorTests`（7）、`ArtworkCacheTests`（6）

**Interfaces:**
- Consumes: 無。
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Debug)] pub struct ArtworkRef { pub id: String, pub data: Option<Arc<[u8]>> }
  #[derive(Clone, Copy, PartialEq, Debug)] pub struct PositionAnchor { pub elapsed: f64, pub timestamp: f64, pub rate: f64 }
  impl PositionAnchor { pub fn position_at(&self, now: f64, cap: Option<f64>) -> f64 }
  #[derive(Clone, PartialEq, Debug)] pub struct TrackSnapshot {
      pub title: String, pub artist: Option<String>, pub album: Option<String>,
      pub duration: Option<f64>, pub is_playing: bool, pub position: Option<PositionAnchor>,
      pub artwork: Option<ArtworkRef>, pub source_id: String }
  pub struct PlaybackEstimator;   // new(); observe(&mut self, title: Option<&str>, is_playing: bool, now: f64); anchor(&self) -> Option<PositionAnchor>
  pub struct ArtworkCache;        // new(capacity: usize)（下限 1）; resolve(&mut self, Option<ArtworkRef>) -> Option<ArtworkRef>
  ```

- [ ] **Step 1: 移植測試（失敗）。** 例：`position_never_rewinds_when_now_is_before_timestamp`（`elapsed 30, timestamp 100, rate 1`，`position_at(90, None) == 30.0`）、`position_is_capped_to_duration`（`elapsed 170, rate 1, now = timestamp+60, cap Some(180)` 得 `180.0`）、`paused_rate_zero_does_not_advance`；估算器：已在播放中開始觀察的曲目 `anchor() == None`（絕不是 0），觀察中換曲才得到 `elapsed 0` 的 anchor，暫停凍結、恢復續算；封面快取：容量 16 預設、最舊先被丟、帶位元組時存入並回傳原值、不帶位元組時補上快取的位元組。其餘案例逐一照 Swift 測試。
- [ ] **Step 2: Run** `cargo test -p musicat-core types estimator artwork_cache`；Expected: FAIL。
- [ ] **Step 3: 實作。** `position_at` 照 Global Constraints 的公式，非有限的輸入不得 panic（NaN 輸入回傳 `elapsed` 夾到 0）。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): snapshot types, position anchor, estimator, artwork cache`。

## Task 4：歌詞模型與 LRC 解析

**Files:**
- Create: `rust/core/src/lyrics/mod.rs`
- Test: 同檔；移植 `LyricsTests`（41）

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Debug)] pub struct LyricLine { pub time: f64, pub text: String }
  #[derive(Clone, PartialEq, Debug)] pub enum Lyrics { Synced(Vec<LyricLine>), Plain(String) }
  impl Lyrics {
      pub const DURATION_TOLERANCE: f64 = 5.0;
      pub fn is_synced(&self) -> bool;
      pub fn is_plausible(&self, duration: Option<f64>) -> bool;
      pub fn line_index(&self, position: f64) -> Option<usize>;   // 同時間戳取最後一行；NaN → None
  }
  pub fn parse_lrc(raw: &str) -> Option<Lyrics>;
  ```

- [ ] **Step 1: 移植測試（失敗）。** 照 `LyricsTests` 41 案，另加 Review Focus 第 2 條的測試：`parse_lrc_handles_a_one_megabyte_single_line_in_linear_time`（斷言在合理時間內完成，以輸入加倍時耗時不超過約 3 倍為限，不寫絕對秒數）、`parse_lrc_treats_huge_minute_numbers_as_not_a_time_tag`（`[99999999999:00]` 不 panic 也不溢位）、`parse_lrc_without_any_time_tag_is_plain`、`metadata_only_tags_are_plain`、`timestamps_on_empty_lines_only_returns_none`。
- [ ] **Step 2: Run** `cargo test -p musicat-core lyrics::tests`；Expected: FAIL。
- [ ] **Step 3: 實作。** 以位元組掃描而非 regex（原因同 Swift 版註解：惡意輸入必須線性）；時間標籤規則：分 1–3 位、秒 1–2 位、小數 1–3 位，`[mm:ss]`、`[mm:ss.x]`、`[mm:ss:x]`，以及增強型的 `<…>` 字詞標籤要移除並收合空白；排序以 `(time, 原始順序)` 穩定排序；BOM 去除。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): lyrics model and forgiving LRC parser`。

## Task 5：曲目識別、標題清理、寬鬆比對

**Files:**
- Create: `rust/core/src/lyrics/identity.rs`、`normalizer.rs`
- Modify: `rust/core/Cargo.toml`（加 `zhconv`、`unicode-normalization`、`regex`）
- Test: 同檔；移植 `TrackIdentityTests`（10）、`TrackNormalizerTests`（15）

**Interfaces:**
- Consumes: `TrackSnapshot`（Task 3）。
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Eq, Hash, Debug)]
  pub struct TrackIdentity { pub title: String, pub artist: String, pub album: Option<String>, pub duration: Option<f64> }  // f64 以 to_bits 參與 Hash／Eq
  impl TrackIdentity {
      pub fn normalized(&self) -> TrackIdentity;
      pub fn cache_key(&self) -> String;                    // 正規化後 title、artist 小寫，以 \u{1} 連接，不含長度
      pub fn duration_matches(&self, other: Option<f64>) -> bool;
      pub fn loosely_matches(a: &str, b: &str) -> bool;
  }
  pub fn identity_from(snapshot: &TrackSnapshot) -> Option<TrackIdentity>;   // 沒有演出者 → None
  ```

- [ ] **Step 1: 先驗證簡繁折疊套件。** 在 `identity.rs` 寫測試 `loosely_matches_ignores_script_case_and_width`：`("愛你", "爱你")`、`("ＡＢＣ", "abc")`、`("Beyoncé", "beyonce")` 皆為 `true`，`("愛你", "愛她")` 為 `false`。以 `zhconv` 把兩側轉成同一種文字後再折疊（NFKC 處理全半形、小寫、去重音符號、trim）。若 `zhconv` 在 `core` 內不能純 Rust 編譯（含 Windows），改選 `ferrous-opencc` 或 `fast2s`，並在 ADR 或 `identity.rs` 註解寫下理由。
- [ ] **Step 2: 移植其餘測試（失敗）。** 照 Swift 兩個測試檔，包含：`feat.`／`ft.`／`Remastered`／`Live`／`Explicit` 括號後綴（含全形括號）被移除但 `(Liverpool Mix)` 保留；`A - B ft. C` 當 `A` 包含於演出者時拆成 `B`；`nil`／全空白演出者回傳 `None`；曲長 `0` 或負值視為未知；多演出者以 `/ & , ;` 分隔取第一位。Review Focus 補：`identity_from_does_not_panic_on_emoji_and_combining_characters`。
- [ ] **Step 3: Run** `cargo test -p musicat-core identity normalizer`；Expected: FAIL。
- [ ] **Step 4: 實作。** 正規表達式照 Swift 版 `TrackIdentity.suffixPattern`、`TrackNormalizer` 的各個 pattern 搬（Rust `regex` 不支援 `\b` 與 Unicode 的差異要逐一用測試確認，必要時以 `(?i)` 與 `\b` 的 Unicode 模式調整）。
- [ ] **Step 5: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): track identity, title normalizer and loose matching`。

## Task 6：歌詞提供者介面與 LRCLIB、網易雲

**Files:**
- Create: `rust/core/src/lyrics/provider.rs`、`lrclib.rs`、`netease.rs`
- Test: 同檔；移植 `ProviderTests`（48，含 URL 建構、候選挑選、錯誤分類）

**Interfaces:**
- Consumes: `HttpClient`／`HttpRequest`（Task 2）、`TrackIdentity`（Task 5）、`Lyrics`／`parse_lrc`（Task 4）。
- Produces:
  ```rust
  pub enum ProviderError { BadStatus(u16), BadResponse, Transport(String) }
  pub trait LyricsProvider: Send + Sync {
      fn id(&self) -> &str;
      fn fetch<'a>(&'a self, track: &'a TrackIdentity) -> BoxFuture<'a, Result<Option<Lyrics>, ProviderError>>;
  }
  pub struct LrclibProvider;   // new(http: Arc<dyn HttpClient>)；id "net.lrclib"
  pub struct NetEaseProvider;  // new(http: Arc<dyn HttpClient>)；id 照 Swift 版
  pub const USER_AGENT: &str = "MusiCat";
  pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
  pub fn build_url(base: &str, query: &[(&str, &str)]) -> String;   // 嚴格百分比編碼，只留 A-Za-z0-9-._~
  ```

- [ ] **Step 1: 移植測試（失敗）。** 以 `FakeHttp` 排程回應，逐案照抄 `ProviderTests`（精確端點 404 乾淨地換成搜尋、非 2xx 且非 404 拋 `BadStatus`、2xx 但 JSON 壞掉拋 `BadResponse`、候選依「標題寬鬆相符＋曲長相符」挑、帶時間軸優先於純文字、`+` 與 `&` 被正確編碼、不帶任何使用者資料的 User-Agent）。Review Focus 補：`oversized_body_is_a_failure_not_a_hit`（`HttpError::TooLarge` 或本體超過 `MAX_BODY_BYTES` 都回 `Err`）。
- [ ] **Step 2: Run** `cargo test -p musicat-core --features testing provider lrclib netease`；Expected: FAIL。
- [ ] **Step 3: 實作兩個提供者。** 流程與錯誤分類照 `LRCLIBProvider.swift`、`NetEaseProvider.swift`（讀這兩個檔），差別只在 HTTP 走 `HttpClient`；每個請求的 `timeout` 設 5 秒。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): LyricsProvider trait with LRCLIB and NetEase providers`。

## Task 7：歌詞走訪、快取、可信度把關

**Files:**
- Create: `rust/core/src/lyrics/repository.rs`
- Test: 同檔；移植 `LyricsRepositoryTests`（30）

**Interfaces:**
- Consumes: `LyricsProvider`、`Clock`、`with_timeout`、`Coalescer`、`Lyrics::is_plausible`、`TrackIdentity::cache_key`。
- Produces:
  ```rust
  pub struct RepositoryConfig { pub negative_ttl: Duration /*86400s*/, pub failure_ttl: Duration /*120s*/, pub provider_timeout: Duration /*5s*/ }
  pub trait LyricsResolving: Send + Sync { fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<Lyrics>>; }
  pub struct LyricsRepository;   // new(providers: Vec<Arc<dyn LyricsProvider>>, clock: Arc<dyn Clock>, cfg: RepositoryConfig)；實作 LyricsResolving
  ```

- [ ] **Step 1: 移植測試（失敗）。** 30 案逐一照抄，並確保以下行為各有一個明確命名的測試：`synced_result_outranks_provider_priority`（高順位只有純文字、低順位有同步 → 用同步）、`plain_is_only_a_fallback`、`implausible_synced_counts_as_empty_not_failure`、`success_is_cached_forever`、`all_clean_misses_are_cached_for_24_hours`、`any_failure_caches_only_120_seconds`、`plain_fallback_found_while_a_source_failed_expires_after_120_seconds`、`provider_timeout_is_a_miss_and_does_not_stall_the_walk`（第一個來源永不回應，`advance(5s)` 後仍走到第二個來源）、`concurrent_callers_share_one_lookup`、`cancelled_lookup_is_not_cached_as_missing`（Review Focus 第 1 條）、`one_cancelled_waiter_does_not_abandon_the_lookup_for_the_others`、`inserting_a_provider_at_the_front_needs_no_other_change`（spec 第 8 節的擴充點）、`cache_key_ignores_duration`。
- [ ] **Step 2: Run** `cargo test -p musicat-core --features testing repository`；Expected: FAIL。
- [ ] **Step 3: 實作。** 單趟走訪：依序問每個來源，逾時以 `with_timeout` 算失敗；帶時間軸且通過 `is_plausible` 就停；純文字保留第一個當保底；快取寫入發生在 `Coalescer` 內部的 future 結尾（見 ADR），所以被取消者不會寫入；快取用 `Mutex<HashMap<String, CacheEntry>>`，`CacheEntry` 為 `Found { lyrics, until: Option<f64> }` 或 `Missing { until: f64 }`，過期以 `Clock::now` 判斷。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): lyrics repository with ordered walk, caching and plausibility gate`。

## Task 8：現正播放介面、監視器、視窗標題解析

**Files:**
- Create: `rust/core/src/nowplaying/mod.rs`、`monitor.rs`、`window_title.rs`
- Test: 同檔；移植 `NowPlayingMonitorTests`（12）、`WindowTitleParserTests`（7）

**Interfaces:**
- Consumes: `TrackSnapshot`、`ArtworkCache`、`Notifier`／`Subscription`。
- Produces:
  ```rust
  pub trait NowPlayingSource: Send + Sync {
      fn id(&self) -> &str;
      fn start(&self);
      fn stop(&self);                     // 停止並發布 None，之後 start 不重播上次快照
      fn subscribe(&self, f: Box<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>) -> Subscription;  // 訂閱當下立即以目前值呼叫一次（初始 None）
  }
  #[derive(Clone, Copy, PartialEq, Debug)] pub enum PlaybackCommand { Play, Pause, TogglePlayPause, Next, Previous }
  pub trait PlaybackControl: Send + Sync { fn send(&self, cmd: PlaybackCommand); }   // 不可用時整個物件為 None（Option<Arc<dyn PlaybackControl>>）
  pub struct NowPlayingMonitor;   // new(sources: Vec<Arc<dyn NowPlayingSource>>, artwork: ArtworkCache); current(&self) -> Option<TrackSnapshot>; subscribe(...) -> Subscription
  pub struct WindowInfo { pub owner: String, pub title: String }
  pub struct ParsedTrack { pub title: String, pub is_playing: bool }
  pub fn parse_window_title(w: &WindowInfo) -> Option<ParsedTrack>;   // 只認 "YouTube Music - <title> | YouTube Music"，尾端 🔊 表示播放中
  ```

- [ ] **Step 1: 移植測試（失敗）。** 監視器：優先序清單第一個非 `None` 的來源勝出（即使暫停）、尚未發布的來源算 `None`、勝出者的封面由 `ArtworkCache` 補齊、只有結果真的變了才通知。標題解析：`🔊` 前後空白、前後綴缺失回 `None`、標題為空回 `None`。
- [ ] **Step 2: Run** `cargo test -p musicat-core --features testing nowplaying`；Expected: FAIL。
- [ ] **Step 3: 實作。** 監視器用 `Mutex<Vec<Option<TrackSnapshot>>>` 保存每個來源的最新值，每次來源通知就重算；以 `TrackSnapshot: PartialEq` 比較是否改變。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): now-playing source traits, monitor and window-title parser`。

## Task 9：呈現邏輯

**Files:**
- Create: `rust/core/src/music/presentation.rs`
- Test: 同檔；移植 `MusicPresentationTests`（12）

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Debug)] pub enum StripContent { Line(String), Title(String), None }
  pub fn playback_position(s: &TrackSnapshot, now: f64) -> Option<f64>;       // 無 anchor → None，絕不補 0
  pub fn strip_content(s: Option<&TrackSnapshot>, lyrics: Option<&Lyrics>, now: f64) -> StripContent;
  pub fn has_progress(s: &TrackSnapshot) -> bool;                              // duration > 0 且有 anchor
  pub fn progress_fraction(s: &TrackSnapshot, now: f64) -> Option<f64>;        // 夾在 0..=1
  ```

- [ ] **Step 1: 移植測試（失敗）。** 12 案；另確保：純文字歌詞永遠回 `Title`、位置在第一行之前回 `Title`、位置未知回 `Title`、`duration == Some(0.0)` 時 `has_progress == false`。
- [ ] **Step 2–4:** Run → FAIL；照 Swift `MusicPresentation.swift` 實作；Run → PASS。更新 `parity.tsv`。Commit: `feat(core): music presentation rules`。

## Task 10：Music 控制器（歌詞查詢的時機與競態）

**Files:**
- Create: `rust/core/src/music/controller.rs`
- Test: 同檔；移植 `MusicViewModelTests`（10）

**Interfaces:**
- Consumes: `NowPlayingMonitor`、`LyricsResolving`、`PlaybackControl`、`Spawner`、`identity_from`、`Notifier`。
- Produces:
  ```rust
  #[derive(Clone, PartialEq, Debug, Default)] pub struct MusicState { pub snapshot: Option<TrackSnapshot>, pub lyrics: Option<Lyrics> }
  pub struct MusicController;
  impl MusicController {
      pub fn new(monitor: Arc<NowPlayingMonitor>, resolver: Arc<dyn LyricsResolving>,
                 control: Option<Arc<dyn PlaybackControl>>, spawner: Arc<dyn Spawner>) -> Arc<Self>;
      pub fn state(&self) -> MusicState;
      pub fn subscribe(&self, f: Box<dyn Fn(&MusicState) + Send + Sync>) -> Subscription;
      pub fn has_control(&self) -> bool;
      pub fn send(&self, cmd: PlaybackCommand);   // 無 control 時為 no-op
  }
  ```

- [ ] **Step 1: 移植測試（失敗）。** 10 案；Review Focus 第 1 條：`slow_lookup_for_the_previous_track_never_overwrites_the_new_tracks_lyrics`（A 的查詢被卡住，切到 B 並完成，再放行 A，`state().lyrics` 仍是 B 的）、`changing_only_play_state_or_position_does_not_start_a_new_lookup`（以 `cache_key` 判斷）、`track_without_artist_clears_lyrics_and_starts_no_lookup`、`dropping_the_controller_cancels_the_inflight_lookup`。
- [ ] **Step 2: Run** `cargo test -p musicat-core --features testing controller`；Expected: FAIL。
- [ ] **Step 3: 實作。** 曲目換了（`cache_key` 不同）就丟掉舊的 `TaskHandle`（取消）、清空歌詞、用 `Spawner` 起新查詢；完成時比對 `current_key` 才寫入。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): MusicController with stale-lookup protection`。

## Task 11：模組狀態與管理器

**Files:**
- Create: `rust/core/src/music/module.rs`
- Test: 同檔；移植 `ModuleManagerPreviewHeightTests` 中屬於「模組管理」的案例與 `MusicModuleTests`（8），屬於幾何的案例留給 Task 19

**Interfaces:**
- Consumes: `MusicController`、`Notifier`。
- Produces:
  ```rust
  pub const MAX_PREVIEW_EXTRA_HEIGHT: f64 = 28.0;
  pub trait ModuleState: Send + Sync {
      fn id(&self) -> &str;
      fn priority(&self) -> i32;
      fn is_active(&self) -> bool;
      fn preview_extra_height(&self) -> f64;
      fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) -> Subscription;   // active 或額外高度變動時通知
  }
  pub struct ModuleManager;  // register(Arc<dyn ModuleState>); active_module_id() -> Option<String>; preview_extra_height() -> f64（夾到 0..=28）; subscribe(...)
  pub struct MusicModuleState; // new(controller: Arc<MusicController>, strip_visible: Arc<AtomicBool>)；id "com.musicat.music"
  ```
- `ModuleState` 只含狀態；圖示、Preview／Expanded 的 GPUI 內容在 `shell`（Task 24），避免 `core` 依賴 GPUI。

- [ ] **Step 1: 移植測試（失敗）。** 多個 active 時 priority 高者勝；額外高度只取 active 模組的、夾到 `0..=28`（`-5 → 0`、`99 → 28`、NaN → 0）；Music 的 `is_active` 等於「目前快照 `is_playing`」（沒曲目或暫停都不是，同 Swift `monitor.isPlayingPublisher`）；額外高度為 24 當且僅當 `is_playing && strip_visible`，否則 0（沒有同步歌詞時歌詞條改顯示曲名，高度不變）。
- [ ] **Step 2–4:** Run → FAIL；實作；Run → PASS。更新 `parity.tsv`。Commit: `feat(core): module state and manager`。

## Task 12：R1 收尾（驗證、開源文件、移植對照）

**Files:**
- Modify: `rust/parity.tsv`、`README.md`、`CONTRIBUTING.md`

- [ ] **Step 1: Run** `sh rust/scripts/parity.sh r1`；Expected: 對 `phase` 為 `r1` 的每一列通過（`r2` 的列留到 Task 26 才檢查）。若有案例因語言差異被合併，在 `waiver_reason` 寫明合併到哪個測試。
- [ ] **Step 2: Run** `cd rust && cargo test -p musicat-core --features testing && cargo clippy -p musicat-core --all-targets -- -D warnings && sh scripts/check-core-purity.sh`；Expected: 全部通過。
- [ ] **Step 3: Windows 編譯確認。** 在 CI 上確認 Windows job 通過（推到作者指定的遠端後才看得到；若尚未 push，在本地 `cargo check -p musicat-core --target x86_64-pc-windows-msvc` 驗證，需要先 `rustup target add`）。
- [ ] **Step 4: Commit:** `docs(rust): R1 parity audit and docs`。**R1 完成。**

---

# R2：macOS 平台層與瀏海外殼

## Task 13：`platform-macos` crate 與 C 動態庫建置

**Files:**
- Create: `rust/platform-macos/build.rs`、`helper/helper.c`、`helper/helper.h`、`src/lib.rs`
- Modify: `rust/platform-macos/Cargo.toml`（`[target.'cfg(target_os = "macos")'.dependencies]` 放平台依賴，其餘平台編成空 crate）

**Interfaces:**
- Produces: 建置後 `target/<profile>/libmusicat_mediaremote_helper.dylib`（與可執行檔同目錄，供 perl 以絕對路徑載入）；入口符號 `musicat_helper_main`；`pub const DYLIB_NAME: &str = "libmusicat_mediaremote_helper.dylib"`。

- [ ] **Step 1: 複製並改名。** 把 `Sources/NotchCatMediaRemoteHelper/helper.c` 與 `include/NotchCatMediaRemoteHelper.h` 複製進 `helper/`（Swift 版原檔不動），僅把入口符號 `notchcat_helper_main` 改為 `musicat_helper_main`，檔頭註解把 NotchCat 改為 MusiCat；**不改線上格式與逾時、重啟邏輯**。
- [ ] **Step 2: 寫失敗測試。** `tests/dylib.rs`（僅 macOS）：建置後 dylib 存在於預期路徑；`nm -gU` 輸出含 `_musicat_helper_main`；`codesign -dv` 顯示 `adhoc`。
- [ ] **Step 3: 寫 `build.rs`。** 用 `cc` crate 取得編譯器，直接以 `-dynamiclib -framework CoreFoundation` 產生 dylib 到 `OUT_DIR`，再複製到 `target/<profile>/`（由 `OUT_DIR` 往上推導）；`cargo:rerun-if-changed=helper/`。arm64 連結器輸出本身是 adhoc 簽章，不需額外 `codesign`（Swift 版同樣）。
- [ ] **Step 4: Run** `cargo test -p musicat-platform-macos --test dylib`；Expected: PASS。Commit: `feat(platform-macos): build the MediaRemote helper dylib via build.rs`。

## Task 14：helper 線上格式解析與行緩衝

**Files:**
- Create: `rust/platform-macos/src/helper_wire.rs`
- Test: 同檔；移植 `HelperMessageParserTests`（15）

**Interfaces:**
- Consumes: `TrackSnapshot`、`PositionAnchor`、`ArtworkRef`。
- Produces:
  ```rust
  pub const SOURCE_ID: &str = "mediaremote";
  #[derive(Clone, PartialEq, Debug)] pub enum HelperMessage { None, Track(TrackSnapshot) }
  pub fn parse_line(line: &str) -> Option<HelperMessage>;   // 非合法訊息回 None（不是 HelperMessage::None）
  pub struct LineBuffer;   // new() 預設單行上限 4 MiB；with_max(n); push(&mut self, bytes: &[u8]) -> Vec<String>
  ```
- 線上格式（不變）：`{"none":true}` 或 `{"title","artist"?,"album"?,"duration"?,"elapsed","timestamp","rate","artworkId"?,"artworkData"?}`；`timestamp` 為 Unix 秒；缺值省略；`duration <= 0` 視為未知；`rate > 0` 即播放中；`elapsed`／`timestamp`／`rate`／`duration` 有非有限數值整行拒絕；沒有標題整行拒絕。

- [ ] **Step 1: 移植測試（失敗）。** 15 案，加 Review Focus 第 3 條：`line_split_across_pushes_is_reassembled`（含被切在多位元組字元中間）、`invalid_utf8_line_is_dropped_without_affecting_neighbours`、`line_over_the_cap_is_discarded_through_its_newline`（含恰好等於上限、上限加一）、`empty_lines_are_skipped`、`garbage_json_returns_none`。
- [ ] **Step 2: Run** `cargo test -p musicat-platform-macos helper_wire`；Expected: FAIL。
- [ ] **Step 3: 實作。** 行緩衝以位元組層級切 `\n`，把 Swift 版 `HelperLineBuffer` 與 `CappedHelperLineBuffer` 合為一個型別（同樣的丟棄語意）；解析用 `serde_json::Value` 逐欄位取，以免 `serde` 的 derive 對缺值過於寬鬆。
- [ ] **Step 4: Run** 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(platform-macos): helper wire parser and capped line buffer`。

## Task 15：行程監管（`HelperSupervisor`）

**Files:**
- Create: `rust/platform-macos/src/supervisor.rs`
- Test: 同檔；移植 `MediaRemoteSourceTests`（26）

**Interfaces:**
- Consumes: `NowPlayingSource`、`Clock`、`Notifier`、`helper_wire::parse_line`。
- Produces:
  ```rust
  pub trait HelperHandle: Send { fn terminate(&mut self); }
  pub trait HelperLauncher: Send + Sync {
      fn launch(&self, sink: Arc<dyn Fn(HelperEvent) + Send + Sync>) -> Result<Box<dyn HelperHandle>, LaunchError>;
  }
  pub enum HelperEvent { Line(String), Exited(i32) }
  pub struct SupervisorConfig { pub retry_delays: Vec<Duration> /*[1,2,4,8,16,30]s*/, pub give_up_retry_interval: Duration /*300s*/ }
  pub struct HelperSupervisor;   // new(launcher, clock, spawner, cfg, wake: ...) 實作 NowPlayingSource；wake_from_sleep(&self) 供平台層在喚醒時呼叫
  ```
- 行為（逐項照 `MediaRemoteSource.swift` 的文件註解）：意外結束後依 `retry_delays` 重啟，收到任何一行就重置序列；延遲用完後發布 `None` 並停止，`give_up_retry_interval` 後重置計數再試，`start()`、喚醒也立刻重試；`launch` 回 `Err` 也算失敗；喚醒時替換正在跑的 helper；`stop()` 之後重試與喚醒都不得動作；以 generation 計數讓舊行程的殘留輸出與計時器失效，且舊行程的輸出不得重置失敗計數。

- [ ] **Step 1: 移植測試（失敗）。** 以假 `HelperLauncher` 與 `FakeClock` 驅動，26 案逐一照抄；另確保 Review Focus 第 3、5 條相關：`launch_failure_counts_toward_giving_up`（perl 或動態庫不存在時不崩潰、最終發布 `None`）、`stale_output_from_a_dead_process_does_not_reset_the_failure_count`、`stop_then_start_does_not_replay_the_last_snapshot`。
- [ ] **Step 2–4:** Run `cargo test -p musicat-platform-macos supervisor` → FAIL；實作（所有狀態放在單一 `Mutex<State>`，事件入口只有 `Line`／`Exited`／`Timer`／`Wake`／`Start`／`Stop`，每個入口持鎖處理完再放鎖後通知）；Run → PASS。更新 `parity.tsv`。Commit: `feat(platform-macos): HelperSupervisor with backoff, give-up retry and wake restart`。

## Task 16：perl 啟動器

**Files:**
- Create: `rust/platform-macos/src/perl_launcher.rs`
- Test: 同檔

**Interfaces:**
- Consumes: `HelperLauncher`／`HelperHandle`／`HelperEvent`（Task 15）、`LineBuffer`（Task 14）、`DYLIB_NAME`。
- Produces:
  ```rust
  pub struct PerlLauncher;   // new(perl_path: PathBuf, dylib: Option<PathBuf>)；default() 用 /usr/bin/perl 與可執行檔旁的 DYLIB_NAME
  pub fn sanitized_env(env: impl Iterator<Item = (String, String)>) -> Vec<(String, String)>;   // 移除前綴 PERL、DYLD_、LD_
  pub enum LaunchError { PerlNotFound, DylibNotFound, Spawn(String) }
  pub const SCRIPT: &str;   // 同 Swift 版 PerlHelperLauncher.script，符號改 musicat_helper_main
  ```
- 行為：以 `/usr/bin/perl -e SCRIPT <dylib 絕對路徑>` 啟動，環境變數經 `sanitized_env`；stdin 保持為開著的 pipe（handle 持有，drop 即關閉讓 helper 在 EOF 結束）；stdout 由專屬執行緒讀取並餵給 `LineBuffer`；行程結束時送出 `HelperEvent::Exited`；`terminate()` 關 stdin 並 kill；找不到動態庫時整個行程只在日誌印一次訊息。

- [ ] **Step 1: 寫失敗測試。** `sanitized_env_removes_perl_dyld_and_ld_prefixed_variables`（`PERL5LIB`、`PERLLIB`、`DYLD_INSERT_LIBRARIES`、`LD_PRELOAD` 被移除，`PATH`、`HOME` 保留）；`missing_dylib_returns_dylib_not_found`；`missing_perl_returns_perl_not_found`；`missing_dylib_message_is_logged_only_once`。
- [ ] **Step 2: 加一個 `#[ignore]` 的整合測試** `real_helper_emits_at_least_one_line_within_five_seconds`（需要 Task 13 的 dylib；環境變數 `MUSICAT_REAL_HELPER=1` 才執行；不進 CI）。
- [ ] **Step 3: Run** `cargo test -p musicat-platform-macos perl_launcher`；Expected: FAIL。實作後 Run 同上；Expected: PASS。再手動 Run：`MUSICAT_REAL_HELPER=1 cargo test -p musicat-platform-macos -- --ignored real_helper`，Expected: PASS，並於 PR 說明記錄輸出。
- [ ] **Step 4: Commit:** `feat(platform-macos): perl helper launcher with sanitized environment`。

## Task 17：播放控制

**Files:**
- Create: `rust/platform-macos/src/mediaremote_control.rs`
- Test: 同檔；移植 `MediaRemoteControlTests`（4）

**Interfaces:**
- Consumes: `PlaybackControl`／`PlaybackCommand`。
- Produces:
  ```rust
  pub fn command_code(cmd: PlaybackCommand) -> u32;   // Play 0、Pause 1、Toggle 2、Next 4、Previous 5
  pub struct MediaRemoteControl;   // load() -> Option<MediaRemoteControl>（以 dlopen 載入 MediaRemote.framework 並 dlsym MRMediaRemoteSendCommand；任一步失敗回 None）；實作 PlaybackControl
  ```

- [ ] **Step 1: 移植測試（失敗）。** 指令碼對照表、`load` 在框架路徑不存在時回 `None` 且不 panic（以可注入的路徑參數測）。
- [ ] **Step 2–4:** Run → FAIL；實作（本檔是 Swift 主程序以外唯一可出現 `MRMediaRemote*` 與 `dlopen` 的地方，`rust/scripts/check-private-api.sh` 以 grep 確認它們沒出現在其他檔，加進 CI）；Run → PASS。更新 `parity.tsv`。Commit: `feat(platform-macos): MediaRemote playback control`。

## Task 18：視窗標題備援來源

**Files:**
- Create: `rust/platform-macos/src/window_title_source.rs`
- Test: 同檔；移植 `WindowTitleSourceTests`（11）

**Interfaces:**
- Consumes: `NowPlayingSource`、`parse_window_title`、`PlaybackEstimator`、`Clock`、`Spawner`。
- Produces:
  ```rust
  pub trait WindowLister: Send + Sync { fn windows(&self) -> Vec<WindowInfo>; }
  pub struct CgWindowLister;     // CGWindowListCopyWindowInfo（kCGWindowOwnerName、kCGWindowName）
  pub struct WindowTitleSource;  // new(lister, clock, spawner, poll_interval: Duration /*照 Swift 版預設*/)；id "windowtitle"；實作 NowPlayingSource
  ```
- 行為：輪詢；只回報標題（無演出者、無封面）；位置由 `PlaybackEstimator` 推估（已在播放的曲目位置未知）；沒有權限時 `CgWindowLister` 回空陣列，來源發布 `None`。

- [ ] **Step 1: 移植測試（失敗）→ Step 2: 實作 → Step 3: Run `cargo test -p musicat-platform-macos window_title_source` → PASS。** 更新 `parity.tsv`。Commit: `feat(platform-macos): window-title fallback source`。

## Task 19：外殼幾何、螢幕選擇、虛擬瀏海（放在 `core`）

**Files:**
- Create: `rust/core/src/shell_logic/geometry.rs`、`screens.rs`
- Test: 同檔；移植 `ModuleManagerPreviewHeightTests` 中屬於 `NotchMode` 幾何的案例

**Interfaces:**
- Consumes: `MAX_PREVIEW_EXTRA_HEIGHT`。
- Produces（座標一律以螢幕左上角為原點、y 向下；平台層負責轉換）：
  ```rust
  pub struct Size { pub w: f64, pub h: f64 }  pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
  pub enum ShellMode { Preview, Expanded }
  pub struct NotchGeometry { pub notch: Size, pub center_x: f64, pub top_y: f64 }
  pub const WIDTH_PADDING: f64 = 160.0;  pub const EXPANDED_HEIGHT: f64 = 180.0;
  pub const PREVIEW_CORNER_RADIUS: f64 = 10.0;  pub const EXPANDED_CORNER_RADIUS: f64 = 24.0;
  pub fn mode_size(mode: ShellMode, g: &NotchGeometry, preview_extra: f64) -> Size;
  pub fn container_size(g: &NotchGeometry) -> Size;                      // 取 Preview（extra=28）與 Expanded 的最大寬高
  pub fn container_frame(g: &NotchGeometry) -> Rect;                     // 以 center_x、top_y 置中貼頂
  pub fn local_rect(mode: ShellMode, g: &NotchGeometry, preview_extra: f64) -> Rect;   // 容器座標系，頂端置中
  pub struct ScreenInfo { pub id: String, pub name: String, pub frame: Rect, pub hardware_notch: Option<Size>, pub is_primary: bool, pub is_builtin: bool }
  pub struct VirtualNotch { pub w: f64, pub h: f64 }                     // 預設 200 × 32
  pub fn choose_screen<'a>(screens: &'a [ScreenInfo], preferred: Option<&str>) -> Option<&'a ScreenInfo>;
  pub fn geometry_for(screen: &ScreenInfo, virtual_notch: VirtualNotch) -> NotchGeometry;
  ```
- 螢幕選擇規則：`preferred` 存在於列表就用；否則有硬體瀏海的螢幕優先，再來內建螢幕，再來主螢幕，再來第一個；列表為空回 `None`（程式不結束，面板暫時隱藏）。沒有硬體瀏海時 `geometry_for` 用虛擬瀏海，且水平置中於該螢幕。

- [ ] **Step 1: 寫失敗測試。** 照 Swift 的 `NotchModeTests` 案例改寫：寬度 = 瀏海寬 + 160（真瀏海 185 與虛擬 200 各一）；Preview 高 = 瀏海高 + 額外高度（`-5 → 0`、`99 → 28`、NaN → 0）；Expanded 高 180、寬相同；容器 = 兩者最大；`local_rect` 的 x 為 `(容器寬 − 該模式寬) / 2`、y 為 0。螢幕選擇（Review Focus 第 4 條）：`preferred_screen_missing_falls_back_to_notched_then_builtin_then_primary`、`empty_screen_list_returns_none_instead_of_panicking`、`preferred_screen_returning_is_chosen_again`、`screen_without_hardware_notch_gets_the_virtual_one_centered`、`virtual_notch_size_is_configurable`。
- [ ] **Step 2: Run** `cargo test -p musicat-core shell_logic`；Expected: FAIL。實作；Run 同上；Expected: PASS。更新 `parity.tsv`。Commit: `feat(core): shell geometry, screen choice and virtual notch`。

## Task 20：Hover／模式狀態機（放在 `core`）

這是 Swift 版最難的部分（hover 閃爍），把它寫成純函式，才能用單元測試守住。

**Files:**
- Create: `rust/core/src/shell_logic/hover.rs`
- Test: 同檔

**Interfaces:**
- Consumes: `Rect`、`ShellMode`。
- Produces:
  ```rust
  pub const LEAVE_DELAY: f64 = 0.150;
  pub struct Point { pub x: f64, pub y: f64 }
  pub struct HoverInput { pub now: f64, pub mouse: Option<Point> /*容器座標；None = 不在視窗內*/,
                          pub preview_rect: Rect, pub container: Rect, pub fullscreen_foreground: bool }
  pub struct HoverOutput { pub mode: ShellMode, pub visible: bool,
                           pub just_expanded: bool /*這一次更新才剛展開：外殼要把切換列選取重設為 active 模組*/,
                           pub next_deadline: Option<f64> /*需要在此時間再呼叫一次 update*/ }
  pub struct HoverMachine;   // new(); update(&mut self, input: HoverInput) -> HoverOutput
  ```
- 規則（spec 第 5.3 節）：收合時只有游標在目前畫出的 `preview_rect` 內才展開；展開後有效範圍立刻是整個 `container`；游標離開有效範圍後 150 毫秒才收合，期間再進入就取消；`fullscreen_foreground` 為真時 `visible = false` 且 `mode = Preview`，離開全螢幕後再出現。

- [ ] **Step 1: 寫失敗測試。**
  - `cursor_inside_container_but_outside_preview_does_not_expand`
  - `cursor_inside_preview_expands_and_reports_just_expanded_once`
  - `after_expanding_the_whole_container_keeps_it_open`
  - `leaving_collapses_only_after_150ms`（`now = t` 離開得 `next_deadline == Some(t + 0.150)` 仍為 Expanded；`now = t + 0.149` 仍 Expanded；`t + 0.150` 變 Preview）
  - `re_entering_within_150ms_cancels_the_collapse`
  - `jitter_across_the_container_edge_does_not_flicker`（游標在容器邊緣內外各 3 次、每次間隔 < 150 毫秒，全程維持 Expanded，`just_expanded` 只出現一次）
  - `preview_height_change_under_a_stationary_cursor_expands_at_most_once`（HANDOFF 第 5 項：`preview_rect.h` 由 32 變 56，游標靜止在 y = 40，不得出現「展開→收合→展開」）
  - `fullscreen_foreground_hides_and_forces_preview`、`leaving_fullscreen_shows_again_in_preview`
  - `mouse_none_counts_as_outside`
- [ ] **Step 2: Run** `cargo test -p musicat-core hover`；Expected: FAIL。實作；Run；Expected: PASS。Commit: `feat(core): hover and mode state machine`。

## Task 21：`platform-macos` 的面板視窗模組（objc2 繞路）與 GPUI 視窗驗證

**Files:**
- Create: `rust/platform-macos/src/panel.rs`、`screens.rs`
- Modify: `rust/shell/Cargo.toml`（`gpui = { version = "=0.2.2", features = ["runtime_shaders"] }`）、`README.md`（Metal Toolchain 建置說明）
- Reference: `spikes/gpui-notch/src/main.rs`（第 333–400 行附近是繞路的實測寫法）

**Interfaces:**
- Produces（`shell` 與 `app` 只准呼叫這幾個函式）：
  ```rust
  pub fn set_accessory_activation_policy();                      // 不顯示 Dock 圖示，必須在建立視窗前呼叫
  pub fn patch_gpui_panel_class();                               // 修補 GPUIPanel 的 canBecomeKeyWindow／canBecomeMainWindow 為 NO；找不到類別時印警告不崩潰
  pub fn configure_panel(handle: NativeWindow, frame: Rect);     // 事後切成無邊框、透明、不搶焦點、層級高於選單列（25）、跨所有空間、定位到 frame
  pub fn list_screens() -> Vec<ScreenInfo>;                      // NSScreen，含 auxiliaryTopLeftArea／TopRightArea 推出的瀏海寬高，座標轉成左上角原點
  pub fn mouse_location() -> Point;                              // 全域滑鼠位置，已轉換座標
  pub fn foreground_is_fullscreen() -> bool;                     // 見 Step 4
  pub struct NativeWindow(pub *mut std::ffi::c_void);
  ```

- [ ] **Step 1: 純邏輯測試。** `screens.rs` 中把「由 NSScreen 的數值算出 `ScreenInfo`」抽成純函式 `screen_info_from(frame, aux_left_w, aux_right_w, aux_h, ...)`，測：瀏海寬 = 螢幕寬 − 左 − 右；任一輔助區為零或寬度非正則 `hardware_notch == None`；y 軸轉換正確（AppKit 左下原點 → 左上原點）。Run `cargo test -p musicat-platform-macos screens`；Expected: 先 FAIL 後 PASS。
- [ ] **Step 2: 實作 `panel.rs`。** 四個繞路逐項搬自 spike，各自一個函式，並在每個函式上方註解寫「為什麼需要」與「對應 GPUI 0.2.2」（spec 第 7.3 節）：事後去邊框、`GPUIPanel` 類別修補、accessory 啟動模式；第四項（`runtime_shaders`）是 Cargo feature，在 `shell/Cargo.toml` 與 README 註明，並分別以有／無 Metal Toolchain 的環境各建置一次記錄結果。
- [ ] **Step 3: 手動驗證（需要作者的 Mac，列入 `HANDOFF-rust.md`）。** 以 `rust/scripts/run.sh` 跑最小視窗，逐項確認 spike 未能自動驗證的三項：點擊穿透（面板外的點擊落到下層視窗）、跨空間與全螢幕空間時面板的表現、點擊面板不搶焦點也不啟動 App。結果寫回 spec 第 7.2 節的表格。
- [ ] **Step 4: 全螢幕偵測的查證。** 先實測 macOS 上我們的跨空間視窗在其他 App 全螢幕時是否自然被系統隱藏；若是，`foreground_is_fullscreen()` 回 `false` 並在函式註解記錄實測結果，**不要硬加偵測**（spec 第 5.3 節）；若不是，用 `CGWindowListCopyWindowInfo` 判斷前景視窗是否覆蓋整個螢幕。把結論與證據（日誌或截圖）放進 PR 說明回報作者。
- [ ] **Step 5: Run** `cargo build -p musicat-platform-macos && cargo test -p musicat-platform-macos`；Expected: PASS。Commit: `feat(platform-macos): NSPanel workarounds, screen listing and fullscreen policy`。

## Task 22：選單列圖示小 spike 與實作

spec 第 13 節：spike 沒驗證過 GPUI 事件迴圈（加 accessory 啟動與焦點修補）能否與狀態列圖示共存。

**Files:**
- Create: `rust/platform-macos/src/tray.rs`、`rust/docs/adr-0002-menu-bar-item.md`

**Interfaces:**
- Produces:
  ```rust
  pub struct MenuModel { pub items: Vec<MenuItem> }
  pub enum MenuItem { Toggle { id: &'static str, label: String, checked: bool }, ScreenChoice { screen_id: Option<String>, label: String, checked: bool }, Separator, Quit }
  pub struct MenuBar;   // new(icon_png: &[u8]) -> Self; set_model(&self, MenuModel); on_event(&self, Box<dyn Fn(MenuEvent) + Send + Sync>)
  pub enum MenuEvent { Toggled(&'static str, bool), ScreenChosen(Option<String>), Quit }
  ```
- 選單內容固定（spec 第 3 節）：切換模式（本輪只列「瀏海式」，浮動視窗在 R4）、選擇顯示螢幕（含「自動」）、各歌詞來源的開關（LRCLIB、網易雲）、macOS perl 宿主開關、歌詞條顯示、結束。

- [ ] **Step 1: 可行性 spike。** 先確認 GPUI 0.2.2 是否有狀態列 API（查 crate 原始碼）；沒有就試 `tray-icon` 與 GPUI 事件迴圈共存（需要在主執行緒、`NSApplication` 已啟動之後建立），再不行用 `objc2` 直接建 `NSStatusItem`。把嘗試過的做法與結論寫進 `adr-0002-menu-bar-item.md`。驗收：accessory 模式與 `patch_gpui_panel_class` 都啟用時，圖示出現、選單可開、點選項目觸發 `MenuEvent`，且瀏海面板的 hover 不受影響。
- [ ] **Step 2: 為 `MenuModel` 寫純函式測試。** 抽出 `build_menu(config_view) -> MenuModel` 到 `app`（見 Task 25），在這裡先測 `MenuModel` 的結構轉換（`MenuItem` 轉成平台項目的對應不依賴真實 AppKit）。
- [ ] **Step 3: 實作並手動驗證**（列入 `HANDOFF-rust.md`）。Commit: `feat(platform-macos): menu bar item`。

## Task 23：`shell` 的視窗、固定容器與動畫

**Files:**
- Create: `rust/shell/src/window.rs`、`lib.rs`
- Test: 動畫曲線與插值為純函式，放 `rust/core/src/shell_logic/geometry.rs`

**Interfaces:**
- Consumes: `NotchGeometry`、`container_frame`、`mode_size`、`HoverMachine`、`platform_macos::{configure_panel, mouse_location, patch_gpui_panel_class, set_accessory_activation_policy}`。
- Produces:
  ```rust
  // core::shell_logic::geometry
  pub const ANIMATION_SECONDS: f64 = 0.28;
  pub fn ease_in_out(t: f64) -> f64;                                   // t 夾到 0..=1，ease_in_out(0)=0，(1)=1，(0.5)=0.5
  pub fn lerp_size(from: Size, to: Size, t: f64) -> Size;
  // shell
  pub struct ShellWindow;   // open(cx, geometry: NotchGeometry, content: Arc<dyn ShellContent>) -> ShellWindow；reposition(&self, geometry: NotchGeometry)；set_visible(&self, bool)
  pub trait ShellContent { fn preview(&self, cx) -> AnyElement; fn expanded(&self, cx) -> AnyElement; fn preview_extra_height(&self) -> f64; fn on_expand(&self); }
  ```
- 行為：GPUI 視窗大小固定為 `container_frame`，終身不改；內容以頂端置中、上方直角、下方圓角（Preview 10、Expanded 24）繪出目前插值的大小；每個畫面重繪週期（動畫中約 60 fps，閒置時降頻）讀 `mouse_location()` 餵給 `HoverMachine`；`just_expanded` 時呼叫 `ShellContent::on_expand`；`next_deadline` 用 `Spawner` 的計時安排下一次更新；`reposition`（螢幕變更）只移動視窗位置，不改大小。

- [ ] **Step 1: 純函式測試（失敗）。** `ease_in_out_endpoints_and_midpoint`、`lerp_size_interpolates_both_axes`、`ease_in_out_clamps_out_of_range_t`。Run `cargo test -p musicat-core shell_logic::geometry`；Expected: FAIL → 實作 → PASS。
- [ ] **Step 2: 實作 `ShellWindow`。** 先用兩個純色佔位內容（Preview 黑條、Expanded 黑色圓角矩形）把外殼行為跑起來。
- [ ] **Step 3: 手動驗收**（對應 HANDOFF 第 5、7 項；列入 `HANDOFF-rust.md`）：hover 不閃爍（游標靜止在 Preview 邊緣不抖）、展開／收合約 0.28 秒、展開後游標小幅晃動不收合、離開後 150 毫秒才收合、事件日誌格式沿用 `spikes/gpui-notch/tools/hover.swift` 的驗證方式。
- [ ] **Step 4: Commit:** `feat(shell): fixed-container panel window with animated Preview/Expanded`。

## Task 24：`shell` 的 Music 內容（Preview 與 Expanded）

**Files:**
- Create: `rust/shell/src/music_views.rs`、`marquee.rs`
- Test: `marquee.rs` 純邏輯；移植 `MarqueeTextTests`（4）

**Interfaces:**
- Consumes: `MusicController`、`MusicState`、`strip_content`、`progress_fraction`、`has_progress`、`MusicModuleState`。
- Produces: `pub struct MusicShellContent`，實作 `ShellContent`（Task 23）。

- [ ] **Step 1: 移植跑馬燈測試（失敗）。** 4 案（文字寬度未超過容器時不捲動、超過時捲動週期與暫停、文字變更時重置位置）；Run `cargo test -p musicat-shell marquee`；Expected: FAIL → 實作 → PASS。
- [ ] **Step 2: 畫 Preview。** 左翼封面（置中裁切成方形，缺席時佔位）、右翼動態音柱（只在播放中動，暫停靜止）、瀏海正下方的歌詞條（高 24、黑底不透明度 0.6，各為具名常數；`strip_content` 決定顯示行或曲名，純文字歌詞永遠顯示曲名）；左右翼寬 = `(寬 − 瀏海寬) / 2`；真瀏海的中央那段不放重要內容。
- [ ] **Step 3: 畫 Expanded。** 上方模組切換列（本輪只有 Music 一個，仍要有此列並在每次 `on_expand` 時重設選取）；封面、曲名、演出者、進度條（`has_progress` 為假就不畫）、上一首／播放暫停／下一首（`has_control == false` 時停用）、歌詞（同步且位置已知時捲動並標示當前行，否則純文字靜態全文）、歌詞條折疊開關；沒有曲目只顯示一行「目前沒有播放」。中文字渲染須正常（spike 已驗證）。
- [ ] **Step 4: 手動驗收**（對應 HANDOFF 第 1、2、3、7、8、14、15 項）。
- [ ] **Step 5: Commit:** `feat(shell): Music Preview and Expanded content`。

## Task 25：`app` 組裝、TOML 設定、ureq HTTP

**Files:**
- Create: `rust/app/src/main.rs`、`config.rs`、`http_ureq.rs`
- Test: `config.rs`；移植 `LyricStripPreferenceTests`（2，改測設定項 `lyric_strip_visible`：預設為真、變更後存檔再讀回仍成立）

**Interfaces:**
- Consumes: 幾乎所有前面任務的成品。
- Produces:
  ```rust
  pub struct Config { pub mode: Mode /*本輪只有 Notch*/, pub screen: Option<String>,
                      pub sources: Sources /*lrclib: bool, netease: bool，預設皆 true*/,
                      pub macos: MacosConfig /*perl_host: bool，預設 true*/,
                      pub notch: NotchConfig /*virtual_width 200, virtual_height 32*/,
                      pub lyric_strip_visible: bool /*預設 true*/ }
  pub fn config_path() -> PathBuf;                  // 各平台標準設定目錄，檔名 musicat.toml
  pub fn load(path: &Path) -> (Config, Vec<String>); // 回傳設定與警告；永遠成功
  pub fn save(path: &Path, cfg: &Config) -> io::Result<()>;   // 原子寫入（暫存檔再改名）
  pub struct UreqHttp;   // 實作 HttpClient：在執行緒池上跑 ureq 的阻塞呼叫，以 oneshot 回傳；本體超過 MAX_BODY_BYTES 回 HttpError::TooLarge
  ```

- [ ] **Step 1: 寫失敗測試（Review Focus 第 5 條）。** `missing_file_yields_defaults_without_warnings`、`syntactically_broken_toml_yields_defaults_and_a_warning_and_does_not_overwrite_the_file`、`unknown_keys_are_ignored_with_a_warning`、`wrong_typed_value_falls_back_to_that_fields_default_only`（其他欄位保留）、`screen_id_that_no_longer_exists_is_kept_in_the_file`（不因為暫時拔掉螢幕就丟掉使用者的選擇）、`save_is_atomic_and_round_trips`、`lyric_strip_visible_defaults_to_true_and_persists`、`perl_host_false_means_macos_source_list_excludes_it`。
- [ ] **Step 2: Run** `cargo test -p musicat-app config`；Expected: FAIL → 實作 → PASS。
- [ ] **Step 3: 組裝 `main`。** 順序：`set_accessory_activation_policy`、`patch_gpui_panel_class`、讀設定、建 `UreqHttp` 與 `Clock`（以 GPUI 計時器實作）與 `Spawner`（以 GPUI 背景執行器實作，回傳的 `Task` 包成 `TaskHandle`）；現正播放來源清單 = `[HelperSupervisor(PerlLauncher)（`perl_host` 開啟且 dylib 可用時）, WindowTitleSource]`；提供者清單依設定啟用 `LrclibProvider`、`NetEaseProvider`；建 `LyricsRepository`、`NowPlayingMonitor`、`MusicController`、`MusicModuleState`、`ModuleManager`；以 `choose_screen` 與 `geometry_for` 開面板；定時重新 `list_screens()`（並在螢幕參數變更通知時）偵測螢幕增減，面板用 `reposition` 移到預設螢幕或選定螢幕，**不結束程式**；選單事件寫回 `Config` 並存檔，來源或 perl 宿主開關即時生效。
- [ ] **Step 4: 手動驗證**：從選單切換螢幕、關閉 perl 宿主後僅剩視窗標題備援、拔插外接螢幕時面板移動且程式不結束（對應 Review Focus 第 4 條）。
- [ ] **Step 5: Commit:** `feat(app): compose MusiCat, TOML config and menu wiring`。

## Task 26：執行腳本、量測、驗收清單

**Files:**
- Create: `rust/scripts/run.sh`、`rust/scripts/measure.sh`、`rust/docs/HANDOFF-rust.md`

- [ ] **Step 1: `run.sh`。** 等同 `scripts/run.sh`：`cd` 到 `rust/`，`cargo build --release -p musicat-app`（`--release`，因為 spike 的 GPUI 透明度以 release 執行檔驗證）後 `exec target/release/musicat`；Metal Toolchain 沒裝時的指示印在失敗訊息裡。Run: `sh rust/scripts/run.sh`；Expected: 面板出現在瀏海。
- [ ] **Step 2: `measure.sh`。** 取樣 `musicat` 與其 perl 子行程的 CPU 與 RSS（`ps -o %cpu,rss -p`），每 10 秒一次、預設 1 小時，輸出 CSV；用於驗收清單第 12、13 項與 spec 第 10 節「記憶體與 CPU」。成功判準寫進 `HANDOFF-rust.md`：1 小時內 RSS 無持續上升趨勢（首尾差小於 10%）；閒置 CPU 與 spike 的 1.4% 同一個量級。
- [ ] **Step 3: `HANDOFF-rust.md`。** 複製 `docs/HANDOFF.md` 的 20 項清單，**項目內容與判定標準不變**，只改名稱與指令：`pkill -f NotchCatMediaRemoteHelper` → 依 Task 13 的新名稱 `pkill -f musicat_helper_main`；`scripts/run.sh` → `rust/scripts/run.sh`；`swift build` 重新連結動態庫 → 重新 `cargo build`；`.scrollIndicators(.hidden)` 相關兩項（第 6、16）改為「GPUI 歌詞捲動區不顯示捲軸，含系統設定『永遠顯示捲動條』」。再附加 spec 第 12 節的第 2、3 項（GPUI spike 未驗證的三項、全螢幕時面板在 macOS 的實際行為），以及 Task 22 的選單列驗收。
- [ ] **Step 4: 整體驗證並提交。** Run: `cd rust && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && sh scripts/check-core-purity.sh && sh scripts/check-private-api.sh && sh scripts/parity.sh`；Expected: 全部通過，`parity.sh`（不帶參數）對全部列通過。Commit: `docs(rust): run script, measurement script and Rust handoff checklist`。

## Task 27：R2 實機驗收（作者執行）

- [ ] **Step 1:** 作者在自己的 Mac 上依 `rust/docs/HANDOFF-rust.md` 逐項驗收，結果填回該檔。builder 不替作者勾選任何一項。
- [ ] **Step 2:** 驗收中發現的問題各開一個小修正任務（先寫能重現的測試或事件日誌，再修）；歌詞條透明度與高度的調整結果寫回 Music spec 與本計畫的 Global Constraints。
- [ ] **Step 3: 停止點。** 全部通過後回報作者；**由作者下令後**，才以一個獨立的 commit 移除 Swift 版（`Package.swift`、`Sources/`、`Tests/`、`scripts/run.sh` 等）。移除後是否把 `rust/` 內容搬到 repo 根目錄，是作者另行的決定。

---

# R3–R5 里程碑（等 R2 通過後再細化）

- **R3 Windows（全部標示「未在實機驗證」）。** (1) 先做 spec 第 7.4 節的 GPUI 驗證：透明置頂無邊框、不進工作列、不搶焦點、多螢幕與 DPI、hover 不閃爍、前景全螢幕時隱藏；作者仍無法實測時，把這六項列成「待驗證」清單交付。若 GPUI 在任一項做不到，採 spec 第 7.2 節備案並回報。(2) `platform-windows`：SMTC（`GlobalSystemMediaTransportControlsSessionManager`）實作 `NowPlayingSource` 與 `PlaybackControl`，先查證時間軸欄位語意（播放中位置是否自行前進、最後更新時間的基準）；封面縮圖以識別碼快取、置中裁切。(3) 假瀏海面板（重用 Task 19、20、23；視窗類型對應 `WS_EX_TOOLWINDOW`／`WS_EX_NOACTIVATE`）。(4) 系統匣選單。(5) CI 的 Windows job 擴大到編譯 `shell` 與 `app`。(6) 附一份 Windows 實機驗收清單。
- **R4 桌面浮動視窗。** `shell` 新增第二種外殼（同一份 `MusicShellContent`）：封面、曲名、演出者、進度條、控制鈕、歌詞；兩種大小（一般、歌詞專用）；可拖動、記住位置（TOML）、置頂切換、透明度；與瀏海模式互斥；選單的「切換模式」開放第二個選項；不做換膚。
- **R5 歌詞庫後續能力。** 本地來源插在提供者清單最前面（Task 7 已留好擴充點）、編輯並覆蓋、每首歌 ±0.5 秒偏移、「這份不對」黑名單、資料夾加中繼資料檔的儲存格式、AI agent 待辦清單與資料夾監看、git 版控與遠端分享、README 的著作權說明；可再拆成多個小階段。

---

## 自我檢查結果

- **Spec 覆蓋：** 第 3 節決定表——語言、名稱、授權（T1）、GPUI 與鎖版（T21、Global）、macOS 先做（R2）、虛擬瀏海與螢幕選擇與拔除（T19、T25）、全螢幕隱藏（T20、T21）、浮動視窗互斥（R4）、選單與 TOML（T22、T25）、perl 宿主可關（T16、T25）、Swift 去留（T27）、core 非同步（T2）、macOS 繞路單一模組（T21）、repo 結構、CI（T1）。第 4.1 節行為——現正播放優先序（T8）、位置公式（T3）、歌詞來源與可信度與快取（T4–T7）、標題清理（T5）、畫面內容（T9、T24）。第 5.3 節硬性設計——固定容器、hover、150 毫秒、數值（T19、T20、T23）。第 6 節平台——perl 宿主（T13–T16）、播放控制（T17）、視窗標題（T18）、Windows（R3）。第 7.3 節繞路（T21）、第 13 節各項「留給 builder」（T2 ADR、T22 選單列 spike、T26 名稱對應）。第 10 節測試策略的行程監管與長時間量測（T15、T26）。
- **未在本計畫實作、已註明原因：** 打包、簽章、安裝程式與 release 工作（spec 第 13 節明列不在 R1、R2）。
- **型別一致性：** `TrackSnapshot.artist` 為 `Option<String>`，`TrackIdentity.artist` 為 `String`（T3、T5）；`LyricsResolving::lyrics` 取 `TrackIdentity` 傳值（T7、T10）；`NowPlayingSource::subscribe` 的回呼簽章一致（T8、T15、T18）；時間一律 `f64` Unix 秒（`Clock::now`、`PositionAnchor::timestamp`、`HoverInput::now`）。
- **比例：** 本計畫以簽章、測試名稱與規則為主，沒有函式本體；唯一寫出具體演算法的是 Task 2 的設計定案與 Task 15 的狀態處理規則，因為簽章與測試無法決定它們。
