//! 外殼形狀的動畫（純邏輯，不碰時間來源：`now` 由驅動端傳入，單位秒、單調時鐘）。
//!
//! 外殼視窗的外框終身不變（見 [`super::geometry`]），只有畫出來的形狀在容器內改變。
//! [`Animator`] 記住「目前畫出的值」與目標，每次 `update` 以 [`ANIMATION_SECONDS`]、[`ease_in_out`]
//! 算出這一刻該畫的值。外殼以 `Animator<ShapeLayout>` 同時做外框、黑底與圓角的動畫（同一條曲線、同一段時間），
//! 所以 Preview 歌詞條出現／消失、展開／收合時黑底與圓角都連續變化、不跳。
//!
//! 規則：
//! - 目標改變（例如 Preview → Expanded，或 Preview 的模組額外高度改變）時，新動畫從**目前畫出的值**
//!   開始、重新計 0.28 秒，所以動畫中途反向不會跳。目標不變則不重新起算。
//! - 動畫結束（經過 ≥ 0.28 秒）時回傳的就是目標本身（不是浮點誤差後的近似值）。
//! - 時間容錯同 [`super::hover::HoverMachine`]：`now` 比看過的最大值早，或為 NaN／無限大，視為沒有時間流逝。
//! - 含 NaN／無限大的目標被忽略（沿用原本的目標）；初始值不是有限數時當成預設值（0）。

use super::geometry::{ease_in_out, lerp_size, ShapeLayout, Size, ANIMATION_SECONDS};

/// 可以做動畫的值：線性插值與「是否全為有限數」。
pub trait Animatable: Copy + PartialEq + Default {
    /// `t = 0` 為 `from`、`t = 1` 為 `to`。
    fn lerp(from: Self, to: Self, t: f64) -> Self;
    /// 所有欄位都是有限數。
    fn is_finite(&self) -> bool;
}

impl Animatable for Size {
    fn lerp(from: Self, to: Self, t: f64) -> Self {
        lerp_size(from, to, t)
    }
    fn is_finite(&self) -> bool {
        self.w.is_finite() && self.h.is_finite()
    }
}

impl Animatable for ShapeLayout {
    fn lerp(from: Self, to: Self, t: f64) -> Self {
        ShapeLayout {
            outer: lerp_size(from.outer, to.outer, t),
            fill: lerp_size(from.fill, to.fill, t),
            radius: from.radius + (to.radius - from.radius) * t,
        }
    }
    fn is_finite(&self) -> bool {
        self.outer.is_finite() && self.fill.is_finite() && self.radius.is_finite()
    }
}

/// 畫出來的值（大小或整個外形）的動畫器。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Animator<T: Animatable = Size> {
    /// 動畫起點（上次改目標時畫出的值）。
    from: T,
    /// 目前的目標。
    target: T,
    /// 上次改目標的時間；`None` = 已靜止。
    start: Option<f64>,
    /// 上次 `update` 算出的值。
    current: T,
    /// 看過的最大有限 `now`。
    last_now: Option<f64>,
}

impl<T: Animatable> Animator<T> {
    /// 以 `initial` 為已靜止的值建立（沒有進行中的動畫）。
    pub fn new(initial: T) -> Self {
        let initial = if initial.is_finite() {
            initial
        } else {
            T::default()
        };
        Self {
            from: initial,
            target: initial,
            start: None,
            current: initial,
            last_now: None,
        }
    }

    /// 容錯後的時間：不倒退；NaN／無限大視為沒有時間流逝；第一次就不合法時當 0。
    fn effective_now(&mut self, now: f64) -> f64 {
        let last = self
            .last_now
            .unwrap_or(if now.is_finite() { now } else { 0.0 });
        let effective = if now.is_finite() { now.max(last) } else { last };
        self.last_now = Some(effective);
        effective
    }

    /// 某時刻、依目前起點與目標應畫出的值。
    fn sample(&self, now: f64) -> T {
        let Some(start) = self.start else {
            return self.target;
        };
        let t = (now - start) / ANIMATION_SECONDS;
        if t >= 1.0 {
            self.target
        } else {
            T::lerp(self.from, self.target, ease_in_out(t))
        }
    }

    /// 在 `now`（秒）取目前應畫出的值。`target` 和上次不同時，從這一刻畫出的值重新起算動畫。
    pub fn update(&mut self, now: f64, target: T) -> T {
        let now = self.effective_now(now);
        if target.is_finite() && target != self.target {
            // 先以舊目標算出此刻畫出的值，當作新動畫的起點（不跳）。
            self.from = self.sample(now);
            self.target = target;
            self.start = Some(now);
        }
        self.current = self.sample(now);
        if self.current == self.target {
            self.start = None;
            self.from = self.target;
        }
        self.current
    }

    /// 上次 `update` 算出的值。
    pub fn current(&self) -> T {
        self.current
    }

    /// 目前的目標。
    pub fn target(&self) -> T {
        self.target
    }

    /// 動畫是否還在進行（上次算出的值還不是目標）。驅動端據此決定要不要繼續逐格重繪。
    pub fn is_animating(&self) -> bool {
        self.current != self.target
    }

    /// 不做動畫，直接停在 `value`（換螢幕、隱藏時用）。不是有限數時忽略。
    pub fn snap_to(&mut self, value: T) {
        if value.is_finite() {
            self.from = value;
            self.target = value;
            self.current = value;
            self.start = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::{ease_in_out, lerp_size, ANIMATION_SECONDS};
    use super::*;

    const PREVIEW: Size = Size { w: 345.0, h: 32.0 };
    const EXPANDED: Size = Size { w: 345.0, h: 180.0 };

    fn close(a: Size, b: Size) -> bool {
        (a.w - b.w).abs() < 1e-9 && (a.h - b.h).abs() < 1e-9
    }

    #[test]
    fn new_animator_is_settled_at_the_initial_size() {
        let mut a = Animator::new(PREVIEW);
        assert_eq!(a.current(), PREVIEW);
        assert!(!a.is_animating());
        assert_eq!(a.update(10.0, PREVIEW), PREVIEW);
        assert!(!a.is_animating());
    }

    #[test]
    fn retarget_animates_with_ease_in_out_over_280ms() {
        let mut a = Animator::new(PREVIEW);
        assert_eq!(a.update(1.0, EXPANDED), PREVIEW);
        assert!(a.is_animating());
        for frac in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let got = a.update(1.0 + frac * ANIMATION_SECONDS, EXPANDED);
            let want = lerp_size(PREVIEW, EXPANDED, ease_in_out(frac));
            assert!(close(got, want), "frac={frac} got={got:?} want={want:?}");
            assert!(a.is_animating());
        }
    }

    #[test]
    fn finished_animation_returns_the_target_exactly() {
        let mut a = Animator::new(PREVIEW);
        a.update(0.0, EXPANDED);
        assert_eq!(a.update(ANIMATION_SECONDS, EXPANDED), EXPANDED);
        assert!(!a.is_animating());
        assert_eq!(a.update(100.0, EXPANDED), EXPANDED);
        assert_eq!(a.current(), EXPANDED);
    }

    #[test]
    fn retarget_mid_animation_starts_from_the_current_drawn_size() {
        let mut a = Animator::new(PREVIEW);
        a.update(0.0, EXPANDED);
        let mid = a.update(0.14, EXPANDED);
        assert!(mid.h > PREVIEW.h && mid.h < EXPANDED.h);
        // 同一時刻改回 Preview：不跳，起點就是剛剛畫出的大小。
        let same_instant = a.update(0.14, PREVIEW);
        assert!(close(same_instant, mid), "{same_instant:?} vs {mid:?}");
        // 新動畫從 mid 開始，再花完整的 0.28 秒回到 Preview。
        let later = a.update(0.14 + 0.5 * ANIMATION_SECONDS, PREVIEW);
        assert!(close(later, lerp_size(mid, PREVIEW, 0.5)));
        assert!(a.is_animating());
        assert_eq!(a.update(0.14 + ANIMATION_SECONDS, PREVIEW), PREVIEW);
        assert!(!a.is_animating());
    }

    #[test]
    fn retarget_to_the_same_target_does_not_restart() {
        let mut a = Animator::new(PREVIEW);
        a.update(0.0, EXPANDED);
        let at_half = a.update(0.14, EXPANDED);
        // 再次給同一個目標不會重新起算：到 0.28 一樣結束。
        assert!(close(a.update(0.14, EXPANDED), at_half));
        assert_eq!(a.update(ANIMATION_SECONDS, EXPANDED), EXPANDED);
    }

    #[test]
    fn rapid_back_and_forth_never_jumps() {
        let mut a = Animator::new(PREVIEW);
        let mut now = 0.0;
        let mut prev = a.current();
        for i in 0..200 {
            let target = if (i / 3) % 2 == 0 { EXPANDED } else { PREVIEW };
            now += 0.016;
            let got = a.update(now, target);
            // 每 16 ms 最多走完整段距離的一小部分（ease-in-out 斜率上限 2 → 16/280*2 ≈ 0.115）。
            let max_step = (EXPANDED.h - PREVIEW.h) * 0.016 / ANIMATION_SECONDS * 2.0 + 1e-9;
            assert!(
                (got.h - prev.h).abs() <= max_step,
                "jump at i={i}: {prev:?} -> {got:?}"
            );
            assert!(got.h >= PREVIEW.h - 1e-9 && got.h <= EXPANDED.h + 1e-9);
            prev = got;
        }
    }

    #[test]
    fn clock_going_backwards_counts_as_no_time_passing() {
        let mut a = Animator::new(PREVIEW);
        a.update(10.0, EXPANDED);
        let mid = a.update(10.14, EXPANDED);
        // 時間倒退：停在原地，不倒退、不 panic。
        assert!(close(a.update(5.0, EXPANDED), mid));
        assert!(close(a.update(-1e300, EXPANDED), mid));
        // 之後以「看過的最大時間」繼續。
        assert_eq!(a.update(10.0 + ANIMATION_SECONDS, EXPANDED), EXPANDED);
        // 倒退中改目標：從目前大小、以最大時間為起點。
        let mut a = Animator::new(PREVIEW);
        a.update(10.0, EXPANDED);
        let mid = a.update(10.14, EXPANDED);
        assert!(close(a.update(3.0, PREVIEW), mid));
        assert_eq!(a.update(10.14 + ANIMATION_SECONDS, PREVIEW), PREVIEW);
    }

    #[test]
    fn nan_and_infinite_time_are_safe() {
        let mut a = Animator::new(PREVIEW);
        // 第一次就是 NaN：視為時間 0。
        assert_eq!(a.update(f64::NAN, EXPANDED), PREVIEW);
        assert!(a.is_animating());
        let mid = a.update(0.14, EXPANDED);
        assert!(close(a.update(f64::NAN, EXPANDED), mid));
        assert!(close(a.update(f64::INFINITY, EXPANDED), mid));
        assert!(close(a.update(f64::NEG_INFINITY, EXPANDED), mid));
        assert_eq!(a.update(ANIMATION_SECONDS, EXPANDED), EXPANDED);
    }

    #[test]
    fn non_finite_targets_are_ignored() {
        let mut a = Animator::new(PREVIEW);
        a.update(0.0, EXPANDED);
        let mid = a.update(0.14, EXPANDED);
        let bad = Size {
            w: f64::NAN,
            h: 10.0,
        };
        // 壞目標被忽略：照原本的動畫繼續。
        assert!(close(a.update(0.14, bad), mid));
        assert_eq!(
            a.update(
                ANIMATION_SECONDS,
                Size {
                    w: 1.0,
                    h: f64::INFINITY
                }
            ),
            EXPANDED
        );
        // 新的 Animator 用壞的初始值也不 panic（退回 0×0）。
        let mut z = Animator::new(bad);
        assert_eq!(z.current(), Size::default());
        assert_eq!(z.update(0.0, PREVIEW), Size::default());
        assert_eq!(z.update(ANIMATION_SECONDS, PREVIEW), PREVIEW);
    }

    #[test]
    fn snap_to_stops_any_animation() {
        let mut a = Animator::new(PREVIEW);
        a.update(0.0, EXPANDED);
        a.update(0.1, EXPANDED);
        a.snap_to(Size { w: 360.0, h: 32.0 });
        assert!(!a.is_animating());
        assert_eq!(a.current(), Size { w: 360.0, h: 32.0 });
        assert_eq!(
            a.update(0.2, Size { w: 360.0, h: 32.0 }),
            Size { w: 360.0, h: 32.0 }
        );
    }

    mod shape {
        use super::super::super::geometry::{shape_layout, NotchGeometry, ShellMode};
        use super::*;

        fn g() -> NotchGeometry {
            NotchGeometry {
                notch: Size { w: 185.0, h: 32.0 },
                center_x: 756.0,
                top_y: 0.0,
            }
        }

        /// 以 120 fps 跑完一段動畫，回傳每格的外形。
        fn run(a: &mut Animator<ShapeLayout>, t0: f64, target: ShapeLayout) -> Vec<ShapeLayout> {
            (0..=40)
                .map(|i| a.update(t0 + i as f64 / 120.0, target))
                .collect()
        }

        fn assert_continuous(frames: &[ShapeLayout]) {
            // ease-in-out 斜率上限約 1.72：每 1/120 秒最多走全程的 1.72 / (120 * 0.28) ≈ 5.2%。
            let k = 1.75 / (120.0 * ANIMATION_SECONDS);
            for w in frames.windows(2) {
                let (a, b) = (w[0], w[1]);
                assert!((a.outer.h - b.outer.h).abs() <= 180.0 * k, "{a:?} -> {b:?}");
                assert!((a.fill.h - b.fill.h).abs() <= 180.0 * k, "{a:?} -> {b:?}");
                assert!((a.radius - b.radius).abs() <= 24.0 * k, "{a:?} -> {b:?}");
                assert!(b.fill.h <= b.outer.h + 1e-9);
            }
        }

        #[test]
        fn strip_appearing_in_preview_keeps_the_black_band_and_fades_the_radius_to_square() {
            let plain = shape_layout(ShellMode::Preview, &g(), 0.0);
            let strip = shape_layout(ShellMode::Preview, &g(), 24.0);
            let mut a = Animator::new(plain);
            let frames = run(&mut a, 1.0, strip);
            assert_continuous(&frames);
            // 黑底一直停在瀏海高度，不跳；外框 32 → 56；圓角 10 → 0。
            assert!(frames.iter().all(|f| f.fill.h == 32.0));
            assert_eq!(frames[0], plain);
            assert_eq!(*frames.last().unwrap(), strip);
            // 消失時反過來。
            let frames = run(&mut a, 3.0, plain);
            assert_continuous(&frames);
            assert_eq!(*frames.last().unwrap(), plain);
        }

        #[test]
        fn expanding_and_collapsing_with_a_strip_animate_the_black_band_continuously() {
            let strip = shape_layout(ShellMode::Preview, &g(), 24.0);
            let expanded = shape_layout(ShellMode::Expanded, &g(), 24.0);
            let mut a = Animator::new(strip);
            let frames = run(&mut a, 1.0, expanded);
            assert_continuous(&frames);
            assert_eq!(frames[0], strip);
            assert_eq!(*frames.last().unwrap(), expanded);
            let frames = run(&mut a, 2.0, strip);
            assert_continuous(&frames);
            assert_eq!(*frames.last().unwrap(), strip);
        }

        #[test]
        fn retargeting_mid_way_between_all_shapes_never_jumps() {
            let shapes = [
                shape_layout(ShellMode::Preview, &g(), 0.0),
                shape_layout(ShellMode::Preview, &g(), 24.0),
                shape_layout(ShellMode::Expanded, &g(), 0.0),
                shape_layout(ShellMode::Preview, &g(), 28.0),
            ];
            let mut a = Animator::new(shapes[0]);
            let mut frames = vec![a.current()];
            let mut now = 0.0;
            for i in 0..400 {
                now += 1.0 / 120.0;
                // 每 7 格換一次目標（遠短於 0.28 秒）。
                frames.push(a.update(now, shapes[(i / 7) % shapes.len()]));
            }
            assert_continuous(&frames);
        }
    }
}
