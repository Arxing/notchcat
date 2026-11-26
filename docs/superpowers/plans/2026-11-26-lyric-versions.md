# 歌詞版本切換與時間微調 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 展開畫面能在同一首歌的多個歌詞時間軸版本間切換、微調 ±0.5 秒，並按歌曲記住。

**Architecture:** 提供者改為回傳所有合格候選；`LyricsRepository` 同時查所有來源，把時間軸相同的候選合併成排序好的版本清單。`MusicController` 持有清單、目前版本與偏移，偏移直接套在輸出的歌詞行時間上（呈現層不改）。記住的選擇是 core 的純資料，檔案讀寫在 app。

**Tech Stack:** Rust workspace `rust/`（`musicat-core`、`musicat-shell`、`musicat-app`）、GPUI =0.2.2、futures-util、serde／serde_json。

**Spec:** `docs/superpowers/specs/2026-11-26-lyric-versions-design.md`

## Global Constraints

- 版本只含時間軸歌詞；候選必須歌名相符、曲長差 ≤ 10 秒、`is_plausible`。純文字只在沒有任何時間軸版本時當保底。
- LRCLIB：精確查詢、原文搜尋、簡體搜尋（`simplified_chinese()` 有值時）三個請求的所有合格紀錄。網易雲：最多前 3 首合格歌曲的歌詞。
- 來源同時查、等全部回來；每個來源上限 5 秒；關掉的來源不發請求。有任一來源失敗時清單只快取 120 秒，否則成功永久、查無 24 小時。
- 合併：配到的相同文字行（忽略繁簡、空白、標點）≥ 較短那份的 60%，且時間差絕對值的中位數 ≤ 1.0 秒。
- 偏移：每次 0.5 秒，Option 點擊 5 秒，範圍 ±120 秒；**＋ 讓歌詞提早出現**；偏移按版本分開。
- 記憶：只在切換或調偏移時寫入；以 `cache_key` 加曲長（差 ≤ 2 秒；未知只配未知）辨認；存網址、歌詞行、各版本偏移；檔名 `lyric-choices.json`，與設定檔同資料夾，原子寫入，壞檔改名 `.bak` 保留。
- `musicat-core` 不碰檔案、不得出現 `cfg(target_os)`；不新增網路服務。
- 介面文字：`時間軸版 1/3 ›`、`時間軸版`、`0.0s`、`+0.5s`、`−1.0s`（減號用 U+2212）。
- 每個任務結束時在 `rust/` 通過：`cargo test --workspace`、`cargo clippy --workspace --all-targets`（零警告）、`cargo fmt --all --check`、`sh scripts/check-core-purity.sh`、`sh scripts/check-private-api.sh`、`sh scripts/parity.sh`。
- 分支 `feature/lyric-versions`；不合併、不推送。commit 訊息結尾：`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

1. 一個來源卡住：另一個來源的版本仍在 5 秒後顯示（Task 3 測試）。
2. 紀錄檔損毀：App 照常啟動，原檔不被無聲覆蓋（Task 6）。
3. 查詢中或換歌瞬間按切換／偏移：不得寫到別首歌，也不寫入紀錄（Task 5）。
4. 播放器晚一拍才回報曲長：記住的選擇仍要配得上（Task 5）。
5. 同一版本因製作人員列、繁簡、標點不同被拆成多個版本（Task 1）。

---

### Task 1: 版本合併

**Files:**
- Create: `rust/core/src/lyrics/versions.rs`
- Modify: `rust/core/src/lyrics/mod.rs`（`pub mod versions;`、`LyricsHit`）、`identity.rs`、`lrclib.rs`、`netease.rs`、`repository.rs`（配合 `with_fit` 改名）

**Interfaces:**
- Produces:
  - `TrackIdentity::duration_gap(&self, other: Option<f64>) -> Option<f64>`：秒差絕對值；任一邊缺少、≤ 0 或 NaN 為 `None`。`duration_fit` 以它實作，行為不變。
  - `LyricsHit.duration_gap: Option<f64>`（`new` 為 `None`）。
  - `LyricsHit::matched(self, track: &TrackIdentity, record_duration: Option<f64>) -> Option<Self>`：取代 `with_fit`；超過 10 秒為 `None`，否則設好 `loose_duration` 與 `duration_gap`。
  - `versions::same_version(a: &[LyricLine], b: &[LyricLine]) -> bool`
  - `versions::merge_versions(hits: Vec<LyricsHit>) -> Vec<LyricsHit>`：忽略非時間軸；同網址只算一次；回傳各版本的代表，預設版本在第一個。代表＝組內 `(loose_duration, duration_gap 未知當 0, 輸入順序)` 最小者。組的排序＝含非放寬成員者先、成員數多者先、最早輸入者先。

- [ ] **Step 1: 寫失敗測試**（`versions.rs` 內）

```rust
fn lines(shift: f64, texts: &[&str]) -> Vec<LyricLine> {
    texts.iter().enumerate()
        .map(|(i, t)| LyricLine { time: 10.0 * i as f64 + shift, text: (*t).into() })
        .collect()
}
fn hit(id: u32, l: Vec<LyricLine>) -> LyricsHit {
    LyricsHit::new(Lyrics::Synced(l), format!("https://test.example/{id}"))
}
const A: [&str; 5] = ["泛黃的春聯", "還殘留在牆上", "依稀可見幾個字", "歲歲平安", "米缸"];
const A_SIMPLIFIED: [&str; 5] = ["泛黄的春联", "还残留在 墙上，", "依稀可见几个字", "岁岁平安", "米缸"];

#[test] fn small_timing_differences_are_one_version() {
    assert!(same_version(&lines(0.0, &A), &lines(1.0, &A)));
    assert!(!same_version(&lines(0.0, &A), &lines(1.1, &A)));
    assert!(!same_version(&lines(0.0, &A), &lines(5.0, &A)));
}
#[test] fn script_spacing_punctuation_and_credit_lines_do_not_split_a_version() {
    let mut with_credits = lines(0.2, &A_SIMPLIFIED);
    with_credits.insert(0, LyricLine { time: 0.0, text: "作词 : 方文山".into() });
    assert!(same_version(&lines(0.0, &A), &with_credits));
}
#[test] fn too_few_matching_lines_are_a_different_version() {
    let other = lines(0.0, &["泛黃的春聯", "還殘留在牆上", "別的", "不同", "句子"]);
    assert!(!same_version(&lines(0.0, &A), &other)); // 2/5 < 60%
    assert!(!same_version(&[], &lines(0.0, &A)));
}
#[test] fn repeated_lines_pair_in_order() {
    let chorus = ["啦", "啦", "啦", "啦"];
    assert!(same_version(&lines(0.0, &chorus), &lines(0.5, &chorus)));
}
#[test] fn the_largest_group_comes_first() {
    let got = merge_versions(vec![hit(1, lines(5.0, &A)), hit(2, lines(0.0, &A)), hit(3, lines(0.4, &A))]);
    assert_eq!(urls(&got), ["https://test.example/2", "https://test.example/1"]);
}
#[test] fn a_group_with_a_close_duration_outranks_a_bigger_loose_one() {
    let loose = |id, s| { let mut h = hit(id, lines(s, &A)); h.loose_duration = true; h };
    let got = merge_versions(vec![loose(1, 0.0), loose(2, 0.1), hit(3, lines(5.0, &A))]);
    assert_eq!(urls(&got)[0], "https://test.example/3");
}
#[test] fn the_representative_has_the_closest_duration() {
    let gap = |id, g| { let mut h = hit(id, lines(0.0, &A)); h.duration_gap = Some(g); h };
    assert_eq!(urls(&merge_versions(vec![gap(1, 3.0), gap(2, 0.5), gap(3, 0.5)])), ["https://test.example/2"]);
}
#[test] fn the_same_url_is_counted_once_and_plain_lyrics_are_ignored() {
    let got = merge_versions(vec![
        hit(1, lines(0.0, &A)), hit(2, lines(5.0, &A)), hit(2, lines(5.0, &A)),
        LyricsHit::new(Lyrics::Plain("x".into()), "https://test.example/9"),
    ]);
    assert_eq!(urls(&got), ["https://test.example/1", "https://test.example/2"]);
    assert!(merge_versions(Vec::new()).is_empty());
}
```

- [ ] **Step 2:** `cargo test -p musicat-core versions`，預期因缺少函式失敗（先放空殼讓失敗是斷言而非編譯錯誤）。
- [ ] **Step 3:** 實作。行的比對鍵：沿用 `identity::fold`（改 `pub(super)`）後只留 `char::is_alphanumeric`；鍵為空的行略過；配對鍵為「文字＋第幾次出現」；分組為貪婪法，和各組第一個成員比。每筆只摺疊一次。
- [ ] **Step 4:** 全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(lyrics): merge candidates into timing versions"`

---

### Task 2: 提供者回傳所有候選

**Files:**
- Modify: `rust/core/src/lyrics/{provider.rs, lrclib.rs, netease.rs, repository.rs}`

**Interfaces:**
- Consumes: `LyricsHit::matched`。
- Produces:
  - `pub struct Candidates { pub hits: Vec<LyricsHit>, pub complete: bool }`
  - `LyricsProvider::fetch(&self, track) -> BoxFuture<'_, Result<Candidates, ProviderError>>`
  - 規則：沒有任何候選且有請求失敗 → `Err`（依請求順序取第一個錯誤）；有候選但有請求失敗 → `complete: false`。
- `LyricsRepository` 本任務維持現有行為：逐來源，用 `keep_better` 取最好的一筆；`complete == false` 視同 `any_failed`。

- [ ] **Step 1: 寫失敗測試**

```rust
// lrclib.rs
#[test] fn lrclib_collects_every_eligible_record_from_all_three_requests() {
    // exact → id 1；原文搜尋 → id 1、2，外加一筆歌名不符與一筆差 20 秒；簡體搜尋 → id 3
    // hits 的網址依序為 /api/get/1、/2、/3（同網址只一次）；complete == true；3 個請求
}
#[test] fn lrclib_partial_failure_returns_what_it_has_as_incomplete() {
    // 原文搜尋有一筆、簡體搜尋 500 → hits 1 筆、complete == false
    // 三個請求都沒有候選且簡體搜尋 500 → Err(BadStatus(500))
}
// netease.rs
#[test] fn net_ease_fetches_up_to_three_eligible_songs_close_ones_first() {
    // 搜尋回 id 1(208s 放寬)、2(200s)、3(201s)、4(202s)、5(歌名不符)；
    // 歌詞請求只有 2、3、4，hits 依此順序
}
#[test] fn net_ease_a_failed_lyric_request_leaves_the_others() {
    // 兩首，一首歌詞 500 → 1 筆、complete == false
}
```

- [ ] **Step 2:** `cargo test -p musicat-core lyrics`，預期新測試失敗。
- [ ] **Step 3:** 實作。同一來源的請求以 `futures_util::future::join_all` 同時發出。`NetEaseProvider::select_song` 改為 `select_songs(data, track) -> Result<Vec<(i64, Option<f64> /* 秒 */)>, ProviderError>`（吻合者在前、各自保留順序、最多 3）。
- [ ] **Step 4:** 更新既有測試：兩個提供者的測試模組各加 `fetch_best` 輔助（`Candidates` → 最好的一筆）沿用原斷言。因為搜尋現在一定會發，改寫斷言請求數的案例（`lrclib_exact_lookup_sends_normalized_parameters_and_user_agent`、`lrclib_omits_duration_and_album_when_unknown`、`lrclib_exact_404_falls_through_to_search`、`lrclib_server_error_on_exact_lookup_throws`、`lrclib_transport_error_propagates`），以及簡體重試那組（改名為描述「一定搜兩種字」的行為，`lrclib_does_not_retry_after_a_close_synced_hit` 刪除）。全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(lyrics): providers return every eligible candidate"`

---

### Task 3: 解析器回傳版本清單

**Files:**
- Modify: `rust/core/src/lyrics/repository.rs`、`rust/core/src/music/{controller.rs, module.rs}`、`rust/app/src/switchable.rs`、`rust/app/examples/lyrics_demo.rs`、`rust/shell/examples/music_demo.rs`

**Interfaces:**
- Consumes: `merge_versions`、`Candidates`。
- Produces: `LyricsResolving::lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Vec<LyricsHit>>`。有時間軸：合併後的版本清單（預設在前）；否則至多一筆純文字；查無：空。
- 控制器本任務只取 `first()`，畫面行為不變。

- [ ] **Step 1: 寫失敗測試**（`repository.rs`）

```rust
#[test] fn every_provider_is_queried_and_versions_are_merged() {
    // a 回 synced()、b 回位移 5 秒的版本、c 回與 b 同時間軸 → 清單兩筆，b 那組在前
}
#[test] fn providers_are_queried_at_the_same_time() {
    // 兩個來源各延遲 3 秒：advance(3s) 後查詢已完成
}
#[test] fn a_hanging_provider_does_not_hide_the_others_versions() {
    // hanging + 立即回答：advance(5s) 後拿到另一個的版本
}
#[test] fn a_partial_version_list_expires_after_failure_ttl() {
    // 一個失敗、一個有版本：59 秒內走快取，之後重查
}
#[test] fn plain_is_returned_alone_only_when_nothing_is_synced() {}
```

- [ ] **Step 2:** 執行確認失敗。
- [ ] **Step 3:** 實作：`join_all` 各來源的 `fetch`（各自 `with_timeout`）→ 依來源順序串接 → 濾掉 `!is_plausible` → 合併或取最好的純文字。`CacheEntry::Found { hits, until: any_failed.then(..) }`。
- [ ] **Step 4:** 更新既有測試（`stops_at_first_synced_result` 刪除；`synced_result_stays_cached_even_if_an_earlier_provider_failed` 改為上面的過期行為；其餘以 `first()` 沿用斷言）與所有 `LyricsResolving` 實作。`lyrics_demo` 逐一印出版本：序號、第一行時間、網址。全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(lyrics): resolve a list of versions from all sources"`

---

### Task 4: 記住選擇的純資料

**Files:**
- Create: `rust/core/src/music/choices.rs`；Modify: `rust/core/src/music/mod.rs`

**Interfaces:**
- Produces:
  - `pub const OFFSET_LIMIT: f64 = 120.0;`
  - `pub struct SavedChoice { pub key: String, pub duration: Option<f64>, pub url: String, pub lines: Vec<LyricLine>, pub offsets: BTreeMap<String, f64> }`
  - `pub struct ChoiceBook { pub songs: Vec<SavedChoice> }`（`Default`）；`find(&self, track: &TrackIdentity) -> Option<&SavedChoice>`；`upsert(&mut self, choice: SavedChoice)`
  - `apply_choice(versions: Vec<LyricsHit>, saved: Option<&SavedChoice>) -> (Vec<LyricsHit>, usize)`
  - `shift_lyrics(lines: &[LyricLine], offset: f64) -> Vec<LyricLine>`（每行 `time - offset`，允許負值）
  - `clamp_offset(v: f64) -> f64`（±120；NaN → 0）
- `apply_choice` 順序：網址相同 → 選它（用新查到的內容）；否則與某版本 `same_version` → 以存下的內容取代它並選它；否則清單是時間軸 → 附加在最後並選它；否則（純文字或空）→ 只剩存下的版本。

- [ ] **Step 1: 寫失敗測試**：`find_matches_key_and_duration_within_two_seconds`（200 配 202、不配 202.1；未知只配未知；key 不同不配）、`upsert_replaces_the_same_song_and_keeps_others`、`apply_choice_without_a_saved_choice_picks_the_first`、`apply_choice_covers_url_same_timing_missing_and_plain`（上述四種各一個斷言清單網址與索引）、`shift_moves_lines_earlier_for_positive_offsets`（10.0 以 0.5 → 9.5；0.0 以 0.5 → -0.5）、`offsets_are_clamped`（500 → 120、-500 → -120、NaN → 0）。
- [ ] **Step 2:** 執行確認失敗。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(music): remember the chosen lyric version and offsets"`

---

### Task 5: 控制器的版本與偏移

**Files:**
- Modify: `rust/core/src/music/controller.rs`

**Interfaces:**
- Consumes: Task 3 的清單、Task 4 全部。
- Produces:
  - `pub struct VersionInfo { pub index: usize, pub count: usize, pub offset: f64 }`
  - `MusicState.version: Option<VersionInfo>`（只有目前顯示時間軸歌詞時為 `Some`）
  - `pub type ChoiceSaver = Arc<dyn Fn(&ChoiceBook) + Send + Sync>;`
  - `MusicController::new_with_choices(monitor, resolver, control, spawner, book: ChoiceBook, save: ChoiceSaver) -> Arc<Self>`；`new` 改為以空紀錄與空函式呼叫它。
  - `next_version(&self)`、`nudge_offset(&self, delta: f64)`、`reset_offset(&self)`
- `state.lyrics` 是套完偏移（再依開關轉繁）的版本；`lyrics_url` 是目前版本的網址。
- 配紀錄與存紀錄用**目前快照**的 identity（曲長可能晚到），不是發出查詢時的。
- 沒有 `version`、清單少於兩個（對 `next_version`）、偏移沒有改變 → 什麼都不做、不存。`save` 在鎖外呼叫。

- [ ] **Step 1: 寫失敗測試**（`GatedResolver::respond_versions(index, Vec<LyricsHit>)`，`respond` 沿用）

```rust
#[test] fn first_version_is_shown_with_url_and_version_info() {} // version == Some{0, 2, 0.0}
#[test] fn next_version_cycles_and_saves_the_choice() {}          // 0→1→0；save 收到目前網址與未偏移的行
#[test] fn nudging_shifts_the_shown_lines_and_saves() {}           // +0.5：10.0 → 9.5；存的行仍是 10.0
#[test] fn offsets_belong_to_their_version() {}                    // 版本 0 調 +0.5，切到 1 為 0.0，切回為 0.5
#[test] fn offset_is_clamped_and_reset_returns_to_zero() {}        // +500 → 120；reset → 0；再 reset 不存
#[test] fn a_saved_choice_is_selected_with_its_offset() {}
#[test] fn a_saved_choice_shows_when_the_lookup_finds_nothing() {}
#[test] fn plain_missing_or_searching_lyrics_have_no_controls_and_save_nothing() {}
#[test] fn playing_without_touching_the_controls_saves_nothing() {}
#[test] fn actions_after_a_track_change_never_write_the_previous_song() {}
#[test] fn a_duration_that_arrives_late_still_matches_the_saved_choice() {} // 先 None，回答前更新為 200
#[test] fn the_traditional_toggle_keeps_the_offset() {}
```

每個測試以註解所列的值寫成斷言。

- [ ] **Step 2:** 執行確認失敗。
- [ ] **Step 3:** 實作。`Core` 持有 `versions`、`index`、本曲 `offsets`、`book`；以單一函式由它們重建 `source`／`state.lyrics`／`lyrics_url`／`version`，查詢完成、三個動作與換曲清空都走它。
- [ ] **Step 4:** 全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(music): switch lyric versions and adjust their timing"`

---

### Task 6: 紀錄檔

**Files:**
- Create: `rust/app/src/choice_file.rs`；Modify: `rust/app/src/{lib.rs, config.rs, main.rs}`、`rust/app/Cargo.toml`（`serde`、`serde_json`）

**Interfaces:**
- Consumes: `ChoiceBook`、`new_with_choices`。
- Produces:
  - `choices_path(config: &Path) -> PathBuf`（同資料夾的 `lyric-choices.json`）
  - `ChoiceFile::open(path: PathBuf) -> (ChoiceFile, ChoiceBook, Vec<String>)`
  - `ChoiceFile::save(&self, book: &ChoiceBook) -> io::Result<()>`
- 格式：`{"version":1,"songs":[{"song":"<cache_key>","duration":209.9|null,"url":"https://…","offsets":{"https://…":0.5},"lines":[[8.43,"…"]]}]}`
- 讀不了、不是合法 JSON、版本不是 1 → 空紀錄加一條警告（不含歌詞或歌名），第一次存檔前把原檔改名 `.bak`；備份失敗就不寫。單筆壞掉（非 https 網址、沒有行、非有限數字）略過那筆；偏移讀入時 `clamp_offset`。
- `config.rs` 的 `write_temp`、`sibling_with_suffix` 改 `pub(crate)` 重用。

- [ ] **Step 1: 寫失敗測試**：`choices_live_next_to_the_config_file`、`a_saved_book_loads_back_unchanged`、`a_missing_file_is_an_empty_book_without_warnings`、`a_broken_file_is_kept_as_bak_on_the_first_save_only`、`bad_entries_are_skipped_and_offsets_clamped`、`a_failed_save_leaves_the_old_file`（路徑上層改成不可建立）。
- [ ] **Step 2:** 執行確認失敗。
- [ ] **Step 3:** 實作；`main.rs` 在建立控制器前開檔、印警告，`save` 失敗只寫日誌。
- [ ] **Step 4:** 全部檢查通過。
- [ ] **Step 5:** `git commit -m "feat(app): persist lyric choices"`

---

### Task 7: 展開畫面與文件

**Files:**
- Modify: `rust/shell/src/{music_layout.rs, music_views.rs}`、`rust/shell/examples/music_demo.rs`、`rust/docs/HANDOFF-rust.md`、`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`

**Interfaces:**
- Consumes: `MusicState.version` 與三個動作。
- Produces（`music_layout`）：`OFFSET_STEP = 0.5`、`OFFSET_BIG_STEP = 5.0`、`OFFSET_BUTTON_WIDTH = 18.0`、`OFFSET_VALUE_WIDTH = 40.0`、`version_label(index: usize, count: usize) -> String`、`offset_label(offset: f64) -> String`、`offset_step(option_held: bool) -> f64`

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn version_label_counts_only_when_there_is_a_choice() {
    assert_eq!(version_label(0, 3), "時間軸版 1/3 ›");
    assert_eq!(version_label(2, 3), "時間軸版 3/3 ›");
    assert_eq!(version_label(0, 1), "時間軸版");
}
#[test] fn offset_label_is_signed() {
    assert_eq!(offset_label(0.0), "0.0s");
    assert_eq!(offset_label(-0.0), "0.0s");
    assert_eq!(offset_label(0.5), "+0.5s");
    assert_eq!(offset_label(-1.0), "\u{2212}1.0s");
}
#[test] fn option_click_takes_the_big_step() {
    assert_eq!((offset_step(false), offset_step(true)), (0.5, 5.0));
}
#[test] fn offset_controls_fit_beside_the_fold_button() {
    // 瀏海 185 與 200：資訊欄寬 = notch + WIDTH_PADDING − 2×內距 − 封面 − 間距
    // control_side_widths(info).1 − CONTROL_ROW_HEIGHT >= 2×OFFSET_BUTTON_WIDTH + OFFSET_VALUE_WIDTH
}
```

- [ ] **Step 2:** 執行確認失敗。
- [ ] **Step 3:** 畫面：`controls()` 左側上行在有 `version` 時顯示 `version_label`，`count >= 2` 時可點（id `music-version-next`，手形游標）；右側彈性區放 `−`（`nudge_offset(-step)`）、數值（`reset_offset`）、`＋`（`nudge_offset(step)`），`step = offset_step(event.modifiers().alt)`，字級沿用 `SOURCE_LABEL_FONT_SIZE`。按鈕群位置不變。切版本後展開歌詞直接跳到目前行（`lyric_scroll.jump`）。`music_demo` 加 `versions` 場景：兩個相差 5 秒的版本。
- [ ] **Step 4:** 全部檢查通過；停掉執行中的 App 後跑 `--scene versions`，展開畫面截圖確認三個控制項不重疊、不裁切（截不到展開畫面就在回報裡寫明沒有目視確認）。
- [ ] **Step 5:** 文件：HANDOFF 新增第 14 節（實機清單照設計第 7 節）與目錄；母 spec 第 5 節的歌詞來源／快取兩條補上同時查詢、版本清單、120 秒快取，第 8 節「時間軸微調」註明已提前實作。
- [ ] **Step 6:** `git commit -m "feat(shell): version switch and offset controls in the expanded view"`
