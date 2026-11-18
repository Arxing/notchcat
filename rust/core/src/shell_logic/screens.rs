//! 螢幕選擇與虛擬瀏海。
//!
//! 外殼只認得 `ScreenInfo`（平台層把 NSScreen／Win32 螢幕轉成它，硬體瀏海尺寸由 Task 21 的 OS 程式推得）。
//! 選定的螢幕被拔掉或闔上時，面板移到預設螢幕、程式不結束；螢幕回來時再選一次就會移回去。
//! 座標：左上原點、y 向下。
//!
//! 硬體瀏海只有在寬、高都是有限且 > 0 時才算數（Swift 的偵測器也要求寬 > 0）；
//! 否則視同沒有瀏海，改用虛擬瀏海。

use super::geometry::{non_negative, NotchGeometry, Rect, Size};

/// 虛擬瀏海的預設寬（可在設定檔調整）。
pub const DEFAULT_VIRTUAL_NOTCH_WIDTH: f64 = 200.0;
/// 虛擬瀏海的預設高（可在設定檔調整）。
pub const DEFAULT_VIRTUAL_NOTCH_HEIGHT: f64 = 32.0;

/// 平台層回報的一個螢幕。
#[derive(Clone, PartialEq, Debug)]
pub struct ScreenInfo {
    /// 穩定識別（設定檔記住使用者選的螢幕用）。
    pub id: String,
    pub name: String,
    /// 螢幕外框，左上原點、y 向下。
    pub frame: Rect,
    /// 實體瀏海的寬高；沒有瀏海為 `None`。
    pub hardware_notch: Option<Size>,
    pub is_primary: bool,
    pub is_builtin: bool,
}

impl ScreenInfo {
    /// 有可用的實體瀏海（寬、高皆有限且 > 0）。
    fn usable_hardware_notch(&self) -> Option<Size> {
        self.hardware_notch
            .filter(|n| n.w.is_finite() && n.w > 0.0 && n.h.is_finite() && n.h > 0.0)
    }
}

/// 沒有實體瀏海的螢幕上畫的虛擬瀏海大小。預設 200 x 32。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VirtualNotch {
    pub w: f64,
    pub h: f64,
}

impl Default for VirtualNotch {
    fn default() -> Self {
        Self {
            w: DEFAULT_VIRTUAL_NOTCH_WIDTH,
            h: DEFAULT_VIRTUAL_NOTCH_HEIGHT,
        }
    }
}

/// 選出面板要放的螢幕。順序：
/// 1. `preferred` 的 id 在列表中 → 它；
/// 2. 否則第一個有實體瀏海的螢幕；
/// 3. 否則第一個內建螢幕；
/// 4. 否則主螢幕；
/// 5. 否則列表第一個。
///
/// 列表為空回 `None`（呼叫端暫時隱藏面板，不結束程式）。
pub fn choose_screen<'a>(
    screens: &'a [ScreenInfo],
    preferred: Option<&str>,
) -> Option<&'a ScreenInfo> {
    preferred
        .and_then(|id| screens.iter().find(|s| s.id == id))
        .or_else(|| screens.iter().find(|s| s.usable_hardware_notch().is_some()))
        .or_else(|| screens.iter().find(|s| s.is_builtin))
        .or_else(|| screens.iter().find(|s| s.is_primary))
        .or_else(|| screens.first())
}

/// 該螢幕上的瀏海幾何。有實體瀏海就用它；否則用 `virtual_notch`，水平置中於該螢幕。
/// 兩種情況 `center_x` 都是螢幕外框的水平中心（實體瀏海在機身上本來就置中；
/// 螢幕沒有提供更精確的中心時以此為準），`top_y` 是螢幕頂端 `frame.y`。
/// 虛擬瀏海的寬、高為 NaN／負數時夾成 0（見 `geometry` 的退化輸入說明）。
pub fn geometry_for(screen: &ScreenInfo, virtual_notch: VirtualNotch) -> NotchGeometry {
    let notch = screen.usable_hardware_notch().unwrap_or(Size {
        w: non_negative(virtual_notch.w),
        h: non_negative(virtual_notch.h),
    });
    NotchGeometry {
        notch,
        center_x: screen.frame.x + screen.frame.w / 2.0,
        top_y: screen.frame.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(id: &str, x: f64, notch: Option<Size>, primary: bool, builtin: bool) -> ScreenInfo {
        ScreenInfo {
            id: id.to_string(),
            name: format!("Screen {id}"),
            frame: Rect {
                x,
                y: 0.0,
                w: 1512.0,
                h: 982.0,
            },
            hardware_notch: notch,
            is_primary: primary,
            is_builtin: builtin,
        }
    }

    fn notch() -> Option<Size> {
        Some(Size { w: 185.0, h: 32.0 })
    }

    fn id_of(s: Option<&ScreenInfo>) -> Option<&str> {
        s.map(|s| s.id.as_str())
    }

    #[test]
    fn preferred_screen_is_chosen_when_present() {
        let screens = [
            screen("a", 0.0, notch(), true, true),
            screen("b", 1512.0, None, false, false),
        ];
        assert_eq!(id_of(choose_screen(&screens, Some("b"))), Some("b"));
    }

    #[test]
    fn preferred_screen_missing_falls_back_to_notched_then_builtin_then_primary() {
        // 有瀏海的優先（即使不是內建、不是主螢幕）。
        let screens = [
            screen("primary", 0.0, None, true, false),
            screen("builtin", 100.0, None, false, true),
            screen("notched", 200.0, notch(), false, false),
        ];
        assert_eq!(
            id_of(choose_screen(&screens, Some("gone"))),
            Some("notched")
        );

        // 沒有瀏海 → 內建。
        let screens = [
            screen("primary", 0.0, None, true, false),
            screen("builtin", 100.0, None, false, true),
            screen("other", 200.0, None, false, false),
        ];
        assert_eq!(
            id_of(choose_screen(&screens, Some("gone"))),
            Some("builtin")
        );

        // 沒有瀏海也沒有內建 → 主螢幕。
        let screens = [
            screen("other", 0.0, None, false, false),
            screen("primary", 100.0, None, true, false),
        ];
        assert_eq!(
            id_of(choose_screen(&screens, Some("gone"))),
            Some("primary")
        );

        // 什麼都不是 → 第一個。
        let screens = [
            screen("first", 0.0, None, false, false),
            screen("second", 100.0, None, false, false),
        ];
        assert_eq!(id_of(choose_screen(&screens, Some("gone"))), Some("first"));
    }

    #[test]
    fn no_preference_uses_the_same_fallback_order() {
        let screens = [
            screen("primary", 0.0, None, true, false),
            screen("notched", 100.0, notch(), false, false),
        ];
        assert_eq!(id_of(choose_screen(&screens, None)), Some("notched"));
    }

    #[test]
    fn empty_screen_list_returns_none_instead_of_panicking() {
        assert!(choose_screen(&[], None).is_none());
        assert!(choose_screen(&[], Some("anything")).is_none());
    }

    #[test]
    fn preferred_screen_returning_is_chosen_again() {
        let external = screen("ext", 1512.0, None, false, false);
        let builtin = screen("builtin", 0.0, notch(), true, true);

        // 外接螢幕在：選它。
        let both = [builtin.clone(), external.clone()];
        assert_eq!(id_of(choose_screen(&both, Some("ext"))), Some("ext"));
        // 被拔掉：退回有瀏海的內建螢幕。
        let only_builtin = [builtin.clone()];
        assert_eq!(
            id_of(choose_screen(&only_builtin, Some("ext"))),
            Some("builtin")
        );
        // 全部消失：None，不 panic。
        assert!(choose_screen(&[], Some("ext")).is_none());
        // 回來：再選回它。
        assert_eq!(id_of(choose_screen(&both, Some("ext"))), Some("ext"));
    }

    #[test]
    fn multiple_notched_screens_pick_the_first() {
        let screens = [
            screen("plain", 0.0, None, true, false),
            screen("n1", 100.0, notch(), false, false),
            screen("n2", 200.0, notch(), false, true),
        ];
        assert_eq!(id_of(choose_screen(&screens, None)), Some("n1"));
    }

    #[test]
    fn unusable_hardware_notch_counts_as_no_notch() {
        for bad in [
            Size { w: 0.0, h: 32.0 },
            Size { w: 185.0, h: 0.0 },
            Size { w: -1.0, h: 32.0 },
            Size {
                w: f64::NAN,
                h: 32.0,
            },
        ] {
            let screens = [
                screen("primary", 0.0, None, true, false),
                screen("bogus", 100.0, Some(bad), false, false),
            ];
            assert_eq!(id_of(choose_screen(&screens, None)), Some("primary"));
            let g = geometry_for(&screens[1], VirtualNotch::default());
            assert_eq!(g.notch, Size { w: 200.0, h: 32.0 });
        }
    }

    #[test]
    fn hardware_notch_is_used_as_is_and_centered_on_the_screen() {
        let s = screen("a", 100.0, notch(), true, true);
        let g = geometry_for(&s, VirtualNotch { w: 999.0, h: 99.0 });
        assert_eq!(g.notch, Size { w: 185.0, h: 32.0 });
        assert_eq!(g.center_x, 100.0 + 1512.0 / 2.0);
        assert_eq!(g.top_y, 0.0);
    }

    #[test]
    fn screen_without_hardware_notch_gets_the_virtual_one_centered() {
        let mut s = screen("ext", 1512.0, None, false, false);
        s.frame.y = 25.0;
        let g = geometry_for(&s, VirtualNotch::default());
        assert_eq!(g.notch, Size { w: 200.0, h: 32.0 });
        assert_eq!(g.center_x, 1512.0 + 1512.0 / 2.0);
        assert_eq!(g.top_y, 25.0);
    }

    #[test]
    fn virtual_notch_defaults_to_200_by_32() {
        let v = VirtualNotch::default();
        assert_eq!((v.w, v.h), (200.0, 32.0));
        assert_eq!(DEFAULT_VIRTUAL_NOTCH_WIDTH, 200.0);
        assert_eq!(DEFAULT_VIRTUAL_NOTCH_HEIGHT, 32.0);
    }

    #[test]
    fn virtual_notch_size_is_configurable() {
        let s = screen("ext", 0.0, None, true, false);
        let g = geometry_for(&s, VirtualNotch { w: 240.0, h: 40.0 });
        assert_eq!(g.notch, Size { w: 240.0, h: 40.0 });
        // 寬度 = 虛擬瀏海寬 + 160。
        assert_eq!(
            crate::shell_logic::mode_size(crate::shell_logic::ShellMode::Preview, &g, 0.0).w,
            400.0
        );
    }

    #[test]
    fn degenerate_virtual_notch_does_not_panic_and_clamps_to_zero() {
        let s = screen("ext", 0.0, None, true, false);
        for (w, h) in [(-5.0, -5.0), (f64::NAN, f64::NAN), (0.0, 0.0)] {
            let g = geometry_for(&s, VirtualNotch { w, h });
            assert_eq!(g.notch, Size { w: 0.0, h: 0.0 });
            assert_eq!(g.center_x, 756.0);
        }
    }
}
