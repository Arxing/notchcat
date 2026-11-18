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
