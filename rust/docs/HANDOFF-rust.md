# MusiCat Rust 版：交接給作者的待辦（需要實機或人工確認）

這份文件列出自動化做不到、必須由作者在自己的 Mac 上確認的項目。每個任務把自己的待辦附加在後面；
確認後把結果寫回 spec（`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`）第 7.2 節的表格與第 12 節，
並在這裡把該項打勾、附上日期與觀察。

## GPUI 視窗驗證（Task 21）

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

## Task 22 選單列

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

## Task 23 外殼視窗、固定容器與動畫

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

## Task 24 音樂畫面（Preview 與 Expanded）

對應 `rust/shell/src/{music_views.rs, music_layout.rs, marquee.rs, artwork.rs}`。用示範程式（離線、腳本來源、自寫歌詞、
程式產生的封面；控制器、模組狀態、模組管理器、外殼視窗與 hover 機制都是真的）：

```sh
cd rust
cargo run --release -p musicat-shell --example music_demo -- --scene playing   # 60 秒後自動結束（MUSICAT_DEMO_SECS 可改）
# --scene playing|paused|none|plain|nolyrics|longtitle
# MUSICAT_DEMO_NO_CONTROL=1：沒有播放控制（按鈕停用樣式）；MUSICAT_DEMO_STRIP=0：歌詞條一開始是收起的
```

Expanded 的按鈕作用在腳本來源上：播放／暫停凍結或恢復位置（stdout 印 `control TogglePlayPause`），上一首／下一首在
「夜航 Night Ferry」（有同步歌詞、紅色封面）與「晨光 Morning Light」（沒有歌詞、藍色封面）之間切換。

**Task 24 建置時螢幕在睡眠且工作階段已鎖定（`CGDisplayIsAsleep=1`、`CGSSessionScreenIsLocked=1`），GPUI 不畫任何東西，
所以以下目測項目全部未驗證**；自動驗證只有：純邏輯單元測試、六個場景都能啟動並在時間到時正常結束（exit 0）、
螢幕睡眠時 playing 場景 15 秒 CPU 約 0.9%（none 場景約 0.5%；**不含繪製成本**，見下方 CPU 項）。

- [ ] **（HANDOFF 1）Preview 版面。** `--scene playing`：長條左翼是封面（24 × 24、圓角 6、離左端 8）、右翼是四根白色音柱
  （離右端 12，播放中上下起伏）、瀏海正下方是 24 點高的 0.6 黑色歌詞條（下方圓角 10，左右內距 12，白色 12 點中字）。
  真瀏海遮住的中央沒有任何內容。開頭 4 秒歌詞條顯示曲名（較淡 0.7），之後每 8 秒換一行歌詞（較亮 0.95）。
- [ ] **封面置中裁切與圓角。** 示範封面是 480 × 300：中央紅色方塊加白色圓環，左右兩側是灰色條紋。畫面上只能看到紅色與白環，
  **不可**出現灰色條紋；四角是圓角（Preview 與 Expanded 都看）。切到第二首（下一首）應變成藍色封面。
- [ ] **（HANDOFF 2）暫停回到空黑條。** `--scene paused`：Preview 只剩黑色長條（沒有封面、音柱、歌詞條；同 Swift，暫停時音樂不是 active 模組）。
  playing 場景中在 Expanded 按暫停 → 收合後 Preview 也回到空黑條、歌詞條收起（0.28 秒動畫）；再按播放回來。
- [ ] **（HANDOFF 1）Expanded 版面。** hover 展開：最上方置中一個 ♪（模組切換列，高約 15，選取時全白），下方左側 68 × 68 封面，
  右側曲名（13 點粗）、演出者（11 點、0.65）、進度條（3 點高，隨時間前進）、控制列（上一首／播放暫停／下一首置中，最右側是
  折疊箭頭）；下方是歌詞區，目前行較亮且粗、其他行 0.45，換行時以 0.3 秒平順捲動讓目前行置中。
- [ ] **（HANDOFF 14）版面預算。** 切換列 15 + 間距 8 + 音樂頁 149（下方內距 8、標頭 68、間距 6、歌詞 67）= 172。
  確認沒有擠壓或裁切：曲名不被切頭、控制列完整、歌詞區最後一行不貼到外形底部；**下方 24 點圓角外沒有露出任何內容**
  （GPUI 只能矩形裁切；內容左右縮 16、底部縮 8，單元測試已確認內容角點在圓角內）。
- [ ] **中文字。** 曲名、歌詞、「目前沒有播放」、說明文字都是正常的中文字形（不是方框）。
- [ ] **跑馬燈。** `--scene longtitle`：Expanded 的曲名與演出者放不下，以每秒 30 點往左捲、尾端隔 32 點接著下一份，無縫循環；
  放得下的文字置中靜止。Preview 歌詞條在第 12–20 秒（第二行是超長行）也要捲動。捲動是否平順（以顯示器頻率重繪）。
- [ ] **（HANDOFF 7）歌詞條觀感與折疊。** 0.6 不透明、24 點高的觀感；在 Expanded 點最右側的箭頭（向上 = 展開中）→ 箭頭變向下，
  收合後 Preview 沒有歌詞條（長條只有瀏海高）；再點一次恢復。折疊後 hover 範圍跟著變小（只有黑色長條會觸發展開）。
- [ ] **（HANDOFF 15）歌詞條出現的動畫。** Rust 版內容槽固定為目標大小、由外形裁切，所以 0.28 秒動畫期間封面**不會**從零長出來
  （Swift 會）；確認歌詞條是從長條下方「被揭開」、看起來自然。
- [ ] **（HANDOFF 8）按鈕可點、不搶焦點。** 讓 TextEdit 在前景打字，hover 展開後點播放／暫停 → stdout 印 `control TogglePlayPause`、
  圖示在 ▶ 與 ❚❚ 間切換、進度凍結／恢復；上一首／下一首換曲（封面變色、第二首沒有歌詞所以歌詞區消失、標頭置中）；
  點完繼續打字仍輸入到 TextEdit。`MUSICAT_DEMO_NO_CONTROL=1`：三個按鈕變暗（0.3）且點了沒有反應。
  真實播放器（YouTube Music、Spotify）的控制要等 Task 25 接上 app 後再驗。
- [ ] **模組切換列。** `--scene paused` 展開：選取被重設為「無」（暫停時沒有 active 模組），顯示「沒有進行中的內容，點上方圖示開啟模組」、
  ♪ 是 0.4 的暗色；點 ♪ → 變全白並顯示音樂頁（停用的暫停圖示是 ▶）。收合再展開 → 又回到說明文字（每次新展開重設選取，spec 5.3）。
- [ ] **沒有曲目。** `--scene none`：Preview 只有黑色長條；Expanded 是說明文字；點 ♪ 後是一行置中的「目前沒有播放」。
- [ ] **純文字與沒有歌詞。** `--scene plain`：歌詞條顯示曲名；Expanded 歌詞區是置中的靜態全文（0.75），可以用觸控板捲動。
  `--scene nolyrics`：歌詞條顯示曲名；Expanded 沒有歌詞區、標頭垂直置中。
- [ ] **（HANDOFF 6、16）捲動條。** GPUI 不畫捲動條：在「系統設定 → 外觀 → 顯示捲動條：永遠」下，`--scene plain` 的歌詞區也不應出現捲動條。
- [ ] **（HANDOFF 13）CPU（含繪製）。** 螢幕醒著、`--scene playing` 停在 Preview（音柱 15 fps + 歌詞條）時，
  `ps -o %cpu= -p <pid>` 每 5 秒量一次、共 15 秒（release）。超過約 3% 時把 `music_layout::EQ_FRAME_SECONDS` 調成 1/12 再量。
  另外量 `--scene longtitle` 停在 Expanded（兩個跑馬燈以顯示器頻率重繪）的 CPU，記錄下來決定跑馬燈要不要限制在 30 fps。
- [ ] **（HANDOFF 3）拖動進度。** 要真實來源（Task 25）：在播放器拖動進度後，Expanded 的進度條與目前歌詞行在 1 秒內跟上。
- [ ] **字形。** 控制列的上一首／播放／暫停／下一首與折疊箭頭是以路徑畫的（GPUI 沒有 SF Symbols），對照 Swift 版確認大小與粗細可接受；
  ♪（U+266A，系統字型）在切換列與封面佔位是否清楚。
