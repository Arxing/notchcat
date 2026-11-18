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
pub const WIDTH_PADDING: f64 = 160.0;
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
        assert_eq!(WIDTH_PADDING, 160.0);
        let real = geometry_with(185.0, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &real, 0.0).w, 345.0);
        assert_eq!(mode_size(ShellMode::Expanded, &real, 0.0).w, 345.0);

        let virtual_notch = geometry_with(200.0, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &virtual_notch, 0.0).w, 360.0);
        assert_eq!(mode_size(ShellMode::Expanded, &virtual_notch, 0.0).w, 360.0);
    }

    #[test]
    fn preview_extra_is_clamped_negative_huge_and_nan() {
        let g = geometry();
        assert_eq!(mode_size(ShellMode::Preview, &g, -5.0).h, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, 99.0).h, 60.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::NAN).h, 32.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::INFINITY).h, 60.0);
        assert_eq!(mode_size(ShellMode::Preview, &g, f64::NEG_INFINITY).h, 32.0);
    }

    #[test]
    fn expanded_has_fixed_height_and_ignores_extra() {
        let g = geometry();
        assert_eq!(EXPANDED_HEIGHT, 180.0);
        for extra in [-5.0, 0.0, 24.0, 99.0, f64::NAN] {
            let size = mode_size(ShellMode::Expanded, &g, extra);
            assert_eq!(size.h, 180.0);
            assert_eq!(size.w, 360.0);
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
        assert_eq!(container.w, 360.0);
        assert_eq!(container.h, 180.0);
        assert!(container.w >= max_preview.w && container.w >= expanded.w);
        assert!(container.h >= max_preview.h && container.h >= expanded.h);
    }

    #[test]
    fn container_grows_to_a_tall_notch_when_preview_is_taller_than_expanded() {
        let g = geometry_with(200.0, 400.0);
        // Preview 400 + 28 超過 Expanded 的 180：容器跟著 Preview 走。
        assert_eq!(container_size(&g).h, 428.0);
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
        assert_eq!(preview.x, (container.w - 360.0) / 2.0);
        assert_eq!(preview.y, 0.0);
        assert_eq!(preview.w, 360.0);
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
}
