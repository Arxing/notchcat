//! 螢幕列舉與座標轉換（NSScreen → `musicat_core::shell_logic::ScreenInfo`）。
//!
//! # 全域座標空間（`ScreenInfo.frame`、[`crate::panel::mouse_location`]、`configure_panel` 的 `frame` 共用）
//!
//! MusiCat 的「全域座標」= **主螢幕左上角為原點、x 向右、y 向下、單位為點（point）**。
//! 主螢幕是帶選單列的那個螢幕（`CGMainDisplayID()`，也就是 `NSScreen.screens[0]`，
//! 在 AppKit 全域座標裡的原點是 `(0, 0)`）。它與 CoreGraphics 的全域顯示座標
//! （`CGDisplayBounds`、`CGWindowListCopyWindowInfo` 的 `kCGWindowBounds`）是同一個空間，
//! 所以外部工具量到的視窗 bounds 可以直接拿來比對。
//!
//! AppKit 的全域座標以主螢幕**左下角**為原點、y 向上。兩者的換算只需要主螢幕高度 `H`：
//!
//! - 點：`(x, y_appkit)` → `(x, H − y_appkit)`
//! - 矩形：`(x, y_appkit, w, h)` → `(x, H − (y_appkit + h), w, h)`（AppKit 的 `y` 是下緣，換成上緣）
//!
//! 換算是自身的反函數（對同一個 `H` 做兩次會回到原值）。其他螢幕（例如主螢幕上方的外接螢幕）
//! 在這個空間裡的 y 可以是負數，這是正常的。
//!
//! 換算與 `ScreenInfo` 的組裝是純函式（[`screen_info_from`] 等），在所有平台上編譯並測試；
//! 讀 NSScreen 的 [`list_screens`] 只在 macOS 編譯。

use musicat_core::shell_logic::{Point, Rect, ScreenInfo, Size};

/// 從 NSScreen 讀到的原始數值（尚未換算）。
#[derive(Clone, PartialEq, Debug)]
pub struct RawScreen {
    /// `CGDirectDisplayID`（`deviceDescription["NSScreenNumber"]`）。
    pub display_id: u32,
    /// `NSScreen.localizedName`。
    pub name: String,
    /// `NSScreen.frame`，**AppKit 全域座標**（主螢幕左下原點、y 向上）。
    pub appkit_frame: Rect,
    /// `auxiliaryTopLeftArea` 的寬高；沒有瀏海時為 0×0。
    pub aux_left: Size,
    /// `auxiliaryTopRightArea` 的寬高；沒有瀏海時為 0×0。
    pub aux_right: Size,
    /// 是否為主螢幕（`CGMainDisplayID()`）。
    pub is_primary: bool,
    /// `CGDisplayIsBuiltin`。
    pub is_builtin: bool,
}

/// 螢幕的穩定識別字串：`CGDirectDisplayID` 的十進位表示（例如 `"1"`）。
///
/// 同一台螢幕在同一次開機內不變；外接螢幕跨重開機或換連接埠時 macOS 可能給新的 id，
/// 屆時 `choose_screen` 會退回預設螢幕（不會崩潰）。
pub fn screen_id(display_id: u32) -> String {
    display_id.to_string()
}

/// AppKit 全域點 → MusiCat 全域點（見模組文件）。`primary_height` 是主螢幕高度（點）。
pub fn appkit_point_to_global(p: Point, primary_height: f64) -> Point {
    Point {
        x: p.x,
        y: primary_height - p.y,
    }
}

/// AppKit 全域矩形（左下原點、`y` 為下緣）→ MusiCat 全域矩形（左上原點、`y` 為上緣）。
pub fn appkit_rect_to_global(r: Rect, primary_height: f64) -> Rect {
    Rect {
        x: r.x,
        y: primary_height - (r.y + r.h),
        w: r.w,
        h: r.h,
    }
}

/// MusiCat 全域矩形 → AppKit 全域矩形（`setFrame` 用）。與 [`appkit_rect_to_global`] 互為反函數。
pub fn global_rect_to_appkit(r: Rect, primary_height: f64) -> Rect {
    // 翻轉是自身的反函數。
    appkit_rect_to_global(r, primary_height)
}

/// 由兩塊輔助區推出實體瀏海的寬高（移植自 Swift `NotchDetector.detect()`）：
/// 寬 = 螢幕寬 − 左區寬 − 右區寬，高 = 左區高（輔助區與選單列同高，在所有有瀏海的 Mac 上都等於瀏海高）。
///
/// 任一輔助區不存在（寬或高為 0）、數值不是有限值、或算出的寬度 <= 0 時回 `None`。
pub fn hardware_notch_from(screen_width: f64, aux_left: Size, aux_right: Size) -> Option<Size> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !(positive(aux_left.w) && positive(aux_left.h) && positive(aux_right.w)) {
        return None;
    }
    let w = screen_width - aux_left.w - aux_right.w;
    positive(w).then_some(Size { w, h: aux_left.h })
}

/// 把一個 NSScreen 的原始數值組成 core 的 `ScreenInfo`（frame 換成 MusiCat 全域座標）。
pub fn screen_info_from(raw: &RawScreen, primary_height: f64) -> ScreenInfo {
    ScreenInfo {
        id: screen_id(raw.display_id),
        name: raw.name.clone(),
        frame: appkit_rect_to_global(raw.appkit_frame, primary_height),
        hardware_notch: hardware_notch_from(raw.appkit_frame.w, raw.aux_left, raw.aux_right),
        is_primary: raw.is_primary,
        is_builtin: raw.is_builtin,
    }
}

/// 主螢幕高度（點），MusiCat 全域座標與 AppKit 全域座標換算的唯一參數。
///
/// 用 `CGDisplayBounds(CGMainDisplayID())`：任何執行緒都能呼叫，且與 `NSScreen.screens[0].frame` 相同。
#[cfg(target_os = "macos")]
pub(crate) fn primary_screen_height() -> f64 {
    core_graphics::display::CGDisplay::main()
        .bounds()
        .size
        .height
}

/// 列出目前所有螢幕（`NSScreen.screens` 的順序）。
///
/// 必須在主執行緒呼叫（AppKit 的要求）；在其他執行緒呼叫時印警告並回空列表
/// （core 把空列表當成「暫時沒有螢幕」，面板隱藏，程式不結束）。
/// 螢幕組態變更的通知不在這裡處理：app 會定期重新呼叫本函式。
#[cfg(target_os = "macos")]
pub fn list_screens() -> Vec<ScreenInfo> {
    use core_graphics::display::CGDisplay;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{ns_string, MainThreadMarker, NSNumber};

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("musicat: list_screens() must run on the main thread; returning no screens");
        return Vec::new();
    };
    let primary_height = primary_screen_height();
    let main_id = CGDisplay::main().id;

    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let display_id = screen
                .deviceDescription()
                .objectForKey(ns_string!("NSScreenNumber"))
                .and_then(|n| n.downcast::<NSNumber>().ok())
                .map(|n| n.unsignedIntValue())
                .unwrap_or(0);
            let f = screen.frame();
            let left = screen.auxiliaryTopLeftArea();
            let right = screen.auxiliaryTopRightArea();
            let raw = RawScreen {
                display_id,
                name: screen.localizedName().to_string(),
                appkit_frame: Rect {
                    x: f.origin.x,
                    y: f.origin.y,
                    w: f.size.width,
                    h: f.size.height,
                },
                aux_left: Size {
                    w: left.size.width,
                    h: left.size.height,
                },
                aux_right: Size {
                    w: right.size.width,
                    h: right.size.height,
                },
                is_primary: display_id == main_id,
                is_builtin: CGDisplay::new(display_id).is_builtin(),
            };
            screen_info_from(&raw, primary_height)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 982.0;

    fn size(w: f64, h: f64) -> Size {
        Size { w, h }
    }

    /// 14 吋 MacBook Pro 的內建螢幕（1512×982，瀏海 185×32），是主螢幕。
    fn builtin() -> RawScreen {
        RawScreen {
            display_id: 1,
            name: "Built-in Retina Display".to_string(),
            appkit_frame: Rect {
                x: 0.0,
                y: 0.0,
                w: 1512.0,
                h: 982.0,
            },
            aux_left: size(663.5, 32.0),
            aux_right: size(663.5, 32.0),
            is_primary: true,
            is_builtin: true,
        }
    }

    #[test]
    fn notch_width_is_screen_width_minus_both_auxiliary_areas() {
        let info = screen_info_from(&builtin(), H);
        assert_eq!(info.hardware_notch, Some(size(185.0, 32.0)));

        // 左右不對稱也照公式算；高度取左區的高。
        let n = hardware_notch_from(1512.0, size(600.0, 38.0), size(700.0, 38.0));
        assert_eq!(n, Some(size(212.0, 38.0)));
    }

    #[test]
    fn absent_or_zero_auxiliary_area_means_no_notch() {
        let zero = size(0.0, 0.0);
        assert_eq!(hardware_notch_from(1920.0, zero, zero), None);
        assert_eq!(hardware_notch_from(1920.0, size(800.0, 32.0), zero), None);
        assert_eq!(hardware_notch_from(1920.0, zero, size(800.0, 32.0)), None);
        // 左區寬度有、高度是 0：沒有可用的瀏海高。
        assert_eq!(
            hardware_notch_from(1920.0, size(800.0, 0.0), size(800.0, 32.0)),
            None
        );
        // NaN／無限大也不算。
        assert_eq!(
            hardware_notch_from(1920.0, size(f64::NAN, 32.0), size(800.0, 32.0)),
            None
        );
        assert_eq!(
            hardware_notch_from(f64::INFINITY, size(800.0, 32.0), size(800.0, 32.0)),
            None
        );

        let mut external = builtin();
        external.aux_left = zero;
        external.aux_right = zero;
        assert_eq!(screen_info_from(&external, H).hardware_notch, None);
    }

    #[test]
    fn non_positive_notch_width_means_no_notch() {
        assert_eq!(
            hardware_notch_from(1512.0, size(756.0, 32.0), size(756.0, 32.0)),
            None
        );
        assert_eq!(
            hardware_notch_from(1512.0, size(800.0, 32.0), size(800.0, 32.0)),
            None
        );
    }

    #[test]
    fn primary_screen_frame_maps_to_origin_top_left() {
        let info = screen_info_from(&builtin(), H);
        assert_eq!(
            info.frame,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1512.0,
                h: 982.0
            }
        );
    }

    #[test]
    fn y_axis_is_flipped_about_the_primary_screen_height() {
        // 外接螢幕在主螢幕右側、下緣齊平（AppKit y = 0），高 1080。
        let mut right = builtin();
        right.appkit_frame = Rect {
            x: 1512.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        // 上緣比主螢幕高 98 點 → MusiCat 全域座標 y = 982 − 1080 = −98。
        assert_eq!(
            screen_info_from(&right, H).frame,
            Rect {
                x: 1512.0,
                y: -98.0,
                w: 1920.0,
                h: 1080.0
            }
        );

        // 外接螢幕疊在主螢幕正上方（AppKit y = 982）。
        let mut above = builtin();
        above.appkit_frame = Rect {
            x: 0.0,
            y: 982.0,
            w: 1920.0,
            h: 1080.0,
        };
        assert_eq!(screen_info_from(&above, H).frame.y, -1080.0);

        // 主螢幕下方（AppKit y = −1080）。
        let mut below = builtin();
        below.appkit_frame.y = -1080.0;
        below.appkit_frame.h = 1080.0;
        assert_eq!(screen_info_from(&below, H).frame.y, 982.0);
    }

    #[test]
    fn point_conversion_flips_y_about_the_primary_height() {
        // AppKit 的主螢幕頂端（y = H）就是 MusiCat 的 y = 0。
        assert_eq!(
            appkit_point_to_global(Point { x: 10.0, y: H }, H),
            Point { x: 10.0, y: 0.0 }
        );
        assert_eq!(
            appkit_point_to_global(Point { x: 10.0, y: 0.0 }, H),
            Point { x: 10.0, y: H }
        );
        assert_eq!(
            appkit_point_to_global(Point { x: -5.0, y: 1000.0 }, H),
            Point { x: -5.0, y: -18.0 }
        );
    }

    #[test]
    fn rect_conversion_round_trips() {
        let panel = Rect {
            x: 583.5,
            y: 0.0,
            w: 345.0,
            h: 180.0,
        };
        let appkit = global_rect_to_appkit(panel, H);
        // 面板頂端貼主螢幕頂端：AppKit 下緣 = 982 − 180。
        assert_eq!(
            appkit,
            Rect {
                x: 583.5,
                y: 802.0,
                w: 345.0,
                h: 180.0
            }
        );
        assert_eq!(appkit_rect_to_global(appkit, H), panel);
    }

    #[test]
    fn screen_id_is_the_decimal_display_id_and_flags_pass_through() {
        assert_eq!(screen_id(1), "1");
        assert_eq!(screen_id(4_280_131_617), "4280131617");

        let mut raw = builtin();
        raw.display_id = 69_734_662;
        raw.is_primary = false;
        raw.is_builtin = false;
        let info = screen_info_from(&raw, H);
        assert_eq!(info.id, "69734662");
        assert_eq!(info.name, "Built-in Retina Display");
        assert!(!info.is_primary);
        assert!(!info.is_builtin);
        assert!(screen_info_from(&builtin(), H).is_primary);
        assert!(screen_info_from(&builtin(), H).is_builtin);
    }
}
