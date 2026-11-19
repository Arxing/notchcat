//! 跑馬燈：放不下時往左循環捲動的單行文字（移植自 Swift `MarqueeText.swift`）。
//!
//! - [`MarqueeLayout`]：純規則（何時捲動、一圈多久、某一刻的位移），與 GPUI 無關，可單元測試。
//! - [`marquee`]：GPUI 元素。以 GPUI 的文字系統量出文字的自然寬度；放得下就置中靜止，放不下就以
//!   每秒 30 點往左捲、畫兩份（中間隔 32 點）無縫循環。元素寬度就是父元素給的寬度（`w_full`），
//!   從不要求更多，所以很長的曲名撐不開版面（同 Swift）。
//!
//! 重繪：只有在捲動時，元素在 paint 裡 `request_animation_frame()`（通知它所在的 view 下一格再畫）；
//! 靜止的跑馬燈不請求任何畫格。外殼隱藏時 GPUI 不畫，請求也就自然停止。
//! 捲動的起點記在 [`MarqueeState`]：文字一改變就從頭開始（Swift 以 `.id(text|width)` 重建）。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    canvas, font, point, prelude::*, px, App, Bounds, ContentMask, FontWeight, Hsla, Pixels,
    SharedString, TextRun, Window,
};

/// 跑馬燈的純規則（Swift `MarqueeLayout`）。
pub struct MarqueeLayout;

impl MarqueeLayout {
    /// 捲動速度（點／秒）。
    pub const POINTS_PER_SECOND: f64 = 30.0;
    /// 文字結尾與下一份開頭之間的空白（點）。
    pub const GAP: f64 = 32.0;

    /// 文字比容器寬才捲動。容器還沒有寬度（第一次排版前）時永不捲動。
    pub fn needs_scroll(text_width: f64, container_width: f64) -> bool {
        // 無限大的文字寬度不是真的量測結果：不捲動（否則一圈無限長）。
        text_width.is_finite() && container_width > 0.0 && text_width > container_width
    }

    /// 一整圈的秒數。
    pub fn loop_duration(text_width: f64) -> f64 {
        (sanitized(text_width) + Self::GAP) / Self::POINTS_PER_SECOND
    }

    /// 經過 `elapsed` 秒時往左的位移（點），在 `0..text_width + GAP` 之間循環。
    pub fn offset(text_width: f64, elapsed: f64) -> f64 {
        if !(elapsed.is_finite() && elapsed > 0.0 && text_width.is_finite()) {
            return 0.0;
        }
        (elapsed * Self::POINTS_PER_SECOND) % (sanitized(text_width) + Self::GAP)
    }
}

/// 一個跑馬燈位置的捲動狀態（目前文字與開始捲動的時刻）。複製的把手共用同一份狀態。
#[derive(Clone, Default)]
pub struct MarqueeState {
    inner: Rc<RefCell<Option<(SharedString, Instant)>>>,
}

impl MarqueeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 下次畫時從頭開始（例如畫面重新出現時，同 Swift 的 `onAppear`）。
    pub fn reset(&self) {
        self.inner.borrow_mut().take();
    }

    /// `text` 已捲動的秒數；文字和上次不同時從 0 重新開始。
    fn elapsed(&self, text: &SharedString) -> f64 {
        let mut slot = self.inner.borrow_mut();
        match &*slot {
            Some((t, start)) if t == text => start.elapsed().as_secs_f64(),
            _ => {
                *slot = Some((text.clone(), Instant::now()));
                0.0
            }
        }
    }
}

/// 跑馬燈文字的外觀。
#[derive(Clone, Copy, Debug)]
pub struct MarqueeStyle {
    pub font_size: f32,
    pub weight: FontWeight,
    pub color: Hsla,
    /// 元素高度（行高）。
    pub line_height: f32,
}

/// 系統 UI 字型（macOS 為 SF，含中日韓文字的系統後備字型）。
pub const SYSTEM_FONT: &str = ".SystemUIFont";

/// 單行跑馬燈元素：寬度 = 父元素給的寬度，高度 = `style.line_height`。換行字元被換成空白。
pub fn marquee(
    state: &MarqueeState,
    text: impl Into<SharedString>,
    style: MarqueeStyle,
) -> impl IntoElement {
    let text: SharedString = text.into();
    let text: SharedString = if text.contains(['\n', '\r']) {
        text.replace(['\n', '\r'], " ").into()
    } else {
        text
    };
    let state = state.clone();
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, cx: &mut App| {
            paint_marquee(bounds, &state, text, style, window, cx);
        },
    )
    .w_full()
    .min_w_0()
    .h(px(style.line_height))
    .flex_none()
}

fn paint_marquee(
    bounds: Bounds<Pixels>,
    state: &MarqueeState,
    text: SharedString,
    style: MarqueeStyle,
    window: &mut Window,
    cx: &mut App,
) {
    let mut f = font(SYSTEM_FONT);
    f.weight = style.weight;
    let run = TextRun {
        len: text.len(),
        font: f,
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.clone(), px(style.font_size), &[run], None);
    let text_width = f64::from(f32::from(line.width));
    let container = f64::from(f32::from(bounds.size.width));
    let line_height = px(style.line_height);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        if MarqueeLayout::needs_scroll(text_width, container) {
            let offset = MarqueeLayout::offset(text_width, state.elapsed(&text)) as f32;
            let x = bounds.origin.x - px(offset);
            let _ = line.paint(point(x, bounds.origin.y), line_height, window, cx);
            let x2 = x + px((text_width + MarqueeLayout::GAP) as f32);
            let _ = line.paint(point(x2, bounds.origin.y), line_height, window, cx);
            // 只在捲動時要求下一格。
            window.request_animation_frame();
        } else {
            // 放得下：置中、靜止；文字改變時仍記下起點，之後若變得放不下就從頭捲。
            let _ = state.elapsed(&text);
            let x = bounds.origin.x + px(((container - text_width) / 2.0).max(0.0) as f32);
            let _ = line.paint(point(x, bounds.origin.y), line_height, window, cx);
        }
    });
}

/// 有限且 >= 0；其餘當 0（一圈至少是 `GAP`，永遠有限且大於 0）。
fn sanitized(width: f64) -> f64 {
    if width.is_finite() && width > 0.0 {
        width
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 移植自 Swift MarqueeTextTests（4 案） ----

    #[test]
    fn test_short_text_does_not_scroll() {
        assert!(!MarqueeLayout::needs_scroll(80.0, 120.0));
    }

    #[test]
    fn test_long_text_scrolls() {
        assert!(MarqueeLayout::needs_scroll(300.0, 120.0));
    }

    #[test]
    fn test_exact_fit_does_not_scroll() {
        assert!(!MarqueeLayout::needs_scroll(120.0, 120.0));
    }

    #[test]
    fn test_zero_width_container_does_not_crash() {
        // 排版前容器還沒有寬度；那時捲動只是白忙，必須靜止。
        assert!(!MarqueeLayout::needs_scroll(120.0, 0.0));
        assert!(!MarqueeLayout::needs_scroll(0.0, 0.0));
    }

    // ---- 額外 ----

    #[test]
    fn loop_duration_is_text_plus_gap_over_speed() {
        assert_eq!(MarqueeLayout::POINTS_PER_SECOND, 30.0);
        assert_eq!(MarqueeLayout::GAP, 32.0);
        assert!((MarqueeLayout::loop_duration(268.0) - 10.0).abs() < 1e-9);
        assert!((MarqueeLayout::loop_duration(0.0) - 32.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn nan_and_negative_widths_never_scroll() {
        assert!(!MarqueeLayout::needs_scroll(f64::NAN, 120.0));
        assert!(!MarqueeLayout::needs_scroll(300.0, f64::NAN));
        assert!(!MarqueeLayout::needs_scroll(300.0, -10.0));
        assert!(!MarqueeLayout::needs_scroll(-300.0, 120.0));
        assert!(!MarqueeLayout::needs_scroll(f64::INFINITY, 120.0));
    }

    #[test]
    fn loop_duration_is_finite_and_positive_for_bad_widths() {
        for w in [f64::NAN, -500.0, f64::INFINITY, f64::NEG_INFINITY] {
            let d = MarqueeLayout::loop_duration(w);
            assert!(d.is_finite() && d > 0.0, "{w} -> {d}");
        }
    }

    #[test]
    fn offset_advances_at_speed_and_wraps_after_one_copy_plus_gap() {
        // 文字 268 + 間隔 32 = 一圈 300 點 = 10 秒。
        let w = 268.0;
        assert_eq!(MarqueeLayout::offset(w, 0.0), 0.0);
        assert!((MarqueeLayout::offset(w, 1.0) - 30.0).abs() < 1e-9);
        assert!((MarqueeLayout::offset(w, 9.5) - 285.0).abs() < 1e-9);
        // 一圈後回到起點：第二份剛好接在第一份原本的位置，所以看起來無縫。
        assert!(MarqueeLayout::offset(w, 10.0).abs() < 1e-9);
        assert!((MarqueeLayout::offset(w, 11.0) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn offset_is_zero_for_bad_inputs() {
        assert_eq!(MarqueeLayout::offset(268.0, f64::NAN), 0.0);
        assert_eq!(MarqueeLayout::offset(268.0, -3.0), 0.0);
        assert_eq!(MarqueeLayout::offset(f64::NAN, 3.0), 0.0);
        assert_eq!(MarqueeLayout::offset(268.0, f64::INFINITY), 0.0);
    }
}
