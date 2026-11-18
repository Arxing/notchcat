//! musicat-shell：GPUI 外殼。
//!
//! - [`ShellContent`]：外殼要畫的內容（Preview／Expanded 兩種畫面）與它需要的回呼。Task 24 的音樂畫面實作它。
//! - [`ShellWindow`]（僅 macOS）：固定大小的瀏海面板視窗，內含 hover 狀態機、0.28 秒的形狀動畫與點擊穿透。
//!
//! 外殼本身不碰 Objective-C：視窗繞路全部經 `musicat_platform_macos::panel` 的公開函式。

use gpui::{AnyElement, App, Window};

#[cfg(target_os = "macos")]
mod window;

#[cfg(target_os = "macos")]
pub use window::ShellWindow;

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
///   （瀏海寬 + 160，瀏海高 + 額外高度），Expanded 槽 = Expanded 的大小（同寬，高 180，上方留 8 點，同 Swift
///   `NotchContentView`）。槽從外形的左上角開始、不隨動畫改變大小，所以內容不會在動畫中重新排版；
///   外形比槽小的部分被裁掉。模式一改變就換成另一個模式的內容（同 Swift），形狀再花 0.28 秒長大／縮小。
/// - 內容自己的狀態改變（例如換歌）要重繪時：讓元素來自內容自己的 `Entity<V: Render>`
///   （`entity.clone().into_any_element()`），在該 entity 上 `cx.notify()`，GPUI 只重繪那個 view，
///   不必經過外殼。
/// - [`preview_extra_height`](Self::preview_extra_height)：active 模組要的 Preview 額外高度
///   （`ModuleManager::preview_extra_height()`）。外殼每一拍（16–40 ms）讀一次並夾到 `0..=28`；
///   改變時 Preview 的高度與 hover 範圍跟著以同樣的動畫改變。
/// - [`on_expand`](Self::on_expand)：每次**新**展開呼叫一次（`HoverOutput::just_expanded`）。
///   app 在這裡呼叫 `ModuleManager::reset_selection_to_active()`（spec 5.3）。在外殼的更新中途呼叫，
///   不要在裡面同步呼叫 [`ShellWindow`] 的方法以外的 GPUI 重入操作。
/// - 裁切的限制（GPUI 0.2.2）：`overflow_hidden` 只能以**矩形**裁切，不會依下方圓角裁切。內容若畫進
///   外形下方兩角（Preview 10 點、Expanded 24 點的圓角範圍）會露出圓角外。內容請在下方兩角留白，
///   或自己用圓角背景（Swift 版本來就不裁切內容，所以這與 Swift 一致）。
pub trait ShellContent: 'static {
    /// Preview（收合）時的內容。
    fn preview(&self, window: &mut Window, cx: &mut App) -> AnyElement;

    /// Expanded（展開）時的內容。
    fn expanded(&self, window: &mut Window, cx: &mut App) -> AnyElement;

    /// active 模組要求的 Preview 額外高度（點）；外殼夾到 `0..=28`，NaN 視為 0。
    fn preview_extra_height(&self) -> f64;

    /// 每次新展開時呼叫一次。
    fn on_expand(&self);

    /// 狀態轉移通知（預設不做事）。示範程式用它印日誌。
    fn on_shell_event(&self, event: ShellEvent) {
        let _ = event;
    }
}
