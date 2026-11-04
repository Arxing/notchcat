# NotchCat 規劃文件

## 1. 目標

在 macOS 有實體瀏海的機種（MacBook Pro 14"/16", 2021+）上，把瀏海變成一個可互動的常駐面板（類似 Dynamic Island）。

核心設計原則：**骨架（Shell）與功能（Module）分離**。

- **Shell**：只負責「瀏海長什麼樣子、什麼時候展開/收合、當前該顯示哪個模組」，本身不含任何具體功能。
- **Module**：實際功能的實作單位（音樂、電量、剪貼簿……），透過統一介面「安裝」進 Shell。

這樣可以先把骨架做穩，之後功能用疊加模組的方式擴充，彼此不互相污染。

## 2. 兩種模式

Panel 在任何時間點只會處於下列兩種模式之一：

| 模式 | 觸發條件 | 內容來源 | 尺寸 |
|---|---|---|---|
| **Preview（預覽）** | 預設狀態；滑鼠不在瀏海上時一律回到這裡 | 若有 active 模組，顯示該模組的「預覽視圖」（例如封面、音柱與歌詞條）；沒有 active 模組時，Shell 自己畫貼合硬體瀏海的黑色空條 | 高度等於實體瀏海、寬度比瀏海略寬；active 模組可向下額外要一段高度（有上限，由 Shell 夾住） |
| **Expanded（展開）** | 滑鼠 hover 在瀏海區域 | 上方是模組切換 AppBar，下方是目前選取模組的「展開視圖」；每次展開，選取會重設成當下的 active 模組，沒有 active 模組時使用者可自行點選 | 較大，動畫展開 |

### 狀態機

```
        hover 進入
Preview ───────────► Expanded
   ▲                    │
   └────────────────────┘
        hover 離開
```

規則：
- 唯一由使用者觸發的轉場是 hover：滑鼠進到瀏海區域進入 Expanded，離開則回到 Preview。
- 「有沒有 active 模組」不改變模式，只改變 Preview 裡畫什麼（模組預覽視圖，或 Shell 的空黑條）以及 Preview 的高度。
- 同時間只能有一個模組佔用 Preview 的「主內容」，由 ModuleManager 依 priority 決定（見 4.2）。

## 3. Shell（骨架）職責

1. **瀏海幾何偵測**
   - macOS 12+ 用 `NSScreen.auxiliaryTopLeftArea` / `auxiliaryTopRightArea` 算出瀏海實際區域。
   - 找不到瀏海（機種不支援）時：整個功能停用，或退化成「貼齊選單列中央的一顆小藥丸」（v1 先不做，直接停用）。
   - 只在內建螢幕（含瀏海的那顆）上顯示，多螢幕情境下其他螢幕不出現 panel。

2. **視窗管理**
   - 用 `NSPanel`（非 activating、`.nonactivatingPanel`），視窗層級設在選單列之上、可跨 Space（`.canJoinAllSpaces`, `.stationary`），不出現在 Dock / Mission Control / App Switcher。
   - 滑鼠事件：面板固定成所有模式中最大的尺寸、不隨動畫 resize；hover 判定範圍由 `NotchHostingView.hoverRect` 手動控制（進入 Preview 的可見範圍才觸發展開，一展開就放大成整個固定範圍），避免動畫中或滑鼠微晃造成閃爍。

3. **狀態機驅動**
   - 維護 Preview / Expanded 兩態，處理 hover 進出、activeModule 變化對 Preview 內容與高度的影響（見上面狀態機）。
   - 動畫：形狀（圓角矩形）與尺寸變化用 spring 動畫，畫面內容用 fade / scale 轉場，避免生硬切換。

4. **渲染容器**
   - 提供一個 SwiftUI 容器 view，根據目前模式向 ModuleManager 要「該顯示什麼」，本身不知道任何模組細節。

Shell **不**認識「音樂」「電量」這些具體概念，只認識「模組」這個抽象單位。

## 4. Module 系統

### 4.1 模組介面（現況）

實際定義在 `Sources/NotchCatKit/ModuleKit/NotchModule.swift`，摘要如下：

```swift
protocol NotchModule: AnyObject, Identifiable where ID == String {
    var id: String { get }              // 唯一識別，例如 "com.notchcat.music"
    var priority: Int { get }           // 多模組同時 active 時，最高者成為 activeModule

    // 模組是否有「環境常駐」內容要展示（例如正在播放音樂）
    // 為 true 時，Preview 改畫該模組的預覽視圖
    var isActivePublisher: AnyPublisher<Bool, Never> { get }

    // Preview 要比瀏海高多少（例如瀏海下方的歌詞條）；Shell 夾在 0...maxPreviewExtraHeight。預設恆為 0
    var previewExtraHeightPublisher: AnyPublisher<CGFloat, Never> { get }

    func tabIcon() -> AnyView           // Expanded AppBar 上的模組圖示
    func previewView() -> AnyView       // Preview 下的內容
    func expandedView() -> AnyView      // Expanded 下的內容

    // 生命週期（v1 沒有啟用／停用 UI，onInstall 後立刻 onStart；onStop／onRemove 尚未被呼叫）
    func onInstall()
    func onStart()
    func onStop()
    func onRemove()

    // 選用：模組自己的設定頁（v1 還沒有地方顯示）
    func settingsView() -> AnyView?
}
```

### 4.2 ModuleManager

- 持有所有已安裝模組的清單與啟用狀態。
- 訂閱每個模組的 `isActivePublisher`，決定「目前 Preview 的主人是誰」（多個 active 時依 priority 排序，並可能之後做輪播）。
- Expanded 狀態下：上方是模組切換 AppBar，下方顯示選取模組的 `expandedView()`；每次展開，選取重設為當下的 active 模組，沒有 active 模組時顯示 Shell 預設畫面，使用者可自行點 AppBar 選一個模組。
- 提供模組的啟用/停用、排序等管理邏輯（未來搭配設定 UI）。

### 4.3 v1 的「安裝」方式

v1 先不做真正的動態外掛載入（外部 `.bundle` / App Extension 需要處理簽章、沙盒、XPC，複雜度陡增，不適合當第一步）。

v1 的模組就是**編譯進同一個 App 的 Swift type**，符合 `NotchModule` protocol、在啟動時註冊進 `ModuleManager`。使用者端的「安裝/啟用」只是「開關某個內建模組」。

若之後真的要開放第三方寫模組，再評估：
- App Extension（獨立 process，走 XPC 溝通，隔離性好但複雜、延遲高）
- 或退而求其次，開一個 Swift Package 介面，讓社群 PR 模組進 repo，一起編譯發布（介於「內建」與「外掛」之間，複雜度低很多）。

## 5. 範例模組：Music（現正播放 + 動態歌詞）

這是規劃中最複雜的模組，先拆解：

- **資料來源**：`MediaRemote` 私有框架（無官方文件，需社群逆向/現成封裝），可取得目前播放 App、歌名/歌手/專輯圖、播放進度、播放狀態。
- **動態歌詞**：MediaRemote 本身不提供歌詞，需要另外接歌詞來源（例如自行抓取 `.lrc`、或音樂 App 若有提供逐字歌詞介面則優先用）；同步邏輯用播放進度對齊歌詞時間軸。
- **isActive**：只要偵測到「有音樂在播放」就回報 true。
- **Preview**：跑馬燈當前歌詞行 + 極小的播放進度指示。
- **Expanded**：封面圖、完整歌名/歌手、播放進度條、上一首/暫停/下一首控制、（如果有）逐句歌詞捲動。

風險提醒：`MediaRemote` 是私有 API，這類工具通常**不上架 Mac App Store**，走「公證（notarization）但外部發布」的路線，並自備更新機制（例如 Sparkle）。

## 6. 技術選型

- **語言/UI**：Swift + SwiftUI（畫面）+ AppKit（`NSPanel`、螢幕/視窗管理需要橋接的部分）。
- **狀態管理**：Combine（相容 macOS 13+）；若最低系統版本訂在 14+，可以改用 Observation framework 讓程式碼更精簡。
- **最低系統需求建議**：macOS 14（Sonoma）起，簡化狀態管理與新 API 的使用；瀏海本身只有 14"/16" MacBook Pro (2021+) 有。
- **發布方式**：直接發布 + 公證，不上 Mac App Store（因為 Music 模組等會用私有 API）；用 Sparkle 做自動更新。

## 7. 專案結構（建議）

```
NotchCat/
  App/                 # App 進入點、選單列圖示（管理已安裝模組、開關）
  Shell/               # 瀏海幾何偵測、NSPanel 管理、兩態狀態機、動畫
  ModuleKit/           # NotchModule protocol、ModuleManager、共用 UI（形狀、轉場元件）
  Modules/
    Music/             # 現正播放 + 動態歌詞
    (未來) Battery/
    (未來) Clipboard/
    (未來) Calendar/
  Resources/
docs/
  PLANNING.md          # 本文件
```

## 8. 分階段路線圖

1. **Phase 0 — 純骨架**：瀏海幾何偵測、`NSPanel` 建置、Preview ⇄ Expanded 的 hover 展開收合動畫，內容先放假資料，不接任何模組。驗收：能在真機瀏海上看到收合/展開，動畫順暢。
2. **Phase 1 — 模組介面打底**：實作 `NotchModule` protocol、`ModuleManager`，用一個最簡單的「時鐘模組」（無 active 狀態，只在 Expanded 顯示時鐘）驗證整個安裝機制跑得通。
3. **Phase 2 — Music 模組**：串接 MediaRemote、做 Preview/Expanded 畫面、動態歌詞來源與同步。這階段獨立拆成子任務執行。
4. **Phase 3 — 擴充與體驗**：更多模組（電量、剪貼簿、日曆…）、模組管理/設定 UI、多模組搶 Preview 的排序體驗調整。
5. **Phase 4 — 發布**：簽章、公證、自動更新，（可選）評估是否開放社群寫模組。

## 9. 待決事項（需要你決定，暫不動工）

- 最低支援的 macOS 版本（14 vs 13）。
- v1 要不要一開始就做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單。
- Music 模組的歌詞來源打算怎麼拿（純顯示系統提供的資訊，或額外串接歌詞服務）。
