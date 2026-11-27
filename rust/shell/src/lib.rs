//! musicat-shell：GPUI 外殼。
//!
//! - [`ShellContent`]：外殼要畫的內容（Preview／Expanded 兩種畫面）與它需要的回呼。Task 24 的音樂畫面實作它。
//! - [`ShellWindow`]（僅 macOS）：固定大小的瀏海面板視窗，內含 hover 狀態機、0.28 秒的形狀動畫與點擊穿透。
//! - [`MusicShellContent`]：音樂模組的 Preview／Expanded 畫面（Task 24），版面常數與純運算在 [`music_layout`]，
//!   跑馬燈在 [`marquee`]，封面解碼與快取在 [`artwork`]。
//! - [`GpuiClock`]／[`GpuiSpawner`]：core 的 `Clock`／`Spawner` 以 GPUI 背景執行器實作（[`runtime`]）。
//!
//! 外殼本身不碰 Objective-C：視窗繞路全部經 `musicat_platform_macos::panel` 的公開函式。

use gpui::{AnyElement, App, Window};

pub mod artwork;
pub mod marquee;
pub mod music_layout;
mod music_views;
pub mod runtime;

pub use music_views::MusicShellContent;
pub use runtime::{GpuiClock, GpuiSpawner};

#[cfg(target_os = "macos")]
mod window;

#[cfg(target_os = "macos")]
pub use window::ShellWindow;

/// Expanded 內容槽上方的留白（Swift `NotchContentView` 的 `.padding(.top, 8)`）。
pub const EXPANDED_TOP_PADDING: f64 = 8.0;

/// 外殼通知內容（或 app）的狀態轉移，給日誌與診斷用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellEvent {
    /// 剛從 Preview 展開（緊接在 [`ShellContent::on_expand`] 之後送出）。
    Expanded,
    /// 剛收合回 Preview（離開 150 ms 之後，或因為隱藏而強制收合）。
    Collapsed,
    /// 面板實際的顯示狀態改變（`set_visible` 與全螢幕前景合併後的結果）。
    Visible(bool),
}

/// 外殼要畫的內容。外殼負責外形（黑底、上方直角、下方圓角、0.28 秒的大小動畫）、hover 與視窗；
/// 內容只負責外形「裡面」的東西。
///
/// # 給 Task 24 的使用說明
///
/// - 物件安全：外殼以 `Arc<dyn ShellContent>` 持有，所有方法都在 GPUI 的主執行緒上呼叫。
/// - [`preview`](Self::preview)／[`expanded`](Self::expanded) 在外殼每次重繪時呼叫（只有大小、模式或可見性
///   改變時才重繪，閒置時不會被呼叫）。回傳的元素放在一個**固定大小**的槽裡：Preview 槽 = Preview 的大小
///   （瀏海寬 + 260，瀏海高 + 額外高度），Expanded 槽 = Expanded 的大小（同寬，高 180，上方留 8 點，同 Swift
///   `NotchContentView`）。槽從外形的左上角開始、不隨動畫改變大小，所以內容不會在動畫中重新排版；
///   外形比槽小的部分被裁掉。模式一改變就換成另一個模式的內容（同 Swift），形狀再花 0.28 秒長大／縮小。
/// - 內容自己的狀態改變（例如換歌）要重繪時：讓元素來自內容自己的 `Entity<V: Render>`
///   （`entity.clone().into_any_element()`），在該 entity 上 `cx.notify()`，GPUI 只重繪那個 view，
///   不必經過外殼。
/// - [`preview_extra_height`](Self::preview_extra_height)：active 模組要的 Preview 額外高度
///   （`ModuleManager::preview_extra_height()`）。外殼每一拍（16–40 ms）讀一次並夾到 `0..=56`（`MAX_PREVIEW_EXTRA_HEIGHT`）；
///   改變時 Preview 的高度與 hover 範圍跟著以同樣的動畫改變。
/// - [`on_expand`](Self::on_expand)：每次**新**展開呼叫一次（`HoverOutput::just_expanded`）。
///   app 在這裡呼叫 `ModuleManager::reset_selection_to_active()`（spec 5.3）。在外殼的輪詢更新中途呼叫，
///   沒有 GPUI context；可以呼叫 [`ShellWindow`] 的方法（它們只設旗標）。
/// - Preview 有額外高度（> 0）時，外殼的黑底只蓋瀏海高度（下方直角），額外高度那一段**透明**：
///   內容自己畫那一段的背景（例如歌詞條的 0.6 不透明黑底，同 Swift `NotchContentView`）。
///   額外高度出現／消失時，外框、黑底與圓角一起以 0.28 秒動畫連續變化。
/// - 裁切的限制（GPUI 0.2.2）：`overflow_hidden` 只能以**矩形**裁切，不會依下方圓角裁切。
///   **Expanded 內容在下方兩角至少留 24 點（`EXPANDED_CORNER_RADIUS`）的空白**（Preview 為 10 點），
///   否則會露出圓角外。Swift 版本來就不裁切內容，所以這與 Swift 一致。
/// - 主執行緒以外的重繪：見 [`ShellWindow::refresh`] 的說明（建議用內容自己的 entity）。
pub trait ShellContent: 'static {
    /// Preview（收合）時的內容。
    fn preview(&self, window: &mut Window, cx: &mut App) -> AnyElement;

    /// Expanded（展開）時的內容。
    fn expanded(&self, window: &mut Window, cx: &mut App) -> AnyElement;

    /// active 模組要求的 Preview 額外高度（點）；外殼夾到 `0..=56`，NaN 視為 0。
    fn preview_extra_height(&self) -> f64;

    /// 每次新展開時呼叫一次。
    fn on_expand(&self);

    /// 狀態轉移通知（預設不做事）。示範程式用它印日誌。
    fn on_shell_event(&self, event: ShellEvent) {
        let _ = event;
    }
}
