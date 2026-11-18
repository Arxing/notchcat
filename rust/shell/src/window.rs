//! 瀏海面板視窗：一個固定大小的 GPUI 視窗（= `container_frame`），裡面畫一個頂端置中、會做動畫的黑色外形。
//!
//! # 驅動方式
//!
//! 視窗本身沒有任何滑鼠追蹤：一個在主執行緒上的輪詢工作每一拍讀全域游標（`panel::mouse_location`），
//! 換成容器座標後餵給 core 的 [`HoverMachine`]。節拍是自適應的（[`driver::next_tick`]）：
//! 動畫中、游標在容器附近（外擴 80 點）或待收合時 16 ms，其餘 40 ms；待收合時一定在 `next_deadline`
//! 準時醒來再呼叫一次 `update`（HoverMachine 先判期限再判 hover，只靠逐格取樣會看到「同一次更新收合又展開」）。
//!
//! 形狀的大小由 core 的 [`Animator`] 算：模式（或 Preview 額外高度）一改變就 `cx.notify()` 一次，
//! 之後 `render` 在動畫進行中每格 `request_animation_frame()`，動畫結束就停。所以閒置時既不重繪、
//! 輪詢也只是 40 ms 一次的游標讀取。
//!
//! # 點擊穿透
//!
//! 容器（例如 345 × 180）比 Preview 長條高得多。GPUI 的透明視窗仍會吃下透明區域的點擊，所以 Preview 或隱藏時
//! 讓面板整個忽略滑鼠（`set_ignores_mouse_events(true)`），只有 Expanded（有按鈕）時接收。
//! 展開只靠輪詢游標位置，不需要滑鼠事件。
//!
//! # NativeWindow 把手的生命週期
//!
//! 把手只在 `open_window` 的建構回呼裡、經 `unsafe NativeWindow::from_ns_view` 建立一次，存進
//! `Shared`（[`ShellWindow`] 與根 view 共用）。外殼從不關閉這個視窗（無邊框面板也沒有關閉按鈕），
//! 視窗一直活到行程結束；萬一 GPUI 關掉了視窗，根 view 被釋放時（`Drop for ShellView`）會先把把手清掉，
//! 之後 [`ShellWindow`] 的方法看不到把手就什麼都不做，所以把手不會在視窗消失後被使用。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions,
};
use musicat_core::shell_logic::driver::{self, TickState};
use musicat_core::shell_logic::{
    container_frame, container_size, corner_radius_for_height, local_rect, mode_size, Animator,
    HoverInput, HoverMachine, NotchGeometry, Rect, ShellMode, Size, EXPANDED_HEIGHT,
};
use musicat_platform_macos::panel::{self, NativeWindow};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{ShellContent, ShellEvent};

/// Expanded 內容槽上方的留白（Swift `NotchContentView` 的 `.padding(.top, 8)`）。
pub const EXPANDED_TOP_PADDING: f64 = 8.0;

/// 多久重新問一次 `foreground_is_fullscreen()`（秒）：它之後可能要查 CGWindowList，不必每拍查。
const FULLSCREEN_POLL_SECONDS: f64 = 1.0;

/// `ShellWindow` 與根 view 共用的狀態（都在主執行緒，所以用 `Rc<RefCell>`）。
/// 借用一律短暫，絕不在借用中呼叫 `ShellContent` 的方法。
struct Shared {
    geometry: NotchGeometry,
    /// 面板的原生把手；視窗消失（根 view 釋放）後為 `None`。
    native: Option<NativeWindow>,
    /// `set_visible` 設定的值。
    user_visible: bool,
    /// `reposition` 之後設為 true，下一拍由 view 套用（形狀直接跳到新大小、重新判 hover）。
    geometry_changed: bool,
}

/// 開啟中的瀏海面板。只能在主執行緒使用（GPUI 的 `App` 回呼裡）。
pub struct ShellWindow {
    shared: Rc<RefCell<Shared>>,
}

impl ShellWindow {
    /// 建立面板：一個大小固定為 `container_frame(geometry)` 的 GPUI 視窗，經 `panel::configure_panel`
    /// 改成瀏海面板（無邊框、不搶焦點、高於選單列、跨所有空間），並開始輪詢 hover。
    ///
    /// 呼叫前提（對應 GPUI 0.2.2，見 `musicat_platform_macos::panel`）：`Application::new()` 之後已呼叫
    /// `panel::patch_gpui_panel_class()`，且在 `Application::run` 的回呼裡已呼叫
    /// `panel::set_accessory_activation_policy()`。
    pub fn open(
        cx: &mut App,
        geometry: NotchGeometry,
        content: Arc<dyn ShellContent>,
    ) -> gpui::Result<ShellWindow> {
        let frame = container_frame(&geometry);
        let shared = Rc::new(RefCell::new(Shared {
            geometry,
            native: None,
            user_visible: true,
            geometry_changed: false,
        }));

        // GPUI 的初始位置只是大概（以顯示器為準的座標）；真正的外框由 configure_panel 設定。
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: gpui::point(px(0.), px(0.)),
                size: size(px(frame.w as f32), px(frame.h as f32)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };

        let view_shared = shared.clone();
        cx.open_window(options, move |window, cx| {
            let native = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                // SAFETY: `ns_view` 是 GPUI 剛建立、此刻正在建構回呼中的視窗的 NSView，所以現在是存活的。
                // 把手只存在 `Shared` 裡：外殼從不關閉這個視窗（它活到行程結束）；若 GPUI 仍關掉了它，
                // 根 view 釋放時 `Drop for ShellView` 先清掉把手，之後不會再被使用（見模組文件）。
                // 所有使用都在 GPUI 的主執行緒上。
                Ok(RawWindowHandle::AppKit(h)) => {
                    Some(unsafe { NativeWindow::from_ns_view(h.ns_view.as_ptr()) })
                }
                other => {
                    eprintln!(
                        "musicat-shell: expected an AppKit window handle, got {other:?}; \
                         the panel cannot be configured"
                    );
                    None
                }
            };
            if let Some(native) = native {
                panel::configure_panel(native, frame);
            }
            view_shared.borrow_mut().native = native;
            cx.new(|cx| ShellView::new(view_shared, content, cx))
        })?;
        Ok(ShellWindow { shared })
    }

    /// 換螢幕：把面板移到新幾何的 `container_frame`。外框終身不變，只有這裡會移動它；
    /// 新舊螢幕的瀏海一樣大時只是移動，不一樣大時容器大小跟著新瀏海（外框仍只在換螢幕時改變）。
    /// 形狀直接跳到新位置的大小（不做動畫），下一拍起以新幾何判 hover。
    pub fn reposition(&self, geometry: NotchGeometry) {
        let native = {
            let mut s = self.shared.borrow_mut();
            s.geometry = geometry;
            s.geometry_changed = true;
            s.native
        };
        if let Some(native) = native {
            panel::set_panel_frame(native, container_frame(&geometry));
        }
    }

    /// 顯示或隱藏面板（例如使用者在選單關掉瀏海式外殼）。隱藏時強制收合、不展開、點擊穿透；
    /// 實際的顯示狀態另外也受全螢幕前景影響。下一拍（≤ 40 ms）生效。
    pub fn set_visible(&self, visible: bool) {
        self.shared.borrow_mut().user_visible = visible;
    }
}

/// 根 view：持有 hover 狀態機、動畫器與內容。
struct ShellView {
    shared: Rc<RefCell<Shared>>,
    content: Arc<dyn ShellContent>,
    hover: HoverMachine,
    mode: ShellMode,
    extra: f64,
    geometry: NotchGeometry,
    animator: Animator,
    /// 已套用到視窗的顯示狀態／點擊穿透（`None` = 尚未套用）。
    applied_visible: Option<bool>,
    applied_ignore: Option<bool>,
    fullscreen: bool,
    fullscreen_checked_at: Option<f64>,
    t0: Instant,
    trace: bool,
}

impl Drop for ShellView {
    fn drop(&mut self) {
        // 視窗消失：之後 ShellWindow 不得再使用原生把手。
        if let Ok(mut s) = self.shared.try_borrow_mut() {
            s.native = None;
        }
    }
}

fn epoch() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

impl ShellView {
    fn new(
        shared: Rc<RefCell<Shared>>,
        content: Arc<dyn ShellContent>,
        cx: &mut Context<Self>,
    ) -> Self {
        let geometry = shared.borrow().geometry;
        let extra = content.preview_extra_height();
        let initial = mode_size(ShellMode::Preview, &geometry, extra);

        // 輪詢工作：每拍之後回傳下一拍的等待時間。view 消失時 update 失敗、工作結束。
        cx.spawn(async move |this, cx| loop {
            let Ok(wait) = this.update(cx, |v, cx| v.tick(cx)) else {
                break;
            };
            cx.background_executor()
                .timer(Duration::from_secs_f64(wait))
                .await;
        })
        .detach();

        Self {
            shared,
            content,
            hover: HoverMachine::new(),
            mode: ShellMode::Preview,
            extra,
            geometry,
            animator: Animator::new(initial),
            applied_visible: None,
            applied_ignore: None,
            fullscreen: false,
            fullscreen_checked_at: None,
            t0: Instant::now(),
            trace: std::env::var("MUSICAT_SHELL_TRACE").as_deref() == Ok("1"),
        }
    }

    fn now(&self) -> f64 {
        self.t0.elapsed().as_secs_f64()
    }

    fn target_size(&self) -> Size {
        mode_size(self.mode, &self.geometry, self.extra)
    }

    /// 一拍：讀游標、餵 HoverMachine、套用模式／顯示／點擊穿透；回傳下一拍的等待秒數。
    fn tick(&mut self, cx: &mut Context<Self>) -> f64 {
        let now = self.now();
        let (geometry, native, user_visible, geometry_changed) = {
            let mut s = self.shared.borrow_mut();
            let changed = std::mem::take(&mut s.geometry_changed);
            (s.geometry, s.native, s.user_visible, changed)
        };
        let mut redraw = false;

        if geometry_changed {
            self.geometry = geometry;
            self.animator.snap_to(self.target_size());
            redraw = true;
        }

        let extra = self.content.preview_extra_height();
        if extra != self.extra && !(extra.is_nan() && self.extra.is_nan()) {
            self.extra = extra;
            redraw = true;
        }

        if self
            .fullscreen_checked_at
            .is_none_or(|t| now - t >= FULLSCREEN_POLL_SECONDS)
        {
            self.fullscreen = panel::foreground_is_fullscreen();
            self.fullscreen_checked_at = Some(now);
        }

        let frame = container_frame(&self.geometry);
        let mouse = panel::mouse_location();
        let out = self.hover.update(HoverInput {
            now,
            mouse: Some(driver::to_local(mouse, frame)),
            preview_rect: local_rect(ShellMode::Preview, &self.geometry, self.extra),
            container: Rect {
                x: 0.0,
                y: 0.0,
                w: container_size(&self.geometry).w,
                h: container_size(&self.geometry).h,
            },
            // 使用者隱藏與全螢幕前景同樣處理：強制 Preview、不可見、不展開。
            fullscreen_foreground: self.fullscreen || !user_visible,
        });

        if self.trace && (out.mode != self.mode || out.next_deadline.is_some()) {
            println!(
                "[{}] shell-tick mode={:?}->{:?} mouse_global=({:.1}, {:.1}) deadline={:?}",
                epoch(),
                self.mode,
                out.mode,
                mouse.x,
                mouse.y,
                out.next_deadline
            );
        }
        if out.mode != self.mode {
            self.mode = out.mode;
            redraw = true;
            if out.mode == ShellMode::Preview {
                self.content.on_shell_event(ShellEvent::Collapsed);
            }
        }
        if out.just_expanded {
            self.content.on_expand();
            self.content.on_shell_event(ShellEvent::Expanded);
        }

        if self.applied_visible != Some(out.visible) {
            if let Some(native) = native {
                panel::set_panel_visible(native, out.visible);
            }
            self.applied_visible = Some(out.visible);
            self.content
                .on_shell_event(ShellEvent::Visible(out.visible));
            redraw = true;
        }

        let ignore = driver::ignores_mouse(out.mode, out.visible);
        if self.applied_ignore != Some(ignore) {
            if let Some(native) = native {
                panel::set_ignores_mouse_events(native, ignore);
            }
            self.applied_ignore = Some(ignore);
        }

        // 模式、額外高度、幾何或顯示改變時重繪一次；之後 render 自己逐格請求，直到動畫結束。
        let target = self.target_size();
        if redraw {
            cx.notify();
        }

        driver::next_tick(TickState {
            now: self.now(),
            animating: self.animator.is_animating() || target != self.animator.current(),
            mouse_near: driver::inflate(frame, driver::NEAR_MARGIN).contains(mouse),
            deadline: out.next_deadline,
        })
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = self.now();
        let target = self.target_size();
        let drawn = self.animator.update(now, target);
        let preview_h = mode_size(ShellMode::Preview, &self.geometry, self.extra).h;
        let radius = corner_radius_for_height(drawn.h, preview_h, EXPANDED_HEIGHT, self.mode);
        if self.animator.is_animating() {
            window.request_animation_frame();
        }
        if self.trace {
            println!(
                "[{}] shell-frame mode={:?} h={:.1} r={:.1} animating={}",
                epoch(),
                self.mode,
                drawn.h,
                radius,
                self.animator.is_animating()
            );
        }

        // 內容槽：該模式的完整大小、從外形左上角開始，不隨動畫改變（避免重新排版）；外形把多的部分裁掉。
        let slot = match self.mode {
            ShellMode::Preview => div()
                .flex_none()
                .w(px(target.w as f32))
                .h(px(target.h as f32))
                .child(self.content.preview(window, cx)),
            ShellMode::Expanded => div()
                .flex_none()
                .w(px(target.w as f32))
                .h(px(target.h as f32))
                .pt(px(EXPANDED_TOP_PADDING as f32))
                .child(self.content.expanded(window, cx)),
        };

        let r = px(radius as f32);
        let shape = div()
            .flex_none()
            .w(px(drawn.w as f32))
            .h(px(drawn.h as f32))
            .bg(rgb(0x000000))
            // 上方兩角直角（貼齊螢幕頂端與瀏海），下方圓角（spec 5.3）。
            .rounded_bl(r)
            .rounded_br(r)
            // GPUI 0.2.2 只能以矩形裁切（不含圓角），見 ShellContent 的文件。
            .overflow_hidden()
            .child(slot);

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .child(shape)
    }
}
