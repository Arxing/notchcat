# ADR-0002：選單列圖示（NSStatusItem）

- 狀態：已採納（2026-11-18）
- 範圍：`rust/core/src/menu.rs`（平台中立的選單模型）、`rust/platform-macos/src/tray.rs`（macOS 渲染器）、
  `rust/shell/examples/tray_smoke.rs`（煙霧範例）
- 相關：`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md` 第 3 節（選單內容）、第 13 節「Menu bar / tray icon」；
  ADR-0001；`rust/platform-macos/src/panel.rs`（面板繞路）

## 背景

spec 第 3 節規定設定入口是選單列（macOS）／系統匣（Windows）圖示的選單：切換模式、選擇顯示螢幕（含「自動」）、
各歌詞來源開關（LRCLIB、網易雲）、macOS perl 宿主開關、歌詞條顯示、結束。先前的 GPUI spike 只驗證了面板視窗，
**沒有**驗證 GPUI 的事件迴圈在「accessory 啟動模式 + `patch_gpui_panel_class` 換掉 `GPUIPanel` 的方法」之下，
能不能再掛一個狀態列圖示，而且點選單後事件能回到 Rust。這是本 ADR 要回答的問題。

## Spike 紀錄（2026-11-18，macOS 15.7 arm64，內建有瀏海螢幕 + 兩台外接螢幕）

### 1. GPUI 0.2.2 有沒有狀態列 API？——沒有可用的

在 `~/.cargo/registry/src/*/gpui-0.2.2/` 搜尋 `NSStatusItem`、`NSStatusBar`、`tray`、`status`：

- 唯一命中的是 `src/platform/mac/status_item.rs`，但它**不在** `src/platform/mac.rs` 的 `mod` 清單裡（死碼，不會被編譯），
  而且引用的是舊版 GPUI 的路徑（`crate::geometry::rect::RectF`、`platform::Event`），在 0.2.2 根本編不過。
- `cx.set_menus` / `OsMenu`（`src/platform/app_menu.rs`）是**應用程式選單列**（畫面左上角 App 名稱旁的那排選單），
  不是狀態列圖示；而且 accessory 模式的 App 沒有前景選單列，設了也看不到。
- 結論：GPUI 0.2.2 沒有公開的狀態列／系統匣 API，必須自己做。

### 2. 候選做法

| 做法 | 相依 | 事件怎麼回來 | 評估 |
| --- | --- | --- | --- |
| (a) `tray-icon` 0.25（tauri 團隊，MIT OR Apache-2.0） | 加 `muda` 0.20、`crossbeam-channel`、`dpi`、`keyboard-types`、`once_cell`、`thiserror`，以及 `objc2-core-graphics` 等；Windows 端另帶 `windows-sys` | 全域靜態通道（`MenuEvent::receiver()`／`TrayIconEvent::receiver()`），要在 GPUI 前景執行器上輪詢 | 能用，但為了「一個圖示 + 一份選單」多帶六七個 crate；事件走行程全域的單例通道，與 MusiCat「注入、可測」的作法不合；macOS 上它內部做的事（`NSStatusItem` + `NSMenu` + 自訂 target 類別）和 (b) 一模一樣。只做了書面評估，沒有實際建置。 |
| (b) 直接用 `objc2` 建 `NSStatusItem` + `NSMenu`（新檔 `platform-macos/src/tray.rs`） | **零新 crate**：只在既有的 `objc2-app-kit` 多開 `NSStatusBar`、`NSStatusItem`、`NSStatusBarButton`、`NSButton`、`NSControl`、`NSMenu`、`NSMenuItem`、`NSCell`、`NSImage`、`objc2-core-foundation`（`CGFloat`；`objc2-core-foundation` 0.3.2 已在 Cargo.lock 裡），`objc2-foundation` 多開 `NSData` | `define_class!` 宣告的 `MusiCatMenuTarget`（`menuItemClicked:`）由 tag 查出 `MenuEvent`，呼叫註冊的 Rust 回呼 | 程式約 250 行，全部是公開 AppKit API，沒有私有 API、沒有視窗繞路。**採用。** |

### 3. 驗收（同一個行程：accessory 模式 + `patch_gpui_panel_class` + GPUI PopUp 面板 + 選單列圖示）

以 `cargo run -p musicat-shell --example tray_smoke` 實測（完整輸出見 Task 22 報告）。結果：

| 項目 | 結果 | 證據 |
| --- | --- | --- |
| (i) 圖示出現在選單列 | 通過（程式層面） | `MenuBar::describe()`：`visible=true title="♪" buttonWindow=(…, 27x24) level=25`；PNG 路徑：`title="<image 18x18 template=true>"`。肉眼確認列入 HANDOFF。 |
| (ii) 選單打得開 | 通過 | `open_menu()`（排到 run loop 下一輪對圖示按鈕 `performClick:`，與使用者點擊同一條路徑）後 NSMenuDelegate 回報 `menuOpen=true opens=1`；`close_menu()` 後 `menuOpen=false`。連跑 3 次皆同。System Events UI scripting 無法使用（`osascript不允許輔助取用 (-1719)`，沒有輔助使用權限），所以「滑鼠點擊」列入 HANDOFF 手動確認。 |
| (iii) 點選項目送出 `MenuEvent` | 通過 | 以 `-[NSMenu performActionForItemAtIndex:]`（走 AppKit 正常的 target/action 派送）觸發：`[callback on main thread=true] Toggled("lyrics.lrclib", false)`、`ScreenChosen(None)`、`Quit`；事件經 `std::sync::mpsc` 交給 GPUI 前景工作，更新狀態後 `set_model` 重建，勾選正確翻轉（`[ ] LRCLIB#8`）。 |
| (iv) 面板不受影響 | 通過 | `describe_panel` 在建立圖示後、1 秒、結束前都是 `class=GPUIPanel level=25 styleMask=128 key=false main=false canBecomeKey=false canBecomeMain=false behavior=337`；16 ms 的滑鼠輪詢照常（`click-through=ON/OFF` 切換照印）；**選單開著的 1.5 秒內面板輪詢跑了 85–89 次**（≈16.8 ms 一次）。`lsappinfo`：`ApplicationType="UIElement"`（沒有 Dock 圖示）；前景 App 仍是原本的 App（沒搶焦點）。 |
| (v) 結束 | 通過 | `Quit` 事件 → drop `MenuBar`（`status item removed=true`）→ `cx.quit()` → 行程 `exit=0`。 |

結論：**GPUI 事件迴圈與狀態列圖示可以共存**，不需要任何繞路。

### 4. Spike 中發現的陷阱：不要在 GPUI 前景工作裡同步打開選單

第一版的 `open_menu()` 在 GPUI 前景工作裡直接 `performClick:`。結果：`open_menu() returned after 337 ms; panel polls while open: 0`
——選單追蹤期間 GPUI 的其他前景工作（面板輪詢、計時器）**完全停住**。原因：GPUI 0.2.2 的 macOS 調度器把前景工作
以 `dispatch_async_f(dispatch_get_main_queue(), …)` 排進**主 dispatch queue**，而主 queue 是序列的：在其中一個區塊裡
進入巢狀的選單追蹤 run loop 時，主 queue 不會再派送下一個區塊。

改成以 `performSelector:withObject:afterDelay:0` 排到 run loop 的下一輪才點（`openMenuNow:`），選單追蹤就不在任何主 queue
區塊之內，GPUI 照常執行（上表 85–89 次輪詢）。使用者真的點圖示時，AppKit 是從 `-[NSApplication sendEvent:]` 進入追蹤，
同樣不在主 queue 區塊之內，所以實際使用沒有這個問題；但 app（Task 25）**不得**在 GPUI 的前景工作或 `update` 裡做任何會
進入巢狀 run loop 的事（同步開選單、`NSAlert runModal` 等）。

## 定案

### 資料模型放在 core（`musicat_core::menu`）

`MenuModel { items }`、`MenuItem::{Toggle, ScreenChoice, Separator, Quit}`、`MenuEvent::{Toggled, ScreenChosen, Quit}`，
都是純資料（`Clone + PartialEq + Debug`），不碰 OS API。點選時該回報什麼事件由 `MenuItem::event_when_selected()` 決定，
所有平台的渲染器共用（Windows 系統匣在 R3 直接沿用）。選單**內容**由 app 依設定組出（Task 25），平台層只負責畫。

- 開關的 id 用 `&'static str`：開關是程式裡固定的一組，id 由 app 以具名常數定義，不會在執行時產生；事件不必配置記憶體、
  可直接比對常數。不用 core 的 enum，是因為開關的集合屬於 app 的設定，放進 core 會讓 core 認識 app 的設定欄位、
  每加一個開關都要改 core；不用 `String`，是因為沒有任何 id 需要在執行時產生。代價是拼錯 id 編譯期抓不到，由 app 的
  常數與「事件 → 設定欄位」的對照測試涵蓋。
- 「切換模式」本輪只有「瀏海式」一項，以 `Toggle { id: "mode.notch", checked: true }` 表示（R4 加浮動視窗時再看要不要
  改成單選群組）。
- 點選開關時回報的是**想要的新狀態**（`!checked`）；渲染器不自己翻勾選，由 app 改設定後重新 `set_model`。

### macOS 渲染器（`musicat_platform_macos::tray`）

- 純函式 `render_items(&MenuModel) -> Vec<RenderedItem { title, kind, checked, tag }>` 與 `event_for_tag(&MenuModel, tag)`，
  所有平台編譯、有單元測試（不需要 AppKit）。`tag = 索引 + 1`，分隔線為 0（NSMenuItem 預設 tag），未知 tag 不產生事件。
  螢幕選項為一組單選：模型裡勾了不只一個時只畫第一個。空標題的退路：自動 → 「自動」、螢幕 → id、開關 → id。結束為「結束 MusiCat」（⌘Q）。
- `MenuBar::new(icon_png: Option<&[u8]>) -> Option<MenuBar>`：不在主執行緒回 `None`。PNG 設成 18 點高的 template 圖示
  （系統依淺色／深色選單列著色）；`None` 或解碼失敗時顯示文字「♪」。正式的 template 圖示是之後的打磨工作。
- `set_model` 每次建一份新的 `NSMenu` 換上去（勾選用 `NSControlStateValueOn/Off`），而不是就地改舊選單。
- **每份選單一個模型快照**（review 修正）：每次 `set_model` 都建一個新的 target（`MusiCatMenuTarget`），它的 ivar 帶著
  建出這份選單的 `MenuModel`；`menuItemClicked:` 以**自己的快照**解析 tag。使用者開著舊選單時模型被換掉（例如接上
  一台螢幕，後面的項目全部位移），點舊選單仍回報舊選單上那個項目的事件，不會對到新模型同位置的別的項目。
  回歸測試：`tray::tests::a_click_on_an_old_snapshot_resolves_against_that_snapshot`；煙霧範例以
  `perform_retired_item` 實測（舊選單索引 4 是 LRCLIB、新模型索引 4 是螢幕「DELL SE2416H」，事件仍是
  `Toggled("lyrics.lrclib", true)`）。
- **被換下的選單延後釋放**（review 修正）：NSMenuItem 對 target 是弱參考，而 AppKit 在動作回呼返回後仍會使用該選單與項目，
  所以 (NSMenu, target) 以「世代」保管（純邏輯 `MenuGenerations`，有單元測試）：剛換下的那一代一律保留到下一次
  `set_model`；更早的世代只在「沒有選單開著、也不在選單動作派送中」時才釋放；drop `MenuBar` 時全部交給
  autorelease pool，在目前的事件派送結束後才釋放（drop 可能發生在回呼裡）。因此**在回呼裡呼叫 `set_model` 是安全的**，
  煙霧範例實測了這條路徑（回呼裡 `set_model(位移模型)`，`performActionForItemAtIndex:` 正常返回、新選單已裝上），
  並以 `NSZombieEnabled=YES` 再跑一次，沒有任何 zombie 訊息。
- `on_event(Box<dyn Fn(MenuEvent) + Send + Sync>)`：回呼**在主執行緒上同步呼叫**。建議回呼只把事件送進通道
  （煙霧範例用 `std::sync::mpsc`，由 GPUI 前景工作取出處理），不得在回呼裡借用 GPUI 的 `App`。
- 記憶體：`MenuBar` 持有 `NSStatusItem`、共享狀態 `Rc<Shared>`（回呼、選單開關狀態、派送深度、已 drop 旗標）與選單世代；
  每個 target 持有 `Rc<Shared>` 與自己的快照，`Shared` 不回頭參考 target、選單或 `MenuBar`，NSMenuItem 對 target、
  NSMenu 對 delegate 都是弱參考，所以沒有參考循環。drop `MenuBar` 時 `removeStatusItem:`，圖示消失，已排程的
  `openMenuNow:` 看到已 drop 旗標就不動作。`MenuBar` 不是 `Send`。
- 巢狀 run loop 的禁忌寫在 `MenuBar` 的型別文件：`set_model`、`perform_item`、`close_menu`、`on_event`、`describe` 不開巢狀
  run loop，可在 GPUI 前景工作裡呼叫；`open_menu` 把開選單排到 run loop 下一輪。
- 診斷：`perform_item(index)`、`perform_retired_item(index)`、`open_menu()`、`close_menu()`、`is_menu_open()`、`describe()`，
  給煙霧範例與日後除錯用；`tray::is_main_thread()` 讓 shell／app 不必碰 objc2 就能檢查執行緒。

## 被否決的替案

- **`tray-icon` crate**：見上表。功能足夠，但相依多、事件走全域單例通道；我們只需要它 macOS 實作裡的那一小部分。
  R3 的 Windows 系統匣若自己寫 Win32 太繁瑣，可以重新評估只在 Windows 端用它（屆時 `MenuModel` 照樣共用）。
- **GPUI 的 `cx.set_menus`**：那是應用程式選單列，不是狀態列圖示；accessory 模式下根本不顯示。
- **GPUI 舊版的 `status_item.rs`**：0.2.2 沒有編譯它，也編不過。
- **在 `panel.rs` 裡實作**：panel.rs 專放 GPUI 視窗繞路，選單列圖示用的是正常公開 API，不是繞路，分開放，panel.rs 的繞路清單才不會混進不相干的東西。

## 殘留風險

- **瀏海遮住狀態列圖示**：有瀏海的 Mac 上，選單列右側的圖示太多時，macOS 會把放不下的圖示藏在瀏海後面（系統行為，
  沒有 API 可查）。MusiCat 的圖示可能因此看不到。列入 HANDOFF；必要時之後在 README 說明（用 ⌘ 拖曳調整順序）。
- **圖示視窗的層級與面板相同（皆為 25）**：`describe()` 量到狀態列圖示所在視窗 `level=25`，與面板的 `NSStatusWindowLevel` 相同。
  面板只在瀏海附近、圖示在右側，正常不會重疊；若使用者的圖示被擠到瀏海旁、剛好落在 Preview 長條（瀏海寬 + 160）範圍內，
  兩者的前後順序未定義。列入 HANDOFF。
- **外觀只在程式層面驗證**：淺色／深色選單列下的 template 著色、文字「♪」的大小與基線、選單在多螢幕時出現在哪個螢幕，
  都需要肉眼確認（HANDOFF「選單列（Task 22）」）。
- **多螢幕時圖示的位置會變**：`buttonWindow` 的 x 在建立當下與 1 秒後不同（圖示移到目前作用中選單列所在的螢幕），屬系統行為。
- **AppKit 行為依版本而異**：只在 macOS 15.7 驗證過。
