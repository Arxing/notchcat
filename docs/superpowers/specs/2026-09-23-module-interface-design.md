# Module 溝通介面設計（Phase 1）

寫於 2026-09-23。目標：讓外部功能（音樂、電量、剪貼簿……）能透過一套統一介面「安裝」進 Shell，Shell 本身不認識任何具體模組。這是 `docs/PLANNING.md` 第 8 節「Phase 1」的落地設計，並更新了原草案裡的幾個決定（見下方「與 PLANNING.md 的差異」）。

## 背景 / 現況

Phase 0 已經完成瀏海骨架：`NSPanel` 疊在瀏海位置、hover 觸發展開收合動畫、內容是寫死的假資料（見 `Sources/NotchCat/Shell/`、`Sources/NotchCat/UI/NotchContentView.swift`）。目前狀態機是三態：一個完全待命、不含任何模組內容的狀態，加上 `Expanded` / `Preview`。

本次設計要做兩件事：
1. 把狀態機從三態簡化成兩態。
2. 在 Shell 之上蓋一層 `ModuleKit`，讓模組可以插進 Preview / Expanded 兩個畫面。

## 與 PLANNING.md 的差異

- **狀態機從三態改成兩態**：拿掉原本那個獨立的待命狀態，只剩 `Preview` / `Expanded`。它原本的視覺尺寸（貼合瀏海寬度 + 160pt、貼合瀏海高度，圓角 10pt）直接變成 Preview 沒有 active 模組時的預設外觀；原本 Preview 的獨立尺寸公式（`notchHeight + 8`）不再使用。程式碼、註解、變數命名一律不再出現舊狀態的名稱。
- **拿掉原本規劃給待命狀態疊加小圖示/紅點的介面方法**：三態簡化成兩態後，這個介面失去存在理由（Preview 本身就是模組的收起態內容）。
- **新增 Expanded 的模組切換 AppBar**：這是 PLANNING.md 只提過一句「例如已安裝模組的橫向切換清單」的功能，這次確定要做，並定義了清楚的行為（見下方）。
- **狀態管理維持 Combine**：不改用 macOS 14+ 的 Observation framework，理由是 Shell 現有程式碼（`NotchShellState`）已經用 `@Published`/`ObservableObject`，專案目前規模小，混用兩套機制增加複雜度大於效能收益。

## 狀態機

`NotchMode` 從三個 case 簡化成兩個：

```swift
enum NotchMode: Equatable {
    case preview
    case expanded
}
```

- **Preview**：預設狀態（滑鼠不在瀏海上）。顯示 `ModuleManager.activeModule?.previewView()`；沒有 active 模組就顯示 Shell 自己的預設內容。尺寸沿用現有 Phase 0 已經調校過的數字（瀏海寬度 + 160pt、瀏海高度、下圓角 10pt、上直角）。
- **Expanded**：hover 觸發。畫面 = 一條 AppBar（列出所有已安裝模組的 `tabIcon()`，可點擊切換）+ 下方 `ModuleManager.selectedModule?.expandedView()`；沒有選中模組就顯示 Shell 預設內容。尺寸沿用現有 Expanded 數字（同寬、180pt 高、下圓角 24pt）。

Hover / hit-test 機制（`NotchHostingView.hoverRect`、進入用精準小範圍、展開後放寬成整個固定容器、離開用 150ms debounce 防抖）**不變**，只是把原本掛在三態上的邏輯改成掛在兩態上——這部分是既有 Phase 0 程式碼的直接簡化，不是新設計。

## NotchModule 協定

```swift
protocol NotchModule: AnyObject, Identifiable {
    var id: String { get }              // 唯一識別，例如 "com.notchcat.clock"
    var priority: Int { get }           // 多模組同時 active 時，數字大的贏

    // 模組是否有「環境常駐」內容要展示（例如正在播放音樂）。
    // ModuleManager 依此 + priority 決定 activeModule。
    var isActivePublisher: AnyPublisher<Bool, Never> { get }

    func tabIcon() -> AnyView          // Expanded AppBar 上的圖示，模組自己畫
    func previewView() -> AnyView      // Preview 內容
    func expandedView() -> AnyView     // Expanded 內容（被選中時才顯示）

    // 生命週期。v1 沒有動態安裝/移除 UI，onInstall/onStart 在註冊當下就會依序呼叫。
    func onInstall()
    func onStart()
    func onStop()
    func onRemove()

    // 選用：模組自己的設定頁，v1 沒有地方顯示它（見「範圍外」），先定義介面。
    func settingsView() -> AnyView?
}
```

## ModuleManager

新的 `ObservableObject`，Shell 唯一認識的入口：

```swift
final class ModuleManager: ObservableObject {
    @Published private(set) var installedModules: [NotchModule] = []
    @Published private(set) var activeModule: NotchModule?      // 依 isActive + priority 自動算出
    @Published var selectedModule: NotchModule?                 // Expanded AppBar 目前選中誰

    func register(_ module: NotchModule) { ... }                // v1 的「安裝」：呼叫 onInstall() + onStart()
    func select(_ module: NotchModule) { selectedModule = module }
}
```

行為：
- `register(_:)` 把模組加進 `installedModules`，呼叫 `onInstall()` 再呼叫 `onStart()`（v1 沒有使用者可見的啟用/停用開關，註冊 = 啟用，沿用 PLANNING.md 待決事項裡「先寫死啟用清單」的預設）。
- 對每個模組的 `isActivePublisher` 訂閱變化，重新計算 `activeModule` = 目前回報 `true` 的模組中 `priority` 最高者，都沒有就是 `nil`。
- `selectedModule` **不**由 `ModuleManager` 自動維護的持續性狀態決定顯示時機；由 `NotchShellController` 在「Preview → Expanded」轉場的當下（滑鼠 hover 進入、真正呼叫 `transition(to: .expanded)` 之前）設成當前的 `activeModule`。之後在同一次展開期間，AppBar 點擊會呼叫 `select(_:)` 覆蓋它。下次收合再展開，會被再次重置成當下的 `activeModule`（不記憶使用者上次手動選的）。

## 畫面組裝

- `Sources/NotchCat/UI/NotchContentView.swift` 改成依 `mode` 讀 `ModuleManager`：
  - `.preview` → `moduleManager.activeModule?.previewView() ?? DefaultShellContent(mode: .preview)`
  - `.expanded` → `ModuleAppBar(modules: moduleManager.installedModules, selected: moduleManager.selectedModule, onSelect: moduleManager.select) ` 疊在上方 + `moduleManager.selectedModule?.expandedView() ?? DefaultShellContent(mode: .expanded)`
- `DefaultShellContent`：把現有 Phase 0 的假內容（黑色形狀 + `"NotchCat — Phase 0..."` 文字）抽成這個名字的 view，當作「沒有模組時」的 fallback，同時也是 App 一啟動、還沒有任何模組回報 active 時看到的畫面。
- `ModuleAppBar`：水平排列 `installedModules` 的 `tabIcon()`，點擊呼叫 `moduleManager.select(module)`。純 Shell/ModuleKit 層的元件，模組不知道它的存在，只需要提供 `tabIcon()`。

## 專案結構異動

沿用 PLANNING.md 第 7 節的建議結構：

```
Sources/NotchCat/
  Shell/            既有，NotchMode 從三態簡化成兩態
  UI/               既有，NotchContentView 改讀 ModuleManager；新增 DefaultShellContent、ModuleAppBar
  ModuleKit/        新增：NotchModule.swift（協定）、ModuleManager.swift
  Modules/
    Clock/          新增：ClockModule.swift（Phase 1 驗證用）
    Placeholder/    新增：PlaceholderModule.swift（Phase 1 驗證用，見下）
```

## Phase 1 驗證用模組

一個模組不足以驗證「切換」這件事，所以寫兩個最小可行模組：

1. **ClockModule**：`isActivePublisher` 固定回報 `true`（永遠 active，證明整條 register → activeModule → 畫面渲染的管線真的通）。`previewView()` 顯示縮寫時間文字；`expandedView()` 顯示大一點的即時時鐘。`priority` 設高一點，預設會是 `activeModule`。
2. **PlaceholderModule**：`isActivePublisher` 固定回報 `false`（永遠不會自動搶到 `activeModule`，但仍會出現在 AppBar 上，可以手動點選）。用來驗證：非 active 的模組依然「已安裝」、`tabIcon()`/`expandedView()` 正常運作、手動選取會覆蓋 active 的預設選擇、重新展開會被重置回 Clock。

兩個都在 `main.swift`（或新的一個 App 層小函式）啟動時手動 `moduleManager.register(...)`，符合 v1「編譯進同一個 App、啟動時註冊」的決定。

## 測試方式

專案目前（Phase 0）沒有自動化測試 target（`Package.swift` 只有一個 executable target），Phase 1 延續這個做法，不新增測試框架。驗收方式維持手動：`swift run` 在真機瀏海上確認——
- 啟動時 Preview/Expanded 顯示 Clock 內容（因為它永遠 active）。
- Expanded 的 AppBar 能看到兩個模組圖示。
- 點 Placeholder 圖示會切到它的 `expandedView()`；收合再展開會重置回 Clock。

## 範圍外（明確不做）

- 動態外掛載入（外部 `.bundle`/App Extension）——沿用 PLANNING.md 決定，v1 不做。
- 模組啟用/停用的使用者可見設定 UI、`settingsView()` 實際顯示的位置——協定先定義好，UI 留到之後（PLANNING.md 第 9 節待決事項，本次不解決）。
- Music 模組、`MediaRemote` 串接——這是 Phase 2 的範圍。
