//! 外殼幾何（Swift `NotchMode` 的純運算部分）。
//!
//! 座標：左上角為原點、y 向下。Swift 用 AppKit 的左下原點（容器 `y = topY - height`）；
//! 這裡容器頂端就在 `top_y`、往下延伸，所以容器框的 `y` 就是 `top_y`。平台層負責換算。
//!
//! 為什麼有「固定容器」：視窗外框終身不變，大小足以容納每個模式的外形，hover 判定區因此穩定。
//! 若判定區隨動畫縮放，它只會和最小的暫態尺寸一樣寬容；Preview 只有約 32pt 高的細條，
//! 游標「靜止」時的微小抖動就會跨過邊界，讓外殼一直開開關關。只有畫出來的內容在容器內做動畫。
//!
//! 退化輸入（瀏海尺寸為 0、負數、NaN、無限大）不會 panic：瀏海的寬、高一律先夾成「有限且 >= 0」
//! （NaN／無限大／負數 → 0），所以寬度至少是 `WIDTH_PADDING`、高度至少是 0 加額外高度。
//! `center_x`／`top_y` 原樣傳遞，不另外處理。

use crate::modules::{clamped_preview_extra, MAX_PREVIEW_EXTRA_HEIGHT};

/// 外殼比實體瀏海寬多少：讓它看起來是貼著瀏海的刻意長條，而不是剛好描出缺口（boring.notch 風格）。
/// Preview 與 Expanded 同寬，只有高度往下長。
/// Swift 版是 160（每側 80）；本版 260（每側 130），讓兩翼放得下曲名與演出者。
pub const WIDTH_PADDING: f64 = 260.0;
/// Expanded 的高度（含模組切換列與內容）。
pub const EXPANDED_HEIGHT: f64 = 180.0;
/// Preview 下方兩角的圓角半徑（上方兩角為直角）。
pub const PREVIEW_CORNER_RADIUS: f64 = 10.0;
/// Expanded 下方兩角的圓角半徑（上方兩角為直角）。
pub const EXPANDED_CORNER_RADIUS: f64 = 24.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// 左上角為原點、y 向下的矩形。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// 點是否落在矩形內。邊界規則：含左、上邊，不含右、下邊（半開區間
    /// `x <= p.x < x + w`、`y <= p.y < y + h`），所以兩個相鄰矩形不會重疊，
    /// 寬或高為 0 的矩形不包含任何點。任一座標或矩形欄位是 NaN 時回傳 false。
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

/// 面板的兩種模式（Swift `NotchMode`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellMode {
    Preview,
    Expanded,
}

/// 瀏海（實體或虛擬）的位置與大小，螢幕座標（左上原點、y 向下）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct NotchGeometry {
    /// 瀏海本身的寬高。
    pub notch: Size,
    /// 瀏海水平中心的 x。
    pub center_x: f64,
    /// 螢幕頂端的 y（左上原點下就是 `frame.y`）。
    pub top_y: f64,
}

/// 有限且 >= 0；NaN、無限大、負數都變 0。
pub(super) fn non_negative(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// 該模式的大小。`preview_extra` 只加在 Preview（夾到 `0..=MAX_PREVIEW_EXTRA_HEIGHT`，NaN 當 0）；
/// 寬度與 Expanded 不受影響。
pub fn mode_size(mode: ShellMode, g: &NotchGeometry, preview_extra: f64) -> Size {
    let w = non_negative(g.notch.w) + WIDTH_PADDING;
    match mode {
        ShellMode::Preview => Size {
            w,
            h: non_negative(g.notch.h) + clamped_preview_extra(preview_extra),
        },
        ShellMode::Expanded => Size {
            w,
            h: EXPANDED_HEIGHT,
        },
    }
}

/// 固定容器的大小：放得下「Preview 加最大額外高度」與 Expanded 兩者（取各自最大的寬、高）。
pub fn container_size(g: &NotchGeometry) -> Size {
    let preview = mode_size(ShellMode::Preview, g, MAX_PREVIEW_EXTRA_HEIGHT);
    let expanded = mode_size(ShellMode::Expanded, g, 0.0);
    Size {
        w: preview.w.max(expanded.w),
        h: preview.h.max(expanded.h),
    }
}

/// 固定容器在螢幕座標的外框：水平置中於 `center_x`，頂端貼 `top_y`、往下延伸。
pub fn container_frame(g: &NotchGeometry) -> Rect {
    let size = container_size(g);
    Rect {
        x: g.center_x - size.w / 2.0,
        y: g.top_y,
        w: size.w,
        h: size.h,
    }
}

/// 該模式的 hover 判定區，位在固定容器自己的座標系（原點在容器左上角），頂端置中。
/// 開啟外殼時要求精準 hover 在看得見的 Preview 上；開啟後控制器立刻把判定區放大到整個容器。
pub fn local_rect(mode: ShellMode, g: &NotchGeometry, preview_extra: f64) -> Rect {
    let container = container_size(g);
    let size = mode_size(mode, g, preview_extra);
    Rect {
        x: (container.w - size.w) / 2.0,
        y: 0.0,
        w: size.w,
        h: size.h,
    }
}

/// 外殼動畫長度（秒）：模式切換時畫出的大小與下方圓角都以此長度、[`ease_in_out`] 曲線過渡（spec 5.3）。
pub const ANIMATION_SECONDS: f64 = 0.28;

/// ease-in-out 曲線：CSS／Core Animation 的 `ease-in-out`（三次 Bézier，控制點 (0.42, 0)、(0.58, 1)），
/// 與 Swift 版 `.easeInOut(duration: 0.28)` 同一條曲線。對 (0.5, 0.5) 點對稱、單調遞增，
/// 斜率上限約 1.72（中點）。
///
/// `t` 先夾到 `0..=1`；NaN 視為 0。端點精確：`ease_in_out(0) == 0`、`ease_in_out(1) == 1`。
pub fn ease_in_out(t: f64) -> f64 {
    const X1: f64 = 0.42;
    const X2: f64 = 0.58;
    if t.is_nan() || t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    // Bézier 參數 s → (x, y)；x(s) 單調，以二分法解 x(s) = t（64 次到 f64 精度，沒有發散的風險）。
    let bezier = |s: f64, p1: f64, p2: f64| {
        let r = 1.0 - s;
        3.0 * r * r * s * p1 + 3.0 * r * s * s * p2 + s * s * s
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let mut s = t;
    for _ in 0..64 {
        s = 0.5 * (lo + hi);
        let x = bezier(s, X1, X2);
        if x == t {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
    }
    bezier(s, 0.0, 1.0)
}

/// 兩個大小之間的線性插值：`t = 0` 為 `from`、`t = 1` 為 `to`（不夾 `t`，由呼叫端先套曲線）。
pub fn lerp_size(from: Size, to: Size, t: f64) -> Size {
    Size {
        w: from.w + (to.w - from.w) * t,
        h: from.h + (to.h - from.h) * t,
    }
}

/// 外形某一刻的樣子：外框（內容裁切範圍）、黑底（從外框左上角開始、同寬）與黑底下方兩角的圓角半徑。
///
/// 外框與黑底分開，是為了 Preview 的模組額外高度（歌詞條）：Swift `NotchContentView` 在
/// `mode == .preview && extra > 0` 時只在瀏海高度的範圍塗黑、下方兩角直角（半徑 0），額外高度那一段透明，
/// 由模組自己畫半透明背景。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ShapeLayout {
    /// 外框：頂端置中於容器、內容槽在這個範圍內被裁切。
    pub outer: Size,
    /// 黑底：與外框同寬、頂端對齊；高度不超過外框。
    pub fill: Size,
    /// 黑底下方兩角的圓角半徑（上方兩角恆為直角）。
    pub radius: f64,
}

/// 該模式（含 Preview 額外高度）靜止時的外形，也就是動畫的目標。
///
/// - Preview、額外高度（夾過）為 0：整個外框塗黑，下方圓角 [`PREVIEW_CORNER_RADIUS`]。
/// - Preview、額外高度 > 0：只塗瀏海高度那一段（下方直角，半徑 0），以下透明、交給模組畫。
/// - Expanded：整個外框塗黑，下方圓角 [`EXPANDED_CORNER_RADIUS`]（額外高度無關）。
///
/// 模式或額外高度改變時由 `Animator<ShapeLayout>` 在兩個目標之間插值，所以黑底高度與圓角都連續變化。
pub fn shape_layout(mode: ShellMode, g: &NotchGeometry, preview_extra: f64) -> ShapeLayout {
    let outer = mode_size(mode, g, preview_extra);
    match mode {
        ShellMode::Preview if clamped_preview_extra(preview_extra) > 0.0 => ShapeLayout {
            outer,
            fill: Size {
                w: outer.w,
                h: non_negative(g.notch.h),
            },
            radius: 0.0,
        },
        ShellMode::Preview => ShapeLayout {
            outer,
            fill: outer,
            radius: PREVIEW_CORNER_RADIUS,
        },
        ShellMode::Expanded => ShapeLayout {
            outer,
            fill: outer,
            radius: EXPANDED_CORNER_RADIUS,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 與 Swift 測試一樣：200 x 32 的瀏海，置中於一個虛構螢幕（左上原點）。
    fn geometry() -> NotchGeometry {
        NotchGeometry {
            notch: Size { w: 200.0, h: 32.0 },
            center_x: 756.0,
            top_y: 0.0,
        }
    }

    fn geometry_with(w: f64, h: f64) -> NotchGeometry {
        NotchGeometry {
            notch: Size { w, h },
            ..geometry()
        }
    }

    /// Swift `testPreviewSizeGrowsByClampedExtraHeightOnly`。
    #[test]
    fn preview_size_grows_by_clamped_extra_height_only() {
        let g = geometry();
        let base = mode_size(ShellMode::Preview, &g, 0.0);
        assert_eq!(base.h, 32.0);

        let grown = mode_size(ShellMode::Preview, &g, 24.0);
        assert_eq!(grown.h, 56.0);
        assert_eq!(grown.w, base.w);

        assert_eq!(
            mode_size(ShellMode::Preview, &g, 500.0).h,
            32.0 + MAX_PREVIEW_EXTRA_HEIGHT
        );

        // Expanded ignores the extra height.
        assert_eq!(
            mode_size(ShellMode::Expanded, &g, 24.0),
            mode_size(ShellMode::Expanded, &g, 0.0)
        );
    }

    /// Swift `testPreviewHoverRectIncludesExtraHeightAndContainerFitsIt`。
    #[test]
    fn preview_hover_rect_includes_extra_height_and_container_fits_it() {
        let g = geometry();
        let rect = local_rect(ShellMode::Preview, &g, 24.0);
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.h, 56.0);
        assert_eq!(rect.w, local_rect(ShellMode::Preview, &g, 0.0).w);

        let container = container_size(&g);
        let max_preview = mode_size(ShellMode::Preview, &g, MAX_PREVIEW_EXTRA_HEIGHT);
        assert!(container.h >= max_preview.h);
        assert!(container.h >= mode_size(ShellMode::Expanded, &g, 0.0).h);
    }

    #[test]
    fn width_is_notch_width_plus_padding_for_real_and_virtual_notch() {
        assert_eq!(WIDTH_PADDING, 260.0);
        let real = geometry_with(185.0, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &real, 0.0).w, 445.0);
        assert_eq!(mode_size(ShellMode::Expanded, &real, 0.0).w, 445.0);

        let virtual_notch = geometry_with(200.0, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &virtual_notch, 0.0).w, 460.0);
        assert_eq!(mode_size(ShellMode::Expanded, &virtual_notch, 0.0).w, 460.0);
    }

    #[test]
    fn preview_extra_is_clamped_negative_huge_and_nan() {
        let g = geometry();
        assert_eq!(mode_size(ShellMode::Preview, &g, -5.0).h, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, 99.0).h, 88.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::NAN).h, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::INFINITY).h, 88.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::NEG_INFINITY).h, 32.0);
    }

    #[test]
    fn expanded_has_fixed_height_and_ignores_extra() {
        let g = geometry();
        assert_eq!(EXPANDED_HEIGHT, 180.0);
        for extra in [-5.0, 0.0, 24.0, 99.0, f64::NAN] {
            let size = mode_size(ShellMode::Expanded, &g, extra);
            assert_eq!(size.h, 180.0);
            assert_eq!(size.w, 460.0);
        }
    }

    #[test]
    fn corner_radii_match_the_spec() {
        assert_eq!(PREVIEW_CORNER_RADIUS, 10.0);
        assert_eq!(EXPANDED_CORNER_RADIUS, 24.0);
    }

    #[test]
    fn container_fits_max_preview_and_expanded() {
        let g = geometry();
        let container = container_size(&g);
        let max_preview = mode_size(ShellMode::Preview, &g, MAX_PREVIEW_EXTRA_HEIGHT);
        let expanded = mode_size(ShellMode::Expanded, &g, 0.0);
        assert_eq!(container.w, 460.0);
        assert_eq!(container.h, 180.0);
        assert!(container.w >= max_preview.w && container.w >= expanded.w);
        assert!(container.h >= max_preview.h && container.h >= expanded.h);
    }

    #[test]
    fn container_grows_to_a_tall_notch_when_preview_is_taller_than_expanded() {
        let g = geometry_with(200.0, 400.0);
        // Preview 400 + 56 超過 Expanded 的 180：容器跟著 Preview 走。
        assert_eq!(container_size(&g).h, 456.0);
    }

    #[test]
    fn container_frame_is_horizontally_centered_on_center_x_and_hangs_from_top_y() {
        let mut g = geometry();
        g.top_y = 7.0;
        let frame = container_frame(&g);
        let container = container_size(&g);
        assert_eq!(frame.w, container.w);
        assert_eq!(frame.h, container.h);
        assert_eq!(frame.x + frame.w / 2.0, g.center_x);
        // y 向下：容器頂端就是 top_y，往下延伸。
        assert_eq!(frame.y, 7.0);
    }

    #[test]
    fn local_rect_is_top_centered_in_the_container() {
        let g = geometry();
        let container = container_size(&g);
        let preview = local_rect(ShellMode::Preview, &g, 24.0);
        assert_eq!(preview.x, (container.w - 460.0) / 2.0);
        assert_eq!(preview.y, 0.0);
        assert_eq!(preview.w, 460.0);
        assert_eq!(preview.h, 56.0);

        let expanded = local_rect(ShellMode::Expanded, &g, 24.0);
        assert_eq!(expanded.x, 0.0);
        assert_eq!(expanded.y, 0.0);
        assert_eq!(expanded.w, container.w);
        assert_eq!(expanded.h, 180.0);
    }

    #[test]
    fn local_rect_stays_inside_the_container() {
        let g = geometry_with(185.0, 38.0);
        let container = container_size(&g);
        for mode in [ShellMode::Preview, ShellMode::Expanded] {
            let r = local_rect(mode, &g, MAX_PREVIEW_EXTRA_HEIGHT);
            assert!(r.x >= 0.0 && r.y >= 0.0);
            assert!(r.x + r.w <= container.w && r.y + r.h <= container.h);
        }
    }

    #[test]
    fn degenerate_notch_sizes_do_not_panic_and_clamp_to_zero() {
        for (w, h) in [
            (0.0, 0.0),
            (-10.0, -3.0),
            (f64::NAN, f64::NAN),
            (f64::INFINITY, f64::NEG_INFINITY),
        ] {
            let g = geometry_with(w, h);
            let preview = mode_size(ShellMode::Preview, &g, 10.0);
            assert_eq!(preview.w, WIDTH_PADDING);
            assert_eq!(preview.h, 10.0);
            let expanded = mode_size(ShellMode::Expanded, &g, 10.0);
            assert_eq!(expanded.w, WIDTH_PADDING);
            assert_eq!(expanded.h, EXPANDED_HEIGHT);
            let container = container_size(&g);
            assert_eq!(container.w, WIDTH_PADDING);
            assert_eq!(container.h, EXPANDED_HEIGHT);
            let _ = container_frame(&g);
            let rect = local_rect(ShellMode::Preview, &g, 10.0);
            assert_eq!(rect.x, 0.0);
            assert_eq!(rect.y, 0.0);
        }
    }

    #[test]
    fn rect_contains_includes_left_top_and_excludes_right_bottom() {
        let r = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 30.0,
        };
        assert!(r.contains(Point { x: 10.0, y: 20.0 }));
        assert!(r.contains(Point {
            x: 109.999,
            y: 49.999
        }));
        assert!(!r.contains(Point { x: 110.0, y: 30.0 }));
        assert!(!r.contains(Point { x: 50.0, y: 50.0 }));
        assert!(!r.contains(Point { x: 9.999, y: 30.0 }));
        assert!(!r.contains(Point { x: 50.0, y: 19.999 }));
    }

    #[test]
    fn rect_contains_is_false_for_empty_rects_and_nan() {
        let empty = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 10.0,
        };
        assert!(!empty.contains(Point { x: 0.0, y: 5.0 }));
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        assert!(!r.contains(Point {
            x: f64::NAN,
            y: 5.0
        }));
        assert!(!r.contains(Point {
            x: 5.0,
            y: f64::NAN
        }));
        let nan_rect = Rect { x: f64::NAN, ..r };
        assert!(!nan_rect.contains(Point { x: 5.0, y: 5.0 }));
    }

    #[test]
    fn animation_lasts_280ms() {
        assert_eq!(ANIMATION_SECONDS, 0.28);
    }

    #[test]
    fn ease_in_out_endpoints_and_midpoint() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-12);
        // 慢進慢出：前段比線性慢，後段比線性快。
        assert!(ease_in_out(0.25) < 0.25);
        assert!(ease_in_out(0.75) > 0.75);
    }

    #[test]
    fn ease_in_out_is_symmetric_and_monotonic() {
        let mut prev = ease_in_out(0.0);
        for i in 1..=1000 {
            let t = i as f64 / 1000.0;
            let v = ease_in_out(t);
            assert!(v >= prev, "not monotonic at t={t}");
            assert!(
                (ease_in_out(1.0 - t) - (1.0 - v)).abs() < 1e-12,
                "asymmetric at t={t}"
            );
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
    }

    #[test]
    fn ease_in_out_clamps_out_of_range_t() {
        assert_eq!(ease_in_out(-0.5), 0.0);
        assert_eq!(ease_in_out(-1e300), 0.0);
        assert_eq!(ease_in_out(f64::NEG_INFINITY), 0.0);
        assert_eq!(ease_in_out(1.5), 1.0);
        assert_eq!(ease_in_out(f64::INFINITY), 1.0);
        assert_eq!(ease_in_out(f64::NAN), 0.0);
    }

    #[test]
    fn lerp_size_interpolates_both_axes() {
        let a = Size { w: 100.0, h: 32.0 };
        let b = Size { w: 300.0, h: 180.0 };
        assert_eq!(lerp_size(a, b, 0.0), a);
        assert_eq!(lerp_size(a, b, 1.0), b);
        assert_eq!(lerp_size(a, b, 0.5), Size { w: 200.0, h: 106.0 });
        assert_eq!(lerp_size(a, b, 0.25), Size { w: 150.0, h: 69.0 });
        // 反方向也成立。
        assert_eq!(lerp_size(b, a, 0.5), Size { w: 200.0, h: 106.0 });
    }

    fn g185() -> NotchGeometry {
        geometry_with(185.0, 32.0)
    }

    #[test]
    fn shape_layout_preview_without_extra_is_fully_black_with_radius_10() {
        let s = shape_layout(ShellMode::Preview, &g185(), 0.0);
        assert_eq!(s.outer, Size { w: 445.0, h: 32.0 });
        assert_eq!(s.fill, s.outer);
        assert_eq!(s.radius, PREVIEW_CORNER_RADIUS);
    }

    #[test]
    fn shape_layout_preview_with_extra_fills_only_the_notch_band_with_square_corners() {
        let s = shape_layout(ShellMode::Preview, &g185(), 24.0);
        assert_eq!(s.outer, Size { w: 445.0, h: 56.0 });
        assert_eq!(s.fill, Size { w: 445.0, h: 32.0 });
        assert_eq!(s.radius, 0.0);
        // 額外高度被夾到 56。
        let s = shape_layout(ShellMode::Preview, &g185(), 99.0);
        assert_eq!(s.outer.h, 88.0);
        assert_eq!(s.fill.h, 32.0);
        assert_eq!(s.radius, 0.0);
    }

    #[test]
    fn shape_layout_treats_negative_and_nan_extra_as_no_strip() {
        for extra in [-5.0, f64::NAN, f64::NEG_INFINITY, 0.0] {
            let s = shape_layout(ShellMode::Preview, &g185(), extra);
            assert_eq!(
                s,
                shape_layout(ShellMode::Preview, &g185(), 0.0),
                "extra={extra}"
            );
        }
    }

    #[test]
    fn shape_layout_expanded_is_fully_black_with_radius_24_regardless_of_extra() {
        for extra in [0.0, 24.0, f64::NAN] {
            let s = shape_layout(ShellMode::Expanded, &g185(), extra);
            assert_eq!(s.outer, Size { w: 445.0, h: 180.0 });
            assert_eq!(s.fill, s.outer);
            assert_eq!(s.radius, EXPANDED_CORNER_RADIUS);
        }
    }

    #[test]
    fn shape_layout_outer_matches_mode_size_and_fill_never_exceeds_it() {
        for g in [
            g185(),
            geometry(),
            geometry_with(0.0, 0.0),
            geometry_with(200.0, 400.0),
        ] {
            for mode in [ShellMode::Preview, ShellMode::Expanded] {
                for extra in [0.0, 10.0, 28.0] {
                    let s = shape_layout(mode, &g, extra);
                    assert_eq!(s.outer, mode_size(mode, &g, extra));
                    assert_eq!(s.fill.w, s.outer.w);
                    assert!(s.fill.h <= s.outer.h);
                }
            }
        }
    }

    #[test]
    fn shape_layout_zero_height_notch_with_extra_has_an_empty_black_band() {
        let s = shape_layout(ShellMode::Preview, &geometry_with(200.0, 0.0), 24.0);
        assert_eq!(s.outer.h, 24.0);
        assert_eq!(s.fill.h, 0.0);
    }
}
