//! 外殼驅動迴圈的純決策（GPUI 視窗程式碼抽出來、可單元測試的部分）。
//!
//! `shell` 的視窗以輪詢驅動：每一拍讀全域滑鼠位置、餵 [`super::HoverMachine`]、決定下一拍何時醒來。
//! 這裡只放與 OS 無關的計算：座標換算、節拍長度、點擊穿透。

use super::geometry::{Point, Rect, ShellMode};

/// 動畫中、游標在容器附近或待收合時的節拍（秒）：約 60 Hz，與畫面更新同頻。
pub const FAST_TICK: f64 = 0.016;
/// 其餘時候（游標遠離面板、沒有動畫）的節拍（秒）：降頻省電。游標從遠處一口氣跳進 Preview 時，
/// 展開最多因此晚這麼久。
pub const IDLE_TICK: f64 = 0.040;
/// 「游標在容器附近」的判定外擴量（點）：游標進入外擴後的範圍就切到 [`FAST_TICK`]，
/// 讓它真正進入 Preview 的那一刻是以快節拍取樣的。
pub const NEAR_MARGIN: f64 = 80.0;
/// 醒來時間的下限（秒）：期限已到或極近時也至少等這麼久，避免忙等。
pub const MIN_TICK: f64 = 0.001;

/// 全域座標的點 → 以 `frame`（容器的全域外框）左上角為原點的容器座標。
pub fn to_local(p: Point, frame: Rect) -> Point {
    Point {
        x: p.x - frame.x,
        y: p.y - frame.y,
    }
}

/// 四邊各外擴 `by`（點）。`by` 為負時內縮（寬高不小於 0）。
pub fn inflate(r: Rect, by: f64) -> Rect {
    Rect {
        x: r.x - by,
        y: r.y - by,
        w: (r.w + 2.0 * by).max(0.0),
        h: (r.h + 2.0 * by).max(0.0),
    }
}

/// 驅動端這一拍的狀態。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TickState {
    /// 現在（秒，與 `deadline` 同一時鐘）。
    pub now: f64,
    /// 形狀還在動畫中。
    pub animating: bool,
    /// 游標在外擴後的容器內（全域座標判定，見 [`NEAR_MARGIN`]）。
    pub mouse_near: bool,
    /// `HoverOutput::next_deadline`：待收合的期限。
    pub deadline: Option<f64>,
}

/// 下一拍要等多久（秒）。動畫中、游標在附近或待收合時用 [`FAST_TICK`]，否則 [`IDLE_TICK`]；
/// 有期限時不晚於期限醒來（HoverMachine 需要在 `next_deadline` 準時再被呼叫一次）。
/// 結果在 `MIN_TICK..=IDLE_TICK` 之間；NaN 的時間或期限當作沒有期限。
pub fn next_tick(s: TickState) -> f64 {
    let base = if s.animating || s.mouse_near || s.deadline.is_some() {
        FAST_TICK
    } else {
        IDLE_TICK
    };
    let until_deadline = s.deadline.map(|d| d - s.now).filter(|d| !d.is_nan());
    let wait = match until_deadline {
        Some(d) if d < base => d,
        _ => base,
    };
    wait.max(MIN_TICK)
}

/// 面板是否要忽略滑鼠（點擊穿透）：只有可見且 Expanded（有按鈕）時接收滑鼠；
/// Preview（容器大部分是透明區域）與隱藏時一律穿透。展開靠輪詢全域游標，不需要滑鼠事件。
pub fn ignores_mouse(mode: ShellMode, visible: bool) -> bool {
    !(visible && mode == ShellMode::Expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_local_subtracts_the_container_origin() {
        let frame = Rect {
            x: 583.5,
            y: 0.0,
            w: 345.0,
            h: 180.0,
        };
        assert_eq!(
            to_local(Point { x: 756.0, y: 16.0 }, frame),
            Point { x: 172.5, y: 16.0 }
        );
        // 其他螢幕（主螢幕上方，y 為負）。
        let above = Rect {
            x: -200.0,
            y: -1080.0,
            w: 360.0,
            h: 180.0,
        };
        assert_eq!(
            to_local(
                Point {
                    x: -100.0,
                    y: -1070.0
                },
                above
            ),
            Point { x: 100.0, y: 10.0 }
        );
    }

    #[test]
    fn inflate_grows_every_side_and_never_goes_negative() {
        let r = Rect {
            x: 10.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        assert_eq!(
            inflate(r, 5.0),
            Rect {
                x: 5.0,
                y: -5.0,
                w: 110.0,
                h: 60.0
            }
        );
        let shrunk = inflate(r, -100.0);
        assert_eq!(shrunk.w, 0.0);
        assert_eq!(shrunk.h, 0.0);
    }

    fn state(animating: bool, mouse_near: bool, deadline: Option<f64>) -> TickState {
        TickState {
            now: 10.0,
            animating,
            mouse_near,
            deadline,
        }
    }

    #[test]
    fn idle_ticks_slowly_and_activity_ticks_fast() {
        assert_eq!(next_tick(state(false, false, None)), IDLE_TICK);
        assert_eq!(next_tick(state(true, false, None)), FAST_TICK);
        assert_eq!(next_tick(state(false, true, None)), FAST_TICK);
        assert_eq!(next_tick(state(false, false, Some(11.0))), FAST_TICK);
        const { assert!(IDLE_TICK > FAST_TICK) };
    }

    #[test]
    fn wakes_exactly_at_a_near_deadline() {
        let got = next_tick(state(false, true, Some(10.005)));
        assert!((got - 0.005).abs() < 1e-12, "{got}");
    }

    #[test]
    fn past_or_bogus_deadlines_do_not_busy_loop() {
        assert_eq!(next_tick(state(false, false, Some(9.0))), MIN_TICK);
        assert_eq!(next_tick(state(false, false, Some(10.0))), MIN_TICK);
        assert_eq!(next_tick(state(false, false, Some(f64::NAN))), FAST_TICK);
        assert_eq!(
            next_tick(TickState {
                now: f64::NAN,
                ..state(false, false, Some(11.0))
            }),
            FAST_TICK
        );
        assert_eq!(
            next_tick(state(false, false, Some(f64::NEG_INFINITY))),
            MIN_TICK
        );
        assert_eq!(
            next_tick(state(false, false, Some(f64::INFINITY))),
            FAST_TICK
        );
    }

    #[test]
    fn only_a_visible_expanded_panel_takes_the_mouse() {
        assert!(!ignores_mouse(ShellMode::Expanded, true));
        assert!(ignores_mouse(ShellMode::Preview, true));
        assert!(ignores_mouse(ShellMode::Expanded, false));
        assert!(ignores_mouse(ShellMode::Preview, false));
    }
}
