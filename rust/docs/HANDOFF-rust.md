# MusiCat Rust 版：給作者的驗收文件

最後更新：2026-11-26（歌詞版本切換與時間微調）。這份文件是 Rust 版 R2 的**唯一**驗收文件：自動化做不到、必須由作者在自己的 Mac 上確認的項目都在這裡。
確認後把結果寫回 spec（`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`）第 7.2 節的表格與第 12 節，
並在這裡把該項打勾、附上日期與觀察。各任務的詳細步驟在文末的附錄 A–E（每個附錄對應一個任務，內容是當時寫下的待辦，沒有刪減）；
前面的章節是索引與總表，不重複附錄的步驟。

## 目錄

1. [怎麼執行](#1-怎麼執行)
2. [Rust 版驗收清單（對應 Swift HANDOFF 的 20 項）](#2-rust-版驗收清單對應-swift-handoff-的-20-項)
3. [spec 第 12 節的額外檢查](#3-spec-第-12-節的額外檢查)
4. [Task 21–25 新增的檢查（索引）](#4-task-2125-新增的檢查索引)
5. [待重跑：Task 23 的 hover 驗證](#5-待重跑task-23-的-hover-驗證)
6. [量測：CPU 與記憶體（measure.sh）](#6-量測cpu-與記憶體measuresh)
7. [移除 Swift 版的關卡與待決事項](#7-移除-swift-版的關卡與待決事項)
8. [歌詞來源連結](#8-歌詞來源連結)
9. [歌詞條行數（1～3 行）](#9-歌詞條行數13-行)
10. [歌詞轉繁體中文](#10-歌詞轉繁體中文)
11. [換螢幕與投影機](#11-換螢幕與投影機)
12. [歌詞命中狀態](#12-歌詞命中狀態)
13. [官方 MV 標題](#13-官方-mv-標題)
14. [歌詞版本切換與時間微調](#14-歌詞版本切換與時間微調)
15. 附錄 A–E：Task 21 視窗、Task 22 選單列、Task 23 外殼、Task 24 音樂畫面、Task 25 真正的 App

## 1. 怎麼執行

```sh
sh rust/scripts/run.sh          # 從任何目錄都行；等同 Swift 版的 scripts/run.sh
```

`run.sh` 先 `cargo build --release -p musicat-app`（一律 release：spike 的 GPUI 透明度以 release 驗證）再 `exec target/release/musicat "$@"`。
建置失敗且訊息提到 Metal 時會說明本專案用 `runtime_shaders` 建置（不需要 Metal Toolchain，見 `rust/README.md`）；
建置後找不到 helper 動態庫時會印 `cargo clean -p musicat-platform-macos` 的提示。第一次建置要編 GPUI，需要好幾分鐘。

開發與驗收用的環境變數（日誌都寫到 stderr）：

| 變數 | 作用 |
|---|---|
| `MUSICAT_LOG=1` | 印出現正播放（**含曲名與演出者**）、歌詞狀態、外殼狀態與選單列狀態。 |
| `MUSICAT_AUTO_QUIT_SECS=<n>` | n 秒後走與選單「結束」相同的路徑結束。 |
| `MUSICAT_CONFIG=<path>` | 改用這個設定檔，避免動到真正的設定檔（`~/Library/Application Support/MusiCat/musicat.toml`）。 |

啟動時一定會在 stderr 印出實際使用的設定檔路徑。驗收時建議：`MUSICAT_LOG=1 sh rust/scripts/run.sh`。
動態庫（helper）放在可執行檔旁邊（`rust/target/release/libmusicat_mediaremote_helper.dylib`），找不到時只剩視窗標題備援。
煙霧／示範範例（`panel_smoke`、`tray_smoke`、`shell_demo`、`music_demo`、`wake_smoke`）的用法在各附錄。

## 2. Rust 版驗收清單（對應 Swift HANDOFF 的 20 項）

這 20 項照抄 `docs/HANDOFF.md`「尚未驗證」，**項目內容與判定標準不變**，只改名稱與指令
（`pkill -f NotchCatMediaRemoteHelper` → `pkill -f musicat_helper_main`；`scripts/run.sh` → `rust/scripts/run.sh`；
`swift build` → `cargo build`；`.scrollIndicators(.hidden)` → GPUI 歌詞捲動區；AppBar／版面預算 → Rust 常數）。
Rust helper 的實際命令列（`ps` 實測，Task 26，App 執行中）是
`/usr/bin/perl -e <含 "musicat_helper_main" 的腳本> <…/target/release/libmusicat_mediaremote_helper.dylib>`，父行程是 `musicat`，
所以 `pkill -f musicat_helper_main` 與 `pgrep -f musicat_helper_main` 都能對到它。

狀態欄：**agent-verified** = 代理人已實測、證據寫在旁邊；**partly** = 一部分已驗證（寫明哪一部分）、其餘仍待作者；
**needs author** = 代理人沒有驗證。證據只引用各任務與附錄實際做過的事（任務報告不在 repo 內，這裡寫的就是證據紀錄）。最右欄的 ☐ 由作者驗收後打勾。

| # | 項目（Swift 原文；僅改名稱與指令） | 狀態 | 證據與作者要做的事 | 作者 |
|---|---|---|---|---|
| 1 | YouTube Music、YouTube、Spotify 各播一首：Preview 出現封面、音柱與歌詞條；Expanded 顯示曲名、演出者、進度條、封面。**Spotify 完全未驗證**（開發機沒安裝）。 | partly | 已驗證：`music_demo --scene playing`（腳本來源）的 Preview 與 Expanded 版面，以 `screencapture` 只截瀏海區域目視確認（附錄 D 開頭）；真實 helper 在 App 內送來現正播放（暫停中的曲目，來源 `mediaremote`，位置與封面都有；附錄 E 開頭）。**作者要做**：三個播放器各播一首看真實畫面（尤其 Spotify）。步驟見附錄 D「Preview 版面」「Expanded 版面」。 | ☐ |
| 2 | 暫停時 helper 輸出的形狀（Preview 是否保留封面與靜止音柱）。 | partly | 已驗證：真實 helper 對一首已暫停的曲目送出暫停快照（日誌 `paused … [mediaremote; position known; artwork=true]`）。2026-11-25 起暫停時音樂仍是 active 模組（先前是空黑條，附錄 E 的截圖是舊行為）。**作者要做**：播放中按暫停，歌詞條收起、封面與靜止音柱留著；展開直接是音樂頁（附錄 D「暫停保留封面與音柱」）。 | ☐ |
| 3 | 拖動進度時 helper 是否會輸出新的一行（畫面是否跟上）。 | needs author | 要真實來源：在播放器拖動進度後，Expanded 的進度條與目前歌詞行在 1 秒內跟上（附錄 D「拖動進度」）。 | ☐ |
| 4 | 視窗標題備援的格式：YouTube Music PWA 以外的情境，以及備援是否需要「螢幕錄製」權限（很可能需要，見 `docs/HANDOFF.md` 的實測發現）。 | needs author | 標題解析有單元測試（Swift 案例逐案移植），但沒有對真實播放器驗證過。`perl_host = false` 時只用視窗標題備援（Task 25b 驗證了沒有 helper 行程，沒有看標題內容）。 | ☐ |
| 5 | hover 不閃爍：Preview 加高後重新驗收（這是 Shell 先前最難的問題）；特別留意歌詞條出現或折疊使 Preview 高度改變時，游標靜止不動是否會產生多餘的展開（Swift 版以 `mouseEntered` 判斷；Rust 版輪詢游標位置，沒有 tracking area）。**差異請一併判斷**：Rust 版在游標靜止、Preview 因歌詞條出現而長高、游標剛好落進新範圍時，會在下一次取樣展開（Swift 沒收到 enter 就不展開）。 | partly | 已驗證：Task 23 以腳本驅動真實游標的 13 步 hover 情境全部符合期望（不在 Preview 下方的容器內不展開、抖動不收合、離開 150 ms 後收合、快速進出只展開一次、跨長條下緣抖動只展開一次），動畫 284／285 ms；但那是修正輪**之前**、佔位內容的結果，修正輪後尚未重跑（見第 5 節）。**作者要做**：第 5 節的重跑，再用真實觸控板看歌詞條出現／折疊時游標靜止的行為（附錄 C「游標靜止在長條邊緣不閃爍」）。 | ☐ |
| 6 | GPUI 歌詞捲動區不顯示捲軸（Expanded 的歌詞區與 `--scene plain` 的靜態全文區）。 | needs author | 附錄 D 的說明是 GPUI 不畫捲軸，但 Task 24 沒有目視確認。與第 16 項一起檢查（附錄 D「捲動條」）。 | ☐ |
| 7 | 歌詞條底色透明度 0.6、高度 24pt 的觀感（`LYRIC_STRIP_OPACITY`、`LYRIC_STRIP_HEIGHT`，在 `core/src/music/module.rs`）；展開／折疊動畫與折疊後的 hover 範圍。 | partly | 已驗證：`music_demo --scene playing` Preview 瀏海下方的 0.6 黑色歌詞條、圓角與中文歌詞行（Task 24 截圖）。**未驗證**：折疊箭頭的點擊、展開／折疊動畫手感、歌詞條出現時的半透明過渡（附錄 C「Preview 歌詞條的黑底與半透明」、附錄 D「歌詞條觀感與折疊」）。調整結果寫回 spec 與兩個常數。 | ☐ |
| 8 | 上一首／播放暫停／下一首（尤其 `next`、`previous`）真的控制得動。 | needs author | 示範程式的按鈕作用在腳本來源，且點擊在非啟用面板上的行為沒有驗證過；真實播放器要在 App 內驗（附錄 D「按鈕可點、不搶焦點」、附錄 E「真實播放器的控制與 CPU」）。 | ☐ |
| 9 | 殺掉 perl 行程（`pkill -f musicat_helper_main`）後自動恢復；暫時改名動態庫（`target/release/libmusicat_mediaremote_helper.dylib`）後 App 不崩潰並退到視窗標題備援（已知會在 stderr 印一行 `system now-playing unavailable`，只印一次）。 | needs author | 監管者的重啟與退避有單元測試（假的啟動器）；沒有對真實 helper 殺過一次。附錄 E「helper 被殺掉後恢復」。 | ☐ |
| 10 | 無網路時歌詞靜默消失、曲目顯示不受影響。 | needs author | 提供者「失敗」與「乾淨地沒有」的區分有單元測試；真實斷網沒有試過（附錄 E「LRCLIB／網易雲開關與離線」）。 | ☐ |
| 11 | 視窗標題備援只顯示曲名（沒有演出者，不查歌詞）。 | needs author | 單元測試涵蓋；真實情境見附錄 E「視窗標題備援」。 | ☐ |
| 12 | helper 的 CPU 與記憶體在長時間執行下的表現。 | partly | 已驗證（短時間）：App 閒置 15 秒約 1.3% CPU、RSS 約 42 MB；helper 約 0.3%、13 MB（Task 25b）。`measure.sh -d 40 -i 5` 對真實 App 與 helper 跑過，RSS 變化 +1.4%／+2.2%（Task 26）。**作者要做**：第 6 節的 1 小時量測。 | ☐ |
| 13 | 音柱（Equalizer）動畫的 CPU 成本。 | partly | 已量測：release、停在 Preview 的示範程式，音柱 12 fps 約 3.6%（目標約 3%，略高）；跑馬燈 30 fps 約 5.4%（Task 24）。**作者要做**：目視平順度、決定是否把 `music_layout::EQ_FRAME_SECONDS` 調成 1/10；在真實播放時再量一次（附錄 D「CPU 與平順度的取捨」）。 | ☐ |
| 14 | Expanded 的模組切換列（AppBar）實際高度為 15pt（`music_layout::APP_BAR_HEIGHT`），版面預算是 149pt（`MUSIC_PAGE_HEIGHT` = `EXPANDED_CONTENT_HEIGHT` 172 − 15 − 8）而非 144pt，請目視確認沒有擠壓或裁切。常數都在 `rust/shell/src/music_layout.rs`；Expanded 高 180（`EXPANDED_HEIGHT`，`rust/core/src/shell_logic/geometry.rs`）、上方內距 8（`EXPANDED_TOP_PADDING`，`rust/shell/src/lib.rs`）。 | partly | 已驗證：預算由常數推導並有單元測試（內容角點在 24 pt 圓角內）；Task 24 的 Expanded 截圖沒有看到擠壓，下方圓角外沒有露出內容。**作者要做**：用真實內容再看一次（附錄 D「版面預算」）。 | ☐ |
| 15 | 歌詞條額外高度的 0.28 秒動畫期間，Preview 的封面是從零尺寸長出來的，請確認觀感可接受。（Rust 版的實作不同：內容槽固定為目標大小、由外形裁切，封面**不會**從零長出來，歌詞條是被「揭開」。請確認新的觀感可接受。） | needs author | 附錄 D「歌詞條出現的動畫」。 | ☐ |
| 16 | GPUI 歌詞捲動區不顯示捲軸，包含「系統設定 > 外觀 > 顯示捲動條：永遠」的情況下。 | needs author | 與第 6 項一起檢查（附錄 D「捲動條」）。 | ☐ |
| 17 | 睡眠／喚醒：播放音樂時讓 Mac 睡眠再喚醒，helper 是否恢復、資料是否是新的。 | partly | 已驗證：`wake_smoke` 以程式發出 `NSWorkspaceDidWakeNotification`（**沒有**讓機器睡眠），回呼在主執行緒被呼叫、取消登記後不再被呼叫（Task 25b）。**作者要做**：真的睡眠再喚醒（附錄 E「睡眠與喚醒」，日誌 `system woke from sleep`、helper 換成新 pid）。 | ☐ |
| 18 | 從選單列 Quit，以及對 App 下 `kill -9`，之後 `pgrep -f musicat_helper_main`（或 `pgrep perl`）都不應留下 helper 行程。 | partly | `kill -9` App：0.05 秒內 helper 消失（Task 25b）；`MUSICAT_AUTO_QUIT_SECS`（與選單「結束」同一條程式路徑）後 `exit=0`、沒有 helper（Task 25b）；Task 26 另驗證 `SIGTERM`、`SIGINT`、系統式結束（`NSRunningApplication.terminate()`，同登出的 Apple Event）都不留 helper。未做：用滑鼠真的點選單「結束」（附錄 E「結束不留 helper」，路徑相同、風險低）。 | ☐ |
| 19 | App 執行中重新 `cargo build --release -p musicat-app`（動態庫被重新連結）後，確認 helper 能恢復。**注意**：cargo 只在 `rust/platform-macos/helper/` 或 `build.rs` 有變更時才重跑 build script、重新連結動態庫，所以要先 `touch rust/platform-macos/helper/helper.c` 才會真的換掉動態庫（build.rs 先刪再複製，新檔案是新的 inode）。只手動刪掉 `target/release/` 裡的那份複本不會被補回，要 `cargo clean -p musicat-platform-macos`（`run.sh` 會提示）。 | needs author | 附錄 E「重新建置後恢復」。 | ☐ |
| 20 | 視窗標題備援在有／沒有「螢幕錄製」權限下的行為（沒有權限時預期什麼都不顯示）。 | needs author | 附錄 E「視窗標題備援」。 | ☐ |

驗收後把歌詞條透明度與高度的調整結果寫回 Music spec 與計畫的 Global Constraints（與 Swift 版相同的要求）。

## 3. spec 第 12 節的額外檢查

spec 第 12 節除了上面的 20 項（第 1 點），還有兩項 Rust 版必須實機確認（第 4 點 Windows 屬於 R3，不在這一輪）：

- **第 2 點：GPUI spike 沒能自動驗證的三項**——點擊穿透、跨空間與全螢幕空間、點擊不搶焦點。
  Task 21 起都做成煙霧範例的待辦（附錄 A：「點擊穿透（面板外）」「點擊長條不搶焦點、不啟動 App」「跨空間」「其他 App 的全螢幕空間」）；
  Task 23 的外殼上再驗收一次（附錄 C：「收合時點擊穿透」「展開時接收點擊、不搶焦點」）。狀態：**needs author**。
- **第 3 點：全螢幕時面板在 macOS 上的實際行為**（spec 第 5.3 節）。附錄 A「其他 App 的全螢幕空間」、附錄 C「全螢幕 App 之上的行為」。
  在作者回報之前，`foreground_is_fullscreen()`（`rust/platform-macos/src/panel.rs`）**固定回 `false`，不要動它**：
  若面板在全螢幕空間裡被系統自然隱藏就維持 `false` 並把實測結果寫進它的文件註解；若仍顯示在全螢幕 App 之上，
  才照註解裡的計畫另開任務實作。狀態：**needs author**。

## 4. Task 21–25 新增的檢查（索引）

下列檢查是 Task 21–25 加進來的（Swift 版沒有，或 Swift 版的 20 項沒有涵蓋）。已去重：與第 2 節某一項相同的只標項目編號，不再重複描述；
步驟都在對應附錄。

**面板與視窗行為**（附錄 A、C）
- 目測長條與瀏海對齊（容器 x 583.5 落在 583，可能偏 0.5 點；aux 區高 32 對選單列視窗高 37）——附錄 A。
- 點擊穿透（面板外、收合時的透明區域）、點擊／展開後點擊不搶焦點、跨空間（Stationary）、其他 App 的全螢幕空間——附錄 A、C；第 3 節。
- 隱藏／再顯示（`MUSICAT_DEMO_BLINK=1`）、內容裁切（GPUI 0.2.2 只能矩形裁切，下方圓角外不得露出內容；第 14 項）——附錄 C。
- 動畫手感（0.28 秒、圓角 10→24、中途反向不跳）、觸控板游標靜止在長條邊緣不閃爍（第 5 項）——附錄 C。

**選單列圖示**（附錄 B、E）
- 圖示看得見、淺色／深色外觀下可讀、多螢幕時出現在哪個螢幕——附錄 B。
- **圖示被藏起來**：瀏海本身會擋住右側圖示，選單列太擠時 macOS 也會把圖示移到畫面外；用 ⌘ 拖曳其他圖示騰出位置（開發機上 21 個圖示有 17 個被移走，包括 MusiCat）——附錄 B、E。
- 點圖示開選單、項目順序與勾選、點選後勾選翻轉、選單開著時面板的 hover 輪詢照常、結束可用、沒有 Dock 圖示且不搶焦點、圖示與面板重疊時誰蓋住誰——附錄 B（滑鼠操作的版本在附錄 E「選單用滑鼠操作」）。

**畫面與互動**（附錄 D）
- 按鈕／折疊箭頭／模組切換列的點擊（非啟用面板上；第 8 項）、字形（路徑繪製的控制鍵與 ♪）、跑馬燈平順度、中文字——附錄 D。
- 場景：`paused`（第 2 項）、`none`、`plain`、`nolyrics`、`longtitle`；封面置中裁切與圓角；模組切換列選取每次新展開重設——附錄 D。
- 音柱與跑馬燈的 CPU 取捨（第 13 項）——附錄 D。

**多螢幕、設定與來源**（附錄 E）
- 換螢幕（外接螢幕選擇、拔掉、接回、闔上筆電、「自動」）、設定檔只在值改變時寫入、歌詞條折疊的記憶（含 2 秒內被 `kill -9` 會遺失）——附錄 E。
- 從選單切換 perl 宿主（含啟動時關閉後第一次勾選才建立 helper；第 9 項）、歌詞來源開關只影響之後的查詢（第 10 項）、真實播放器的控制與 CPU——附錄 E。

**建置**（附錄 A）
- 作者的 Mac 若有 Metal Toolchain，拿掉 `runtime_shaders` 試一次預先編譯 shader 的路徑——附錄 A 最後一項。

## 5. 待重跑：Task 23 的 hover 驗證

Task 23 的修正輪（`247c40b`：歌詞條半透明區域、隱藏時 Animator 閒置、drop 即關閉）之後，**代理人沒能重跑** hover 腳本與動畫量測：
當時作者的 Mac 鎖定且螢幕睡眠（`CGDisplayIsAsleep=1`、`CGSSessionScreenIsLocked=1`，`screencapture` 失敗），GPUI 在那個狀態不畫東西，
動畫無法量測、歌詞條看不到。Task 26 也沒有重跑（hover 腳本要移動作者的真實游標約 20 秒），一併留給作者。
`HoverMachine`、輸入接線與到期喚醒在修正輪沒有改；改的是繪製與關閉行為，所以需要確認的是：

1. 一次 `shell_demo` ＋ `hover 756 32`，確認 13 步情境的期望仍全部成立（`756` 是 1512 點寬螢幕的瀏海中心 x，`32` 是瀏海高；別的螢幕請換成自己的值）。
   執行期間**不要碰滑鼠**（腳本會把游標移動約 20 秒再還原）：

   ```sh
   cd rust
   MUSICAT_DEMO_SECS=90 cargo run --release -p musicat-shell --example shell_demo > /tmp/shell_demo.log &
   sleep 3 && ../spikes/gpui-notch/tools/hover 756 32      # 每一步印 STEP；log 裡每次轉移印 expand／collapse
   # 對照 /tmp/shell_demo.log 與 STEP 的時間戳：每個 STEP 的名稱就寫著期望（expect NO expand、expect 1 exit …）；摘要見附錄 C 開頭
   ```

2. 一次動畫量測：`MUSICAT_SHELL_TRACE=1`，量展開與收合各約 0.28 秒（上次 284／285 ms）：

   ```sh
   cd rust
   MUSICAT_SHELL_TRACE=1 MUSICAT_DEMO_SECS=30 cargo run --release -p musicat-shell --example shell_demo > /tmp/shell_anim.log &
   sleep 3 && ../spikes/gpui-notch/tools/anim 756 32       # 進入長條、停 1.2 秒、離開
   grep shell-frame /tmp/shell_anim.log | head -80         # 高度 32→180、圓角 10→24，約 0.28 秒
   ```

3. 用眼睛看歌詞條切換：`shell_demo` 每 6 秒切換一次 24 點高的佔位歌詞條（`MUSICAT_DEMO_TOGGLE_SECS`、`MUSICAT_DEMO_EXTRA` 可改）。
   出現時瀏海高度那一段仍是純黑、下方兩角從圓角平順變成直角、下面長出 0.6 黑色帶；消失時反過來，黑底高度不跳（附錄 C「Preview 歌詞條的黑底與半透明」）。

（`hover`、`anim` 是 `spikes/gpui-notch/tools/` 裡已編好的小工具；沒有的話 `swiftc -O hover.swift -o hover`、`swiftc -O anim.swift -o anim`。）

## 6. 量測：CPU 與記憶體（measure.sh）

驗收清單第 12、13 項與 spec 第 10 節「記憶體與 CPU」。**播放音樂時**量 1 小時：

```sh
sh rust/scripts/run.sh                                   # 終端機 1（保持執行，放音樂）
sh rust/scripts/measure.sh -d 3600 -i 10 -o ~/musicat-measure.csv     # 終端機 2
```

`measure.sh [-d 秒數=3600] [-i 秒數=10] [-o 檔案=musicat-measure.csv] [-p PID]`：以 `pgrep -x musicat`（或 `-p`）找到 App，
每個間隔用 `ps -o %cpu=,rss= -p` 記一行 CSV：`elapsed_s, musicat_cpu_pct, musicat_rss_kb, helper_cpu_pct, helper_rss_kb`
（helper = App 的子行程中命令列含 `musicat_helper_main` 的 perl；沒有時兩欄留空）。App 結束時腳本跟著結束並印摘要。
只寫 `-o` 指定的檔案（預設在目前目錄），不連網。

**成功判準**
- RSS：App 與 helper 各自，**最後十分之一樣本的平均相對於最前十分之一樣本平均的成長 ≤ 10%**（腳本印 `PASS`／`FAIL`；結束碼 1 = FAIL）。
  1 小時內沒有持續上升的趨勢。注意 helper 的 RSS 前幾分鐘會爬升再進入平穩（Swift 實測在約 416 個來源處，MediaRemote 內部約 400 秒的逾時）；
  若 helper 因此 `FAIL`，看摘要裡「參考，不計入判定」那一行（第 2 個十分之一對最後十分之一）與 CSV 曲線：爬升後平穩就是正常，持續上升才是問題。
- CPU：腳本只印平均與最大值，由你判斷：閒置時要與 spike 的 1.4% 同一個量級。參考值：App 閒置 1.3%（Task 25b，15 秒）、
  播放中音柱約 3.6%（Task 24）；helper 閒置約 0.3%。helper 的 CPU 與 RSS 也請一起看。
- 把摘要貼回 spec 第 12 節；CSV 留著。

## 7. 移除 Swift 版的關卡與待決事項

**關卡：你完成這份清單之後，再告訴代理人「移除 Swift 版」，那會是一個獨立的提交；在那之前什麼都不會被移除**
（Swift 原始碼、`Package.swift`、`scripts/run.sh`、`docs/HANDOFF.md` 都原封不動；代理人不會自己刪）。
清單中有 **needs author** 或 **partly** 而你判斷可以接受的項目，請在移除前一併註明。

待決事項（需要你決定，不影響這一輪的驗收）：
- `foreground_is_fullscreen()` 要不要實作（取決於第 3 節的全螢幕實測）。
- 歌詞條透明度 0.6、高度 24、音柱 12 fps、跑馬燈 30 fps 要不要調（第 7、13 項）。
- 有 Metal Toolchain 時要不要拿掉 `runtime_shaders`（附錄 A 最後一項）。
- 與 Swift 版 `docs/HANDOFF.md`「待決事項」相同的兩項：v1 要不要做「模組開關」的使用者可見設定 UI（目前寫死啟用清單）、多模組搶 Preview 的排序體驗。
- Windows 全部（R3，未在實機驗證）。

**不在這一輪**：`docs/HANDOFF.md` 的「下一輪設計的輸入」（本地歌詞來源排第一、可編輯覆寫、±offset 與黑名單、行級準確度、準確度優先、
本地歌詞資料夾給 AI agent 寫、公開 GitHub repo 分享、git 版控、版權）是 **R5**，R2 不做。
Swift 的「已知風險」（perl 宿主脆弱、僅供個人使用、不可對外發布；歌詞命中率樣本太少等）在 Rust 版同樣成立，見 `rust/README.md`「macOS 風險」。

## 8. 歌詞來源連結

有歌詞時，Expanded 控制列左側的空位顯示一個小而低對比的來源標籤（網址的主機名稱加 `↗`，例如 `lrclib.net ↗`、`music.163.com ↗`），
點擊後用系統預設瀏覽器開啟該歌詞的來源網址（只接受 `https://`，`platform-macos` 的 `links::open_url` 再驗證一次）。
標籤文字由 `music_layout::source_label` 產生（有單元測試）；字級、不透明度與最大字元數是同一檔案的 `SOURCE_LABEL_*` 常數。
示範程式（`music_demo`）不開瀏覽器，只印出 `open <網址>`。以下請用真正的 App（`cargo run --release -p musicat-app`）
與示範程式（`cargo run --release -p musicat-shell --example music_demo -- --scene <場景>`）確認：

**標籤寬度。** 標籤的空位是控制列左側，預設瀏海寬 185 點時是 110.5 點（`(445 − 32 − 68 − 12 − 112) / 2`：
外形寬 445、左右內距 32、封面 68、間距 12、三顆按鈕加間距 112；見 `music_layout::control_side_widths`）。
2026-11-26 加寬之前只有 60.5 點、網易雲的標籤會被截斷；現在字級 9 點時 `lrclib.net ↗` 與 `music.163.com ↗` 都預期**完整顯示**。

- [ ] **(a) 有命中時標籤顯示、點擊開瀏覽器、面板不搶焦點。** 播放一首查得到歌詞的歌，展開面板 → 控制列左側出現來源標籤，
  游標移上去是手形；點一下 → 預設瀏覽器開啟該網址，面板不搶走目前 App 的焦點（點擊前的前景 App 仍是前景，除了瀏覽器本身被系統帶到前面）。
- [ ] **(b) 沒命中時沒有標籤、版面不變。** `music_demo --scene none`、`--scene nolyrics`（以及真實 App 裡查不到歌詞的歌）→ 沒有標籤，
  控制列的高度、三顆按鈕與右側折疊箭頭的位置與有標籤時完全相同。
- [ ] **(c) 純文字歌詞也有標籤。** `music_demo --scene plain`（以及真實 App 裡只有純文字歌詞的歌）→ 標籤照樣出現、可點。
- [ ] **(d) 標籤不蓋到按鈕。** 超長歌名（`--scene longtitle`）或窄螢幕（瀏海較窄的機型）→ 標籤放不下時主機名稱以 `…` 結尾
  （`↗` 保留），不會與「上一首」按鈕重疊，也不會把按鈕擠離中央。預設瀏海寬度下 `lrclib.net ↗` 與 `music.163.com ↗` 都應完整。
- [ ] **(e) LRCLIB 沒有有效 id 時開的是請求位址。** LRCLIB 的回應沒有可用的 `id` 時，連結退用當時的請求位址（`/api/get?...` 或 `/api/search?...`），
  瀏覽器會顯示 JSON 或搜尋結果清單——這是預期行為，不是錯誤。

---

## 9. 歌詞條行數（1～3 行）

選單「歌詞條顯示 1／2／3 行」（互斥勾選，寫回設定檔的 `lyric_strip_lines`）決定瀏海下方歌詞條一次顯示幾行：
1 行 = 目前行；2 行 = 目前行＋下一行；3 行 = 前一行＋目前行＋下一行（目前行在中間、最亮、太長會跑馬燈；前後文較暗、只截斷）。
歌詞條高度依行數為 24／40／56 點（`core/src/music/module.rs` 的 `lyric_strip_height`）；沒有同步歌詞時只顯示歌詞命中狀態（單列、垂直置中，見第 12 節）；
第一行之前的「前一行」與最後一行之後的「下一行」是空列，目前行不會跳動。
外殼會把 Preview 額外高度夾到 `MAX_PREVIEW_EXTRA_HEIGHT`（原為 28，為了三行放寬到 56；`every_line_count_reaches_the_shell_unclipped` 擋回歸）。
3 行在外接螢幕的虛擬瀏海上已用截圖目視確認完整顯示（2026-11-25）。選單行為與列的內容都有單元測試；以下視覺與動畫沒有驗證，請用真正的 App 確認：

換行時歌詞在 0.3 秒內往上捲一行、亮度跟著過渡（`music_layout::StripScroll`；一次跳超過 2 行、換歌、改行數、Preview 重新出現時直接換）。
3 行的換行中途畫面已用連續截圖確認（2026-11-25）；1、2 行與手感沒有驗證。

- [ ] **(a) 切換行數立即生效。** 播放一首有同步歌詞的歌，從選單切 1 → 2 → 3 → 1：歌詞條高度跟著變、內容立即換成對應的列，沒有殘影，選單的勾跟著移動；
  再點一次已勾選的行數沒有任何變化。
- [ ] **(b) 高度變化時的動畫與 hover。** 歌詞條高度改變（24 → 56）時，外形有平順的過渡、游標停在原地不會多餘展開或收合（同第 5 項）。
- [ ] **(c) 3 行的觀感。** 三行擠在 56 點高的黑條裡，上下留白、下方圓角是否好看；前後文 0.45 的不透明度是否合適（`music_layout::STRIP_CONTEXT_OPACITY`）。
- [ ] **(d) 重啟後記得。** 選 3 行後結束再開，仍是 3 行。
- [ ] **(e) 換行動畫手感。** 1、2、3 行各看幾次換行：捲動順不順、0.3 秒會不會太慢或太快；拖進度條時直接換、不亂捲；
  很長的歌詞行（跑馬燈）換行時有沒有突兀的跳動。

---

## 10. 歌詞轉繁體中文

選單「歌詞轉繁體中文」（設定檔 `traditional_lyrics`，預設關）把簡體中文歌詞轉成台灣繁體字形顯示（OpenCC `s2tw`，只換字、不換用語），
立即套用、不重查；只轉歌詞，歌名與歌手照播放器給的顯示。整份歌詞判斷（`core/src/lyrics/traditional.rs`，有單元測試）：
日文（假名佔十分之一以上）不轉；本來就是繁體（繁體專用字多於簡體寫法的字）不轉，因為再轉一次會壞（干擾 → 幹擾）。

已知限制：一簡對多繁靠 OpenCC 詞庫，詞庫沒收的詞可能選錯（例：「头发干了」→「頭髮幹了」）；簡繁各半的歌詞整份不轉；
轉換是對整份判斷，中文歌裡的日文段落若假名太少，會一起被轉。

- [ ] **(a) 開關立即生效。** 播放簡體歌詞的歌，勾選後歌詞條與 Expanded 立即變繁體，取消後立即變回簡體；重啟後記得。
- [ ] **(b) 日文歌與繁體歌不受影響。** 開著開關播一首日文歌、一首繁體歌詞的歌，字形和關掉時相同。
- [ ] **(c) 轉換品質。** 聽幾首歌，有沒有明顯選錯的字（尤其 干／乾／幹、里／裡、发／髮、只／隻）。常見錯字可以再加例外表。

---

## 11. 換螢幕與投影機

2026-11-25 修正：換到別的螢幕或接投影機後跑版。原因是 GPUI 0.2.2 在原生視窗回呼（`setFrameSize:`、
`viewDidChangeBackingProperties`）裡以 `try_borrow_mut` 借用 `App`，而外殼在 `cx.update` 之中改視窗外框，
回呼同步發生、借不到、靜默丟掉，GPUI 就一直用舊的視窗大小與縮放比例排版。現在改外框與顯示／隱藏都在 `update` 之外套用
（`shell/src/window.rs` 的模組文件）。**升級 GPUI 時重跑下面的檢查。**

自動檢查（需要視窗伺服器，不進 CI；接著越多不同的螢幕測得越完整，面板會在各螢幕頂端閃現約半分鐘）：

```sh
cargo run --release -p musicat-shell --example move_check
```

單螢幕（內建 Retina）上自動檢查已通過；2026-11-25 作者在實機回報跑版已修好。多螢幕下的 `move_check` 還沒有紀錄。

- [ ] **(a) 接著外接螢幕跑 `move_check`**，全部 `ok`。
- [ ] **(b) 選單切換螢幕。** 在內建與外接螢幕之間來回選，Preview、歌詞條、Expanded 的大小與位置正常。
- [ ] **(c) 插拔。** App 開著時接上、拔掉螢幕或投影機（含鏡像輸出與闔上筆電），面板每次都回到某個螢幕的頂端中央、不跑版。

---

## 12. 歌詞命中狀態

四種狀態：「查詢中…」「沒有歌詞」「純文字版」「時間軸版」。歌詞條沒有歌詞行可顯示時只顯示狀態（不重複左翼的曲名；歌詞在捲動時不顯示）；
Expanded 控制列左側上行是狀態、下行是來源連結（沒命中時只有狀態）。時間軸版有兩個以上版本時，Expanded 的上行是
「時間軸版 1/3 ›」（見第 14 節）；歌詞條仍只顯示「時間軸版」。

已知限制：沒有演出者的曲目不查詢，顯示「沒有歌詞」；網路失敗和真的沒收錄都顯示「沒有歌詞」。

- [ ] **(a) 歌詞條狀態。** 換歌後先看到「查詢中…」，接著是「純文字版」或「沒有歌詞」（置中，歌詞條裡沒有曲名）；
  時間軸版在第一行之前顯示「時間軸版」，開始捲動後換成歌詞。
- [ ] **(b) Expanded。** 三種結果各看一次：兩行文字沒有被裁切、按鈕位置不變、點下行仍開啟來源頁面。

---

## 13. 官方 MV 標題

YouTube 官方 MV 的標題（例如 `周杰倫 Jay Chou【愛在西元前 Love before AD】Official MV`）整串拿去查一定落空。
`core/src/lyrics/normalizer.rs` 在三個條件都符合時只取括號裡的歌名：開頭是演出者（或其一部分）、後面接 `【…】`、
結尾是 `Official MV`／`Official Music Video`；中文歌名後面的英文副標一併拿掉。畫面上顯示的曲名不變。

已知限制：

- 其他標題格式（`《…》`、頻道名不是歌手、結尾是其他字樣、整張專輯合輯）不處理。
- 影片長度和歌詞庫收錄的歌曲版本差超過 10 秒時仍查不到。
- 歌詞時間軸照歌曲版本做的；MV 前面多了劇情或前奏時歌詞會對不上。

- [ ] 播放幾支官方 MV：歌詞出得來，而且是同一首歌、時間軸對得上。

同日另外放寬歌詞比對（所有曲目都適用，不只 MV）：

- 接受曲長差 10 秒內的候選（歌名仍要相符）。超過 5 秒的配到別的版本、時間軸偏掉的機會較高，所以排在 5 秒內的後面
  （版本清單的排序見第 14 節）。
- LRCLIB 另外以轉簡體、演出者只留中文的字串搜一次（轉換後有差異時，多一個請求）。
  **2026-11-26 版本切換起改為同時查詢**：LRCLIB 的精確查詢、原文搜尋、簡體搜尋同時發出，兩個來源也同時查、等兩邊都回來
  （每個來源上限 5 秒）。原先寫在這裡的「找不到才再搜一次」與開發機的耗時（約 2.0／2.5 秒）是依序查詢時的作法與量測，
  已不適用；現在的等待時間見第 14 節 (d)。

- [ ] 放寬後配到的歌詞是否可接受：留意有沒有別的版本（現場版、翻唱）或整首偏移。偏移太常見就把
  `core/src/lyrics/identity.rs` 的 `LOOSE_DURATION_TOLERANCE` 調回 5 秒。

---

## 14. 歌詞版本切換與時間微調

設計：`docs/superpowers/specs/2026-11-26-lyric-versions-design.md`。同一首歌在歌庫裡常有時間軸不同的好幾份歌詞，哪一份對得上取決於播放的音檔，
程式無法判斷，所以讓你在 Expanded 邊聽邊換；都對不上時微調時間。收合的瀏海列只反映結果，沒有新的控制項。

- **版本怎麼來。** 啟用的來源同時查、全部回來（或逾時）才顯示：每個來源上限 5 秒（單一請求：LRCLIB 4 秒、網易雲搜尋 2.5 秒、取歌詞 2 秒）。
  LRCLIB 收下三個請求的所有合格紀錄，網易雲最多取前 3 首合格歌曲的歌詞；時間軸幾乎相同的合併成一個版本（`core/src/lyrics/versions.rs`）。
  預設是第一個：含曲長差 5 秒內紀錄的版本在前，再依紀錄筆數。有來源失敗時清單可能缺版本，只快取 120 秒。
- **切換版本。** 控制列左側上行：兩個以上版本時是「時間軸版 1/3 ›」，點一下換下一個（最後一個之後回到第一個），歌詞直接跳到目前行；
  只有一個版本時是「時間軸版」、不可點。下行的來源連結指向**目前版本**。
- **時間微調。** 控制列右側（折疊箭頭左邊）是 `−`、目前偏移、`＋`。**＋ 讓歌詞提早出現**（歌詞慢半拍時按 ＋）；點一下 0.5 秒，
  按住 Option 點是 5 秒，範圍 ±120 秒，點中間的數字歸零。每個版本各自記自己的偏移。查詢中、沒有歌詞、純文字版時，版本切換與偏移控制都不出現。
- **記住。** 只有按過切換或偏移才寫進 `lyric-choices.json`（設定檔同一個資料夾，內容見 `rust/README.md`）；以歌名、演出者加曲長（差 2 秒以內）辨認，
  下次播同一首自動套用。查詢結果裡找不到記住的版本時，用存下的歌詞內容。

已知限制：同名但長度不同的曲目（MV 版與專輯版）分開記。沒有「忘記這首」的按鈕（偏移點數字歸零，版本再切回想要的）。
LRCLIB 搜尋每次最多回 20 筆。瀏海寬小於 164 點時控制列右側放不下三個控制項、會被裁掉（中間的按鈕群不受影響）；虛擬瀏海設得很窄會遇到，
把有瀏海的顯示器調成「較大文字」推算也可能遇到（沒有實機確認）。紀錄檔讀不進來的內容會備份成 `.bak`、`.bak2`…，舊備份要自己清。

已自動驗證（2026-11-26，開發機內建螢幕、瀏海 185 點）：版面文字與寬度的單元測試；`music_demo --scene versions` 以暫時加入、未提交的程式碼
強制展開並在示範程式內部合成點擊（沒有移動游標），只截面板區域目視：三個控制項不重疊、不裁切（含最寬的 `−120.0s`），有沒有控制項時三顆按鈕位置相同，
純文字版與沒有歌詞的曲目沒有控制項，換版本後來源連結跟著換、歌詞直接跳到目前行，暫停中按 ＋ 畫面立即更新。
**沒有驗證**：真正的滑鼠／觸控板點擊（含 Option 與手形游標）、200 點寬的虛擬瀏海外觀、真實來源與重啟後保留。

示範：`cargo run --release -p musicat-shell --example music_demo -- --scene versions`（兩個相差 5 秒的版本；選擇只在記憶體裡、不寫檔，
點來源連結只印網址）。以下用真正的 App 確認：

- [ ] **(a) 切到對得上的版本。** 播放《上海 一九四三》：能切到另一組時間軸（2026-11-26 查到第一句約 3.4～3.8 秒與 8.4 秒兩組），
  而且其中一組對得上。順便留意版本清單裡有沒有混進別首歌的歌詞、有沒有明明一樣卻被分成兩個版本。
- [ ] **(b) 重啟後保留。** 切版本、調偏移後結束 App 再開，播同一首：版本與偏移照舊。
- [ ] **(c) 沒有歌詞的歌不顯示控制項。** 查不到歌詞的歌、純文字版、以及「查詢中…」的時候，只有狀態文字，沒有 `−`／偏移／`＋`，
  三顆按鈕位置不變。
- [ ] **(d) 等待時間可以接受。** 換歌到歌詞出現：現在每首新歌都等兩個來源，通常比以前多約 1 秒（設計時的估計，沒有量測），最慢約 5 秒。
- [ ] **(e) 點擊手感。** 在非啟用面板上點版本標籤、`−`、數字、`＋` 都有反應，游標是手形，Option 點是 5 秒，點完不搶焦點；
  播放中與暫停中都立即更新。9 點的小字看得清楚、按得準（`music_layout::OFFSET_BUTTON_WIDTH`／`OFFSET_VALUE_WIDTH`）。

---

## 附錄 A　Task 21 GPUI 視窗驗證

對應 GPUI 0.2.2 與 `rust/platform-macos/src/panel.rs` 的繞路。以下都用煙霧範例：

```sh
cd rust
MUSICAT_SMOKE_SECS=120 cargo run -p musicat-shell --example panel_smoke
```

它會在有瀏海的螢幕（沒有就用主螢幕與 200 × 32 的虛擬瀏海）頂端顯示黑色 Preview 長條，左翼 `MusiCat`、右翼 `現正播放`，
`MUSICAT_SMOKE_SECS` 秒後自動結束（預設 8 秒）。游標移進／移出長條時 stdout 會印 `click-through=OFF/ON`。

Task 21 已自動驗證（2026-11-18，macOS 15.7 arm64，主螢幕為有瀏海的內建 1512 × 982，另接兩台外接螢幕），
由外部工具（`spikes/gpui-notch/tools/winfo.swift`、`lsappinfo`、System Events）量測：

- 層級與外框：CGWindowList `layer=25`（選單列 24）、`bounds X=583 Y=0 345×180`，與 core 的 `container_frame` 一致。
- `describe_panel`：`class=GPUIPanel level=25 styleMask=128（Borderless|NonactivatingPanel）key=false main=false
  canBecomeKey=false canBecomeMain=false behavior=337（CanJoinAllSpaces|Stationary|IgnoresCycle|FullScreenAuxiliary）`。
- 不出現在 Dock／Cmd-Tab：`lsappinfo` 的 `ApplicationType="UIElement"`；System Events 的非 background-only 行程中沒有它。
- 不搶焦點（啟動時）：前景 App 在執行前、執行中、結束後都是同一個（Slack）。
- 座標：`list_screens()` 的外接螢幕外框與 CGWindowList 中各螢幕選單列視窗的 bounds 相同；`mouse_location()` 與 `CGEvent` 位置一致。

下列項目**尚未**驗證：

- [ ] **目測長條與瀏海是否對齊。** 容器 x 為 583.5，AppKit 落在 583，所以在這台 Mac 上長條可能比瀏海中心偏 0.5 點；
  另外 aux 區回報的瀏海高 32，而內建螢幕的選單列視窗高 37，確認長條高度看起來是否貼合瀏海。

- [ ] **點擊穿透（面板外）。** 開一個 Finder 視窗，把它拖到面板正下方、與面板的固定容器（Preview 長條下方約 180 點高、
  與長條同寬的透明區域）重疊。執行煙霧範例，點容器內、長條以外的透明處 → 應該點到 Finder 視窗（例如選取檔案），
  stdout 維持 `click-through=ON`。
- [ ] **點擊長條不搶焦點、不啟動 App。** 先讓 TextEdit 在前景並把游標放在文件裡打字。執行煙霧範例，用滑鼠點一下黑色長條，
  再繼續打字 → 字應該仍輸入到 TextEdit；選單列左上角的 App 名稱仍是 TextEdit；Dock 與 Cmd-Tab 都沒有 `panel_smoke`。
  可在另一個終端機確認：
  `osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'`。
- [ ] **跨空間。** 開兩個桌面空間（Mission Control 右上角 +）。執行煙霧範例後用 Ctrl-→／Ctrl-← 切換空間 →
  長條在每個空間都在同一位置、切換時不跟著滑動（Stationary）、不出現在 Mission Control 的視窗縮圖中。
- [ ] **其他 App 的全螢幕空間（決定 `foreground_is_fullscreen()` 要不要實作，spec 第 5.3 節）。**
  讓 Safari 進入全螢幕（綠色按鈕或 Ctrl-Cmd-F），再執行煙霧範例（`MUSICAT_SMOKE_SECS=60`），並在範例執行中切進、切出全螢幕空間。
  記錄：長條在全螢幕空間裡**看得見還是被系統隱藏**？選單列自動隱藏時它是否仍在？
  - 若被系統自然隱藏：`foreground_is_fullscreen()` 維持回 `false`，把實測結果寫進它的文件註解，結案。
  - 若仍顯示在全螢幕 App 之上：照 `foreground_is_fullscreen()` 註解裡的計畫，以 `CGWindowListCopyWindowInfo`
    判斷前景 App 是否有覆蓋整個螢幕的第 0 層視窗，另開任務實作。
  - 另外記錄：把 `FullScreenAuxiliary` 拿掉是否改變結果（只是資訊，不必改程式）。
- [ ] **有 Metal Toolchain 時不用 `runtime_shaders` 的建置方式。** 若作者的 Mac 裝了 Metal Toolchain（`xcrun -sdk macosx metal --version` 成功），
  把 `rust/shell/Cargo.toml` 的 `features = ["runtime_shaders"]` 暫時拿掉跑一次煙霧範例，確認預先編譯 shader 的路徑也能開視窗，
  結果寫回 `rust/README.md` 的建置說明。Task 21 的建置機器沒有 Metal Toolchain：拿掉 feature 時 gpui 的 build script 確實失敗（`cannot execute tool 'metal' due to missing Metal Toolchain`），所以只驗證了 `runtime_shaders` 這條路。

## 附錄 B　Task 22 選單列

對應 `rust/platform-macos/src/tray.rs` 與 `rust/docs/adr-0002-menu-bar-item.md`。用煙霧範例（同時顯示面板與選單列圖示）：

```sh
cd rust
MUSICAT_SMOKE_SECS=120 cargo run -p musicat-shell --example tray_smoke
# PNG（template）圖示：MUSICAT_TRAY_ICON=/path/to/icon.png MUSICAT_SMOKE_SECS=120 cargo run -p musicat-shell --example tray_smoke
```

範例會在第 2、3 秒以程式點選「LRCLIB」與「自動」、第 4 秒自己打開選單 1.5 秒，最後一秒以程式點選「結束」。
手動測試時請在第 6 秒之後再操作（避免和自動步驟搶），結束前都可以自己點。stdout 會印出每個 `MenuEvent` 與 `tray after set_model:`。

Task 22 已自動驗證（2026-11-18，macOS 15.7 arm64）：圖示建立後 `visible=true`、程式觸發的三個項目都送出正確的 `MenuEvent`
（回呼在主執行緒）、`set_model` 後勾選翻轉、程式打開的選單 `menuOpen=true` 且選單開著時面板輪詢照常（1.5 秒 85–89 次）、
面板 `level=25 canBecomeKey=false`、`ApplicationType="UIElement"`、結束時圖示移除且行程 `exit=0`。
System Events 沒有輔助使用權限，無法以 UI scripting 點擊，以下需要肉眼與滑鼠確認：

- [ ] **圖示看得見、外觀正確。** 選單列右側出現「♪」。切換「系統設定 → 外觀」淺色／深色（以及桌面底色讓選單列變淺／變深）時，
  文字與 PNG（`MUSICAT_TRAY_ICON`）圖示都跟著變成黑／白、清楚可讀、垂直置中。多螢幕時確認出現在哪個螢幕的選單列上。
- [ ] **瀏海不會把圖示藏起來。** 內建有瀏海的螢幕上，若右側圖示很多，確認「♪」沒有被擠到瀏海後面；
  若被藏起來，記錄下來（系統行為，之後在 README 說明以 ⌘ 拖曳調整順序）。
- [ ] **點一下圖示會開選單。** 用滑鼠點「♪」→ 選單出現，項目依序為：瀏海式（勾）、分隔線、自動與各螢幕（只有一個勾）、分隔線、
  LRCLIB／網易雲／macOS perl 宿主／顯示歌詞條（勾選依狀態）、分隔線、結束 MusiCat（⌘Q）。再點一下或按 Esc 會關閉。
- [ ] **勾選會翻轉。** 點「網易雲」→ stdout 印 `MenuEvent received: Toggled("lyrics.netease", false)`；再打開選單，「網易雲」的勾消失。
  點某個螢幕 → 該螢幕打勾、「自動」的勾消失。
- [ ] **選單開著時面板照常。** 打開選單後把游標移到瀏海的 Preview 長條上 → stdout 仍會印 `click-through=OFF/ON`（hover 輪詢沒有停）。
  關閉選單後 hover 長條也正常。
- [ ] **結束可用。** 點「結束 MusiCat」→ 圖示消失、面板消失、行程結束（stdout 有 `quitting (status item removed=true)`）。
- [ ] **沒有 Dock 圖示、不搶焦點。** 範例執行中 Dock 與 Cmd-Tab 都沒有 `tray_smoke`；打開選單、點項目後，左上角的前景 App 名稱不變
  （`osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'`）。
- [ ] **圖示與面板不重疊。** 若「♪」剛好落在瀏海旁的 Preview 長條範圍內（兩者都在 25 層），記錄誰蓋住誰。

## 附錄 C　Task 23 外殼視窗、固定容器與動畫

對應 `rust/shell/src/window.rs`（`ShellWindow`）與 core 的 `shell_logic::{animation, driver}`。用示範程式（佔位內容，真的 hover／動畫機制）：

```sh
cd rust
cargo run -p musicat-shell --example shell_demo            # 60 秒後自動結束（MUSICAT_DEMO_SECS 可改）
# 每 6 秒切換一次 24 點高的佔位歌詞條（MUSICAT_DEMO_TOGGLE_SECS、MUSICAT_DEMO_EXTRA 可改）；MUSICAT_DEMO_BLINK=1：第 3 秒隱藏、第 4 秒再顯示
# MUSICAT_SHELL_TRACE=1：每畫一格印 shell-frame（高度、圓角），以及待收合／轉移時的游標位置
```

stdout 每次轉移印一行 `expand`／`collapse`／`visible=..`（epoch 時間戳 + 啟動後毫秒）。

Task 23 已自動驗證（2026-11-18，macOS 15.7 arm64，內建 1512 × 982 有瀏海 185 × 32）：
以 `spikes/gpui-notch/tools/hover.swift` 的 13 步腳本驅動真實游標，每一步的期望都成立
（不在 Preview 下方的容器內展開、進長條展開一次、抖動不收合、離開 150 ms 後收合、100 ms 內出去又回來不收合、
快速進出 ×6 與跨長條下緣抖動都只展開一次、收合時停在長條下方不展開）；`anim.swift` 加 `MUSICAT_SHELL_TRACE` 量到展開 284 ms、
收合 285 ms（約 120 fps）；CGWindowList 外框 `X=583 Y=0 345×180`、`layer=25`，`set_visible(false)` 後 `onscreen` 消失、再顯示後回來；
閒置 CPU（release，15 秒）約 0.9%。以下需要人工：

- [ ] **動畫手感。** hover 長條展開、移開收合：大小與下方圓角（10 → 24）一起平順變化約 0.28 秒，沒有跳格、閃爍或內容重新排版；
  動畫中途反向（剛展開就離開）從當下的樣子接著縮回，不會跳。對照 Swift 版的感覺。
- [ ] **游標靜止在長條邊緣不閃爍（真實觸控板）。** 用觸控板（不是腳本）把游標慢慢移到 Preview 長條下緣附近停住、手指輕放在觸控板上
  → 不會開開關關；展開後在容器內小幅晃動不收合；離開後約 150 ms 才收合。
- [ ] **收合時點擊穿透（透明區域）。** 開一個 Finder 視窗拖到面板正下方，讓它與固定容器（長條下方、同寬、約 180 點高的透明區域）重疊。
  收合時點容器內、長條以外的透明處 → 點到 Finder（例如選取檔案）。也點一下長條本身旁邊的選單列項目（如果被容器蓋到）→ 照常作用。
- [ ] **展開時接收點擊、不搶焦點。** 讓 TextEdit 在前景打字，hover 展開後點一下展開的黑色區域，再繼續打字 → 字仍輸入到 TextEdit、
  前景 App 不變（Task 24 有按鈕後再驗一次按鈕可點）。
- [ ] **全螢幕 App 之上的行為。** `foreground_is_fullscreen()` 仍固定回 `false`（見 Task 21 的同名待辦）；在 Safari 全螢幕空間裡
  hover 面板位置，記錄面板是否出現、是否會展開。
- [ ] **內容裁切。** GPUI 0.2.2 只能矩形裁切：佔位內容刻意不碰下方兩角。Task 24 的真實內容上線後，看下方圓角外是否露出內容。
- [ ] **隱藏／再顯示。** `MUSICAT_DEMO_BLINK=1` 時第 3 秒面板消失（系統會有很短的淡出）、第 4 秒回來，位置大小不變。
- [ ] **Preview 歌詞條的黑底與半透明。** 示範每 6 秒切換歌詞條（stdout 印 `extra=24`／`extra=0`）。出現時：瀏海高度那一段仍是純黑、
  下方兩角從圓角平順變成直角，下面長出一條看得到桌面／視窗透出來的 0.6 黑色帶；消失時反過來，黑底高度不跳。
  展開與收合（有歌詞條時）黑底從瀏海高度平順長到 180、圓角 0 → 24，反之亦然。
- [ ] **drop 即關閉。** 示範結束前 drop `ShellWindow`：面板立刻消失，stdout `poll after drop: ... stopped=true`（自動檢查，記錄即可）。

## 附錄 D　Task 24 音樂畫面（Preview 與 Expanded）

對應 `rust/shell/src/{music_views.rs, music_layout.rs, marquee.rs, artwork.rs}`。用示範程式（離線、腳本來源、自寫歌詞、
程式產生的封面；控制器、模組狀態、模組管理器、外殼視窗與 hover 機制都是真的）：

```sh
cd rust
cargo run --release -p musicat-shell --example music_demo -- --scene playing   # 60 秒後自動結束（MUSICAT_DEMO_SECS 可改）
# --scene playing|paused|none|plain|nolyrics|longtitle|versions（版本切換與時間微調，見第 14 節）
# MUSICAT_DEMO_NO_CONTROL=1：沒有播放控制（按鈕停用樣式）；MUSICAT_DEMO_STRIP=0：歌詞條一開始是收起的
```

Expanded 的按鈕作用在腳本來源上：播放／暫停凍結或恢復位置（stdout 印 `control TogglePlayPause`），上一首／下一首在
「夜航 Night Ferry」（有同步歌詞、紅色封面）與「晨光 Morning Light」（沒有歌詞、藍色封面）之間切換。

Task 24 已自動驗證（2026-11-19，macOS 15.7 arm64，內建有瀏海 185 × 32，容器 583.5,0 345 × 180）：純邏輯單元測試；
六個場景都能啟動並在時間到時正常結束（exit 0）；以 `screencapture` 只截瀏海區域、目視確認（截圖已刪除）：
- `--scene playing` Preview：左翼封面置中裁切（沒有灰色條紋）、圓角；右翼四根音柱；瀏海下方 0.6 黑歌詞條，中文歌詞行正常。
- `--scene playing` Expanded（以 `CGWarpMouseCursorPosition` 把游標移進長條 2.5 秒後還原，唯一一次移動游標）：♪ 切換列、
  68 點封面、曲名／演出者（中文正常）、進度條、上一首／暫停／下一首與折疊箭頭的字形、歌詞區目前行加亮且其他行變暗。
- `--scene longtitle` Preview 第 14 秒：超長歌詞行在歌詞條內往左捲動，左右被內距裁切。

CPU（release、螢幕醒著、停在 Preview、15 秒 `ps -o time`）：音柱 15 fps 時約 4.5%，降到 12 fps 後約 3.6%（取樣 2.1／3.0／3.4%）；
`longtitle` 歌詞條跑馬燈以顯示器頻率重繪時約 11%（取樣最高 18.6%），改由定時器以 30 fps 重繪後約 5.4%（取樣最高 7.6%）。
（Task 23 外殼本身閒置約 0.9%。）沒有自動驗證的項目照下列清單人工確認；已目測的項目仍保留，請以真實觸控板與實機再看一次觀感。

- [ ] **（HANDOFF 1）Preview 版面。** `--scene playing`：長條左翼是封面（24 × 24、圓角 6、離左端 8）、右翼是四根白色音柱
  （離右端 12，播放中上下起伏）、瀏海正下方是 24 點高的 0.6 黑色歌詞條（下方圓角 10，左右內距 12，白色 12 點中字）。
  真瀏海遮住的中央沒有任何內容。開頭 4 秒歌詞條顯示「時間軸版」（較淡 0.7），之後每 8 秒換一行歌詞（較亮 0.95）。
  播放中封面右邊另有曲名、音柱左邊另有演出者（11 點，放不下就跑馬燈，不進瀏海範圍、不蓋住封面與音柱）；暫停時兩邊文字消失。
  長條寬 445 點（瀏海兩側各 130 點，Swift 版是各 80）：確認蓋到的選單列範圍可以接受，且游標掠過瀏海旁的選單項目時
  展開範圍變大是否會礙事。
- [ ] **封面置中裁切與圓角。** 示範封面是 480 × 300：中央紅色方塊加白色圓環，左右兩側是灰色條紋。畫面上只能看到紅色與白環，
  **不可**出現灰色條紋；四角是圓角（Preview 與 Expanded 都看）。切到第二首（下一首）應變成藍色封面。
- [ ] **（HANDOFF 2）暫停保留封面與音柱。** `--scene paused`：Preview 有封面與靜止的音柱、沒有歌詞條（2026-11-25 起與 Swift 不同）。
  playing 場景中在 Expanded 按暫停 → 收合後歌詞條收起（0.28 秒動畫）、封面與靜止音柱留著；再按播放歌詞條回來、音柱起伏。
- [ ] **（HANDOFF 1）Expanded 版面。** hover 展開：最上方置中一個 ♪（模組切換列，高約 15，選取時全白），下方左側 68 × 68 封面，
  右側曲名（13 點粗）、演出者（11 點、0.65）、進度條（3 點高，隨時間前進）、控制列（上一首／播放暫停／下一首置中，最右側是
  折疊箭頭）；下方是歌詞區，目前行較亮且粗、其他行 0.45，換行時以 0.3 秒平順捲動讓目前行置中。
- [ ] **（HANDOFF 14）版面預算。** 切換列 15 + 間距 8 + 音樂頁 149（下方內距 8、標頭 68、間距 6、歌詞 67）= 172。
  確認沒有擠壓或裁切：曲名不被切頭、控制列完整、歌詞區最後一行不貼到外形底部；**下方 24 點圓角外沒有露出任何內容**
  （GPUI 只能矩形裁切；內容左右縮 16、底部縮 8，單元測試已確認內容角點在圓角內）。
- [ ] **中文字。** 曲名、歌詞、「目前沒有播放」、說明文字都是正常的中文字形（不是方框）。
- [ ] **跑馬燈。** `--scene longtitle`：Expanded 的曲名與演出者放不下，以每秒 30 點往左捲、尾端隔 32 點接著下一份，無縫循環；
  放得下的文字置中靜止。Preview 歌詞條在第 12–20 秒（第二行是超長行）也要捲動。捲動是否夠平順（跑馬燈由定時器以 30 fps 重繪、每格 1 點；音柱 12 fps）。
- [ ] **（HANDOFF 7）歌詞條觀感與折疊。** 0.6 不透明、24 點高的觀感；在 Expanded 點最右側的箭頭（向上 = 展開中）→ 箭頭變向下，
  收合後 Preview 沒有歌詞條（長條只有瀏海高）；再點一次恢復。折疊後 hover 範圍跟著變小（只有黑色長條會觸發展開）。
- [ ] **（HANDOFF 15）歌詞條出現的動畫。** Rust 版內容槽固定為目標大小、由外形裁切，所以 0.28 秒動畫期間封面**不會**從零長出來
  （Swift 會）；確認歌詞條是從長條下方「被揭開」、看起來自然。
- [ ] **（HANDOFF 8）按鈕可點、不搶焦點。** 讓 TextEdit 在前景打字，hover 展開後點播放／暫停 → stdout 印 `control TogglePlayPause`、
  圖示在 ▶ 與 ❚❚ 間切換、進度凍結／恢復；上一首／下一首換曲（封面變色、第二首沒有歌詞所以歌詞區消失、標頭置中）；
  點完繼續打字仍輸入到 TextEdit。`MUSICAT_DEMO_NO_CONTROL=1`：三個按鈕變暗（0.3）且點了沒有反應。
  真實播放器（YouTube Music、Spotify）的控制要等 Task 25 接上 app 後再驗。
- [ ] **模組切換列。** `--scene paused` 展開：直接是音樂頁、♪ 全白（暫停仍是 active 模組），暫停圖示是 ▶。
  `--scene none` 展開：選取是「無」，顯示「沒有進行中的內容，點上方圖示開啟模組」、♪ 是 0.4 的暗色。
- [ ] **沒有曲目。** `--scene none`：Preview 只有黑色長條；Expanded 是說明文字；點 ♪ 後是一行置中的「目前沒有播放」。
- [ ] **純文字與沒有歌詞。** `--scene plain`：歌詞條顯示「純文字版」；Expanded 歌詞區是置中的靜態全文（0.75），可以用觸控板捲動。
  `--scene nolyrics`：歌詞條顯示「沒有歌詞」；Expanded 沒有歌詞區、標頭垂直置中。
- [ ] **（HANDOFF 6、16）捲動條。** GPUI 不畫捲動條：在「系統設定 → 外觀 → 顯示捲動條：永遠」下，`--scene plain` 的歌詞區也不應出現捲動條。
- [ ] **（HANDOFF 13）CPU 與平順度的取捨。** 目前音柱 12 fps（約 3.6%，略高於 3% 目標）、跑馬燈 30 fps（每格 1 點）。
  請目視音柱與跑馬燈是否夠平順；若可接受更頓的音柱，把 `music_layout::EQ_FRAME_SECONDS` 調成 1/10 再量；
  若跑馬燈太頓，`MARQUEE_FRAME_SECONDS` 可提高（CPU 會跟著增加）。在真實 App（Task 25）播放時再量一次。
- [ ] **（HANDOFF 3）拖動進度。** 要真實來源（Task 25）：在播放器拖動進度後，Expanded 的進度條與目前歌詞行在 1 秒內跟上。
- [ ] **字形。** 控制列的上一首／播放／暫停／下一首與折疊箭頭是以路徑畫的（GPUI 沒有 SF Symbols），對照 Swift 版確認大小與粗細可接受；
  ♪（U+266A，系統字型）在切換列與封面佔位是否清楚。

## 附錄 E　Task 25 真正的 App（組裝、選單、螢幕、喚醒）

對應 `rust/app/src/{main.rs, composition.rs, gated_source.rs}`、`rust/shell/src/runtime.rs`、`rust/platform-macos/src/power.rs`。
執行方式見 `rust/README.md` 的「執行（macOS）」：

```sh
cd rust
MUSICAT_LOG=1 cargo run --release -p musicat-app     # MUSICAT_LOG=1 會在 stderr 印曲名與演出者，自己看就好
```

Task 25b 已自動驗證（2026-11-20，macOS 15.7 arm64，只接內建有瀏海的螢幕，顯示器醒著且未鎖定）：
- `MUSICAT_LOG=1 MUSICAT_AUTO_QUIT_SECS=25`：面板 `layer=25 bounds 583,0 345×180`（CGWindowList）；選單列圖示的視窗
  `Item-0`（layer 25、27 × 37）存在，`MenuBar::describe` 為 `visible=true` 且選單項目正確——但這台機器選單列圖示太多，
  21 個圖示裡有 17 個（包括 MusiCat）被系統移到畫面外；perl helper 在啟動 0.08 秒內出現（`pgrep -f musicat_helper_main`），
  日誌有現正播放的更新（當時是暫停中的曲目，來源 `mediaremote`）；25 秒時自己結束、`exit=0`、helper 已不在。
- `kill -9` App：0.05 秒內 `pgrep -f musicat_helper_main` 為空（helper 在 stdin 關閉時自行結束）。
- 損毀的設定檔（以 `MUSICAT_CONFIG` 指到暫存檔）：印出警告、以預設值啟動（perl 宿主照預設開啟），檔案的 SHA-1 前後相同、沒有 `.bak`。
  `perl_host = false`：沒有 helper 行程，只用視窗標題備援。
- 閒置（曲目暫停、Preview 空黑條）15 秒：App CPU 0.19 秒 ≈ 1.3%，RSS 約 42 MB；helper 0.04 秒 ≈ 0.3%，RSS 約 13 MB。
- 喚醒通知：`cargo run -p musicat-shell --example wake_smoke` 以程式發出 `NSWorkspaceDidWakeNotification`（**沒有**讓機器睡眠），
  回呼在主執行緒上被呼叫 2 次、取消登記後不再被呼叫，`PASS`。

以下需要滑鼠、外接螢幕或真的睡眠，請人工確認（括號內是對應的 Swift `docs/HANDOFF.md` 項目編號）：

- [ ] **選單用滑鼠操作。** 點「♪」打開選單（看不到圖示時先按住 ⌘ 把其他圖示拖開）。項目：自動、各螢幕、分隔線、
  歌詞來源：LRCLIB、歌詞來源：網易雲音樂、顯示歌詞條、使用 perl 宿主讀取現正播放（macOS）、分隔線、結束 MusiCat（⌘Q）。
  每點一次，勾選立刻翻轉，`MUSICAT_LOG=1` 時 stderr 有對應的一行；設定檔（`~/Library/Application Support/MusiCat/musicat.toml`）
  只在值真的改變時被寫入。
- [ ] **換螢幕（接外接螢幕）。** 選單選外接螢幕 → 面板 2 秒內移到外接螢幕頂端（沒有瀏海時是 200 × 32 的虛擬瀏海），重新啟動後仍在那裡。
  拔掉外接螢幕 → 面板移回內建螢幕、程式不結束、選單裡外接螢幕的項目消失且「自動」**不**打勾（設定仍記著外接螢幕）；
  接回來 → 面板移回外接螢幕。選「自動」→ 回到有瀏海的內建螢幕。闔上筆電只用外接螢幕時面板在外接螢幕上。
- [ ] **（9）從選單關閉 perl 宿主。** 播放 YouTube Music 時取消勾選「使用 perl 宿主」→ `pgrep -f musicat_helper_main` 變空，
  畫面改由視窗標題備援顯示（只有曲名，見下一項）；再勾回來 → helper 回來、資料恢復完整。啟動時就是 `perl_host = false` 的話，
  勾選後才第一次建立 helper（不需要重新啟動）。
- [ ] **（9）helper 被殺掉後恢復。** `pkill -f musicat_helper_main` → 1 秒左右 helper 自動重啟、畫面恢復。
  暫時把 `target/release/libmusicat_mediaremote_helper.dylib` 改名後啟動 → 不崩潰，stderr 印一次 `system now-playing unavailable`，
  只剩視窗標題備援；（改回名字後要重新啟動才會用 helper。）
- [ ] **（11、20）視窗標題備援。** 關掉 perl 宿主後只顯示曲名（沒有演出者、不查歌詞）；在有／沒有「螢幕錄製」權限下各看一次
  （沒有權限時預期什麼都不顯示）。
- [ ] **（10）LRCLIB／網易雲開關與離線。** 兩個都關 → 下一首起沒有歌詞、不發任何請求；只開一個 → 只查那一個。
  切換只影響**之後的查詢**，通常是下一首（目前曲目已顯示的歌詞保留到下一次查詢：換曲，或同一首的曲長晚到、改變超過 2 秒）。
  斷網時歌詞靜默消失、曲目顯示不受影響。
- [ ] **歌詞條折疊的記憶。** 在 Expanded 點右下角箭頭把歌詞條收起 → 2 秒內設定檔的 `lyric_strip_visible` 變成 `false`、
  選單的「顯示歌詞條」勾選跟著消失；結束再啟動仍是收起的。從選單切換也一樣，且 Expanded 的箭頭方向跟著變。
- [ ] **（17）睡眠與喚醒。** 播放音樂時讓 Mac 睡眠再喚醒 → `MUSICAT_LOG=1` 時 stderr 有 `system woke from sleep`，
  helper 被換成新的行程（pid 改變），幾秒內畫面上的曲目與進度是新的。
- [ ] **（18）結束不留 helper。** 從選單「結束 MusiCat」→ 圖示與面板消失、`pgrep -f musicat_helper_main` 為空；
  對 App 下 `kill -9` 也一樣（已自動驗證，請在實際使用中再看一次）。
- [ ] **單一執行個體。** 啟動 App 兩次：第二個在 stderr 印出「MusiCat 已在執行中，結束。／MusiCat is already running; exiting.」並以非 0 狀態碼（1）結束，
  第一個照常執行、`pgrep -x musicat` 只有一個、`pgrep -fc musicat_helper_main` 只有一個。結束第一個之後（從選單「結束 MusiCat」與 `kill -9` 各試一次）
  馬上再啟動能正常起來（沒有過期的鎖）。用別的 `MUSICAT_CONFIG` 啟動的執行個體不受影響（鎖檔 `musicat.lock` 在各自的設定檔旁）。
- [ ] **（19）重新建置後恢復。** App 執行中重新 `cargo build --release -p musicat-app`（動態庫被重新連結）→ `pkill -f musicat_helper_main`
  之後 helper 能以新的動態庫重啟。
- [ ] **真實播放器的控制與 CPU。** YouTube Music、Spotify 各播一首：Preview／Expanded 正常、按鈕控制得動（Task 24 的 HANDOFF 8 待辦）、
  播放中（音柱動畫）App 的 CPU 量一次（`ps -o time` 15 秒）。

選單列圖示看不到有兩個原因：選單列太擠（macOS 會把放不下的圖示移到畫面外；Task 25b 的開發機上 21 個圖示有 17 個被移走，包括 MusiCat），以及有瀏海的螢幕上**瀏海本身**也會把右側的圖示擋住（README 有同樣的說明）。兩者的處理方式相同：按住 ⌘ 拖曳其他圖示騰出位置。

已知限制：沒有「螢幕參數變更」的即時通知，螢幕增減最多 2 秒後才反映；Expanded 箭頭造成的歌詞條變更最多 2 秒後才寫回設定檔
（2 秒內被 `kill -9` 會遺失）。
