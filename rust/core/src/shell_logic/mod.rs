//! 外殼（瀏海面板）的純邏輯：幾何、螢幕選擇、虛擬瀏海。
//!
//! 座標一律以螢幕左上角為原點、y 向下；平台層（macOS 的 AppKit 是左下原點）負責轉換。
//! 這裡不碰任何作業系統 API，所以 Windows CI 也跑得了這些測試。

pub mod geometry;
pub mod hover;
pub mod screens;

pub use geometry::{
    container_frame, container_size, local_rect, mode_size, NotchGeometry, Point, Rect, ShellMode,
    Size, EXPANDED_CORNER_RADIUS, EXPANDED_HEIGHT, PREVIEW_CORNER_RADIUS, WIDTH_PADDING,
};
pub use hover::{HoverInput, HoverMachine, HoverOutput, LEAVE_DELAY};
pub use screens::{choose_screen, geometry_for, ScreenInfo, VirtualNotch};
