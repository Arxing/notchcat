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
# MUSICAT_DEMO_EXTRA=24：Preview 加 24 點額外高度；MUSICAT_DEMO_BLINK=1：第 3 秒隱藏、第 4 秒再顯示
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
