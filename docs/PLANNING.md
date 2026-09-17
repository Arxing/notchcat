# NotchCat 規劃文件

## 1. 目標

在 macOS 有實體瀏海的機種（MacBook Pro 14"/16", 2021+）上，把瀏海變成一個可互動的常駐面板（類似 Dynamic Island）。

核心設計原則：**骨架（Shell）與功能（Module）分離**。

- **Shell**：只負責「瀏海長什麼樣子、什麼時候展開/收合、當前該顯示哪個模組」，本身不含任何具體功能。
- **Module**：實際功能的實作單位（音樂、電量、剪貼簿……），透過統一介面「安裝」進 Shell。

這樣可以先把骨架做穩，之後功能用疊加模組的方式擴充，彼此不互相污染。

## 2. 三種模式

Panel 在任何時間點只會處於下列三種模式之一：

| 模式 | 觸發條件 | 內容來源 | 尺寸 |
|---|---|---|---|
| **Hidden（隱藏）** | 預設狀態；滑鼠不在瀏海上，且沒有模組宣告自己 active | Shell 自己畫（貼合硬體瀏海的黑色區塊，最多疊加模組給的小圖示/紅點） | 等於實體瀏海大小 |
| **Expanded（展開）** | 滑鼠 hover 在瀏海區域 | 目前的 active 模組提供的「展開視圖」；若沒有 active 模組，顯示 Shell 預設內容（時鐘/已安裝模組清單） | 較大，依內容自適應，動畫展開 |
| **Preview（預覽）** | 滑鼠離開後，若有模組宣告自己 active（例如正在播放音樂） | 該模組提供的「預覽視圖」（例如跑馬燈歌詞、封面 + 進度條） | 比 Hidden 大一點，比 Expanded 小，常駐顯示 |

### 狀態機

```
        hover 進入                    hover 離開
Hidden ───────────────► Expanded ───────────────► (依 activeModule 是否存在)
  ▲                                                     │
  │ activeModule 消失                                    │
  │                                                     ▼
  └───────────────────────────────────────────────  Preview
                    activeModule 出現 ▲│
                                      │▼ activeModule 消失
                                   Hidden
```

規則：
- Hidden ⇄ Preview 的切換**不是使用者觸發**，而是由「目前有沒有 active 模組」自動決定。
- 任何時候 hover 都會進到 Expanded；hover 離開時，依當下 activeModule 是否存在，回到 Preview 或 Hidden。
- 同時間只能有一個模組佔用 Preview / Expanded 的「主內容」，由 ModuleManager 依 priority 決定（見 4.3）。

## 3. Shell（骨架）職責

1. **瀏海幾何偵測**
   - macOS 12+ 用 `NSScreen.auxiliaryTopLeftArea` / `auxiliaryTopRightArea` 算出瀏海實際區域。
   - 找不到瀏海（機種不支援）時：整個功能停用，或退化成「貼齊選單列中央的一顆小藥丸」（v1 先不做，直接停用）。
   - 只在內建螢幕（含瀏海的那顆）上顯示，多螢幕情境下其他螢幕不出現 panel。

2. **視窗管理**
   - 用 `NSPanel`（非 activating、`.nonactivatingPanel`），視窗層級設在選單列之上、可跨 Space（`.canJoinAllSpaces`, `.stationary`），不出現在 Dock / Mission Control / App Switcher。
   - 滑鼠事件：平時只有瀏海實際區域接受 hover 偵測，其餘區域 `ignoresMouseEvents`，避免蓋住整個選單列。

3. **狀態機驅動**
   - 維護 Hidden / Expanded / Preview 三態，處理 hover 進出、activeModule 變化的轉場邏輯（見上面狀態機）。
   - 動畫：形狀（圓角矩形）與尺寸變化用 spring 動畫，畫面內容用 fade / scale 轉場，避免生硬切換。

4. **渲染容器**
   - 提供一個 SwiftUI 容器 view，根據目前模式向 ModuleManager 要「該顯示什麼」，本身不知道任何模組細節。

Shell **不**認識「音樂」「電量」這些具體概念，只認識「模組」這個抽象單位。

## 4. Module 系統

### 4.1 模組介面（草案）

```swift
protocol NotchModule: AnyObject, Identifiable {
    var id: String { get }              // 唯一識別，例如 "com.notchcat.music"
    var priority: Int { get }           // 多模組搶 Preview/Expanded 主控權時的排序依據

    // 模組是否有「環境常駐」內容要展示（例如正在播放音樂）
    // Shell 依此決定 Hidden ⇄ Preview 的切換
    var isActivePublisher: AnyPublisher<Bool, Never> { get }

    // 三種模式各自要畫什麼，回傳 AnyView（或 associatedtype View 版本）
    func hiddenAdornment() -> AnyView?   // 可選：Hidden 狀態下的小圖示/紅點
    func previewView() -> AnyView        // Preview 狀態下的內容
    func expandedView() -> AnyView       // Expanded 狀態下的內容

    // 生命週期
    func onInstall()
    func onStart()   // 使用者啟用
    func onStop()    // 使用者停用
    func onRemove()

    // 選用：模組自己的設定頁
    func settingsView() -> AnyView?
}
```

### 4.2 ModuleManager

- 持有所有已安裝模組的清單與啟用狀態。
- 訂閱每個模組的 `isActivePublisher`，決定「目前 Preview 的主人是誰」（多個 active 時依 priority 排序，並可能之後做輪播）。
- Expanded 狀態下：若有 active 模組，優先顯示該模組的 `expandedView()`；否則顯示 Shell 預設畫面（例如已安裝模組的橫向切換清單，讓使用者可以手動選一個模組看，或看時鐘）。
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
  Shell/               # 瀏海幾何偵測、NSPanel 管理、三態狀態機、動畫
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

1. **Phase 0 — 純骨架**：瀏海幾何偵測、`NSPanel` 建置、Hidden ⇄ Expanded 的 hover 展開收合動畫，內容先放假資料，不接任何模組。驗收：能在真機瀏海上看到收合/展開，動畫順暢。
2. **Phase 1 — 模組介面打底**：實作 `NotchModule` protocol、`ModuleManager`，用一個最簡單的「時鐘模組」（無 active 狀態，只在 Expanded 顯示時鐘）驗證整個安裝機制跑得通。
3. **Phase 2 — Music 模組**：串接 MediaRemote、做 Preview/Expanded 畫面、動態歌詞來源與同步。這階段獨立拆成子任務執行。
4. **Phase 3 — 擴充與體驗**：更多模組（電量、剪貼簿、日曆…）、模組管理/設定 UI、多模組搶 Preview 的排序體驗調整。
5. **Phase 4 — 發布**：簽章、公證、自動更新，（可選）評估是否開放社群寫模組。

## 9. 待決事項（需要你決定，暫不動工）

- 最低支援的 macOS 版本（14 vs 13）。
- v1 要不要一開始就做「模組開關」的使用者可見設定 UI，還是先寫死啟用清單。
- Music 模組的歌詞來源打算怎麼拿（純顯示系統提供的資訊，或額外串接歌詞服務）。
