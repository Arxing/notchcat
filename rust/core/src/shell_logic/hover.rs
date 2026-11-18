//! Hover／模式狀態機（Swift `NotchShellController` + `NotchHostingView` 的純邏輯版）。
//!
//! Swift 靠 `NSTrackingArea` 的 enter／exit 事件；Rust 版每個畫格輪詢全域滑鼠位置、餵進這個機器，
//! 所以這裡要從「樣本」重現同樣可觀察的行為。純函式式：不碰時間、OS、執行緒。
//!
//! # 規則（spec 5.3）
//! - 收合（Preview）時，有效 hover 範圍**只有**目前畫出的 `preview_rect`（含模組額外高度）。
//!   游標在其中 → 立刻展開，且該次更新回報 `just_expanded = true`（外殼要把模組切換列選取重設為
//!   active 模組）。
//! - 展開後有效範圍立刻是整個固定 `container`，游標抖動不會造成離開。游標在 container 內 →
//!   維持展開並取消待收合。
//! - 游標離開有效範圍（或 `mouse == None`，視為不在視窗內）→ 在 `LEAVE_DELAY` 之後才收合。
//!   只有「第一次離開」啟動計時；已在待收合時再離開不會把期限往後推；重新進入就取消，之後再離開
//!   重新計 150 ms。
//! - 全螢幕前景：`visible = false`、強制 Preview、清掉待收合，且當作滑鼠在外面（隱藏時不展開）。
//!   離開全螢幕後面板重新出現、為 Preview；從下一個樣本起照常運作（若游標剛好在 `preview_rect`
//!   內就會展開，這是刻意接受的）。
//!
//! # 狀態圖
//! ```text
//!            mouse in preview_rect                       mouse in container
//!  Preview ───────────────────────────▶ Expanded ◀───────(cancel pending)──┐
//!     ▲        (just_expanded = true)    │   ▲                              │
//!     │                                  │   └──────────────────────────────┘
//!     │   now >= leave + 0.150           ▼
//!     └──────────────────────── Expanded + pending(leave)
//!                                (mouse outside container / None; first exit only)
//!  任何狀態 ── fullscreen_foreground ──▶ Preview, visible = false, pending 清除
//! ```
//!
//! # 細節
//! - 期限判定「先於」hover 判定：`now >= leave + LEAVE_DELAY` 時 Swift 的 `DispatchWorkItem`
//!   已經跑過了，所以先收合；同一次樣本若游標在 `preview_rect` 內，緊接著重新展開
//!   （`just_expanded = true`，與 Swift 先收合後重新 enter 相同）。此時輸出的 `mode` 仍是 Expanded，
//!   但 `just_expanded` 為 true——這是唯一「Expanded → Expanded 卻 just_expanded」的情況，
//!   只發生在驅動端沒有在 `next_deadline` 準時呼叫（取樣稀疏）時。
//! - 時間容錯：`now` 比上次早 → 視為沒有時間流逝（用上次最大的 `now`）；`now` 為 NaN 或無限大 →
//!   同樣視為沒有時間流逝（其餘輸入照常處理）。永遠不會 panic、不會下溢。
//! - 座標 NaN → 視為在範圍外（`Rect::contains` 對 NaN 為 false）。
//! - `preview_rect` 在游標靜止時改變（歌詞條出現／收起，高度 32 ↔ 56）：機器只看當下樣本，
//!   游標在舊範圍下方、新範圍內 → 收合時展開一次，之後由 container 管（不會展開→收合→展開）；
//!   游標在舊範圍內、新（縮小）範圍外 → 收合時就是沒有 hover。展開中 `preview_rect` 的變化無關。

use super::geometry::{Point, Rect, ShellMode};

/// 離開有效範圍後延遲多久才收合（秒）。Swift `NotchShellController.collapseDelay = 0.15`。
pub const LEAVE_DELAY: f64 = 0.150;

/// 一次取樣。座標皆為固定容器自己的座標系（原點在容器左上角、y 向下）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HoverInput {
    /// 現在時間（秒，單調時鐘）。
    pub now: f64,
    /// 游標位置；`None` = 游標不在我們的視窗內（外面）。
    pub mouse: Option<Point>,
    /// 目前畫出來的 Preview 範圍（含模組額外高度）。
    pub preview_rect: Rect,
    /// 整個固定容器。
    pub container: Rect,
    /// 有全螢幕 App 在前景。
    pub fullscreen_foreground: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HoverOutput {
    pub mode: ShellMode,
    /// 面板是否可見（全螢幕前景時為 false）。
    pub visible: bool,
    /// 這一次更新才剛（重新）展開：外殼要把切換列選取重設為 active 模組。
    pub just_expanded: bool,
    /// 需要在此時間（含）再呼叫一次 `update`（待收合中才有）。
    pub next_deadline: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct HoverMachine {
    expanded: bool,
    /// 第一次離開有效範圍的時間；`Some` = 收合計時中。
    leave_time: Option<f64>,
    /// 目前為止看過的最大有限 `now`。
    last_now: Option<f64>,
}

impl HoverMachine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 容錯後的時間：不倒退；NaN／無限大視為沒有時間流逝。
    fn effective_now(&mut self, now: f64) -> f64 {
        let last = self.last_now.unwrap_or(0.0);
        let effective = if now.is_finite() { now.max(last) } else { last };
        self.last_now = Some(effective);
        effective
    }

    /// 餵一個樣本。驅動端每個畫格都呼叫，並且在 `next_deadline` 到時再呼叫一次。
    pub fn update(&mut self, input: HoverInput) -> HoverOutput {
        let now = self.effective_now(input.now);

        if input.fullscreen_foreground {
            // 隱藏中：強制 Preview、清掉待收合；滑鼠當作在外面，所以不展開。
            self.expanded = false;
            self.leave_time = None;
            return HoverOutput {
                mode: ShellMode::Preview,
                visible: false,
                just_expanded: false,
                next_deadline: None,
            };
        }

        // 先判期限：到期時 Swift 的 DispatchWorkItem 已經跑過了（收合），之後的樣本才算數。
        if self.expanded {
            if let Some(leave) = self.leave_time {
                if now >= leave + LEAVE_DELAY {
                    self.expanded = false;
                    self.leave_time = None;
                }
            }
        }

        // 有效 hover 範圍：收合只看目前畫出的 Preview，展開立刻是整個容器。
        let active = if self.expanded {
            input.container
        } else {
            input.preview_rect
        };
        let hovering = input.mouse.is_some_and(|m| active.contains(m));

        let mut just_expanded = false;
        if self.expanded {
            if hovering {
                // 重新進入：取消待收合。
                self.leave_time = None;
            } else if self.leave_time.is_none() {
                // 只有第一次離開才啟動計時，之後再離開不會把期限往後推。
                self.leave_time = Some(now);
            }
        } else if hovering {
            self.expanded = true;
            self.leave_time = None;
            just_expanded = true;
        }

        HoverOutput {
            mode: if self.expanded {
                ShellMode::Expanded
            } else {
                ShellMode::Preview
            },
            visible: true,
            just_expanded,
            next_deadline: self.leave_time.map(|leave| leave + LEAVE_DELAY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 360.0,
        h: 180.0,
    };

    fn preview(h: f64) -> Rect {
        Rect {
            x: 80.0,
            y: 0.0,
            w: 200.0,
            h,
        }
    }

    fn p(x: f64, y: f64) -> Option<Point> {
        Some(Point { x, y })
    }

    fn input(now: f64, mouse: Option<Point>) -> HoverInput {
        HoverInput {
            now,
            mouse,
            preview_rect: preview(32.0),
            container: CONTAINER,
            fullscreen_foreground: false,
        }
    }

    /// 在 preview 內／container 內但 preview 外／container 外的代表點。
    const IN_PREVIEW: (f64, f64) = (100.0, 10.0);
    const IN_CONTAINER_ONLY: (f64, f64) = (100.0, 100.0);
    const OUTSIDE: (f64, f64) = (500.0, 100.0);

    fn at(pt: (f64, f64)) -> Option<Point> {
        p(pt.0, pt.1)
    }

    fn expanded_machine(t: f64) -> HoverMachine {
        let mut m = HoverMachine::new();
        let out = m.update(input(t, at(IN_PREVIEW)));
        assert_eq!(out.mode, ShellMode::Expanded);
        m
    }

    #[test]
    fn cursor_inside_container_but_outside_preview_does_not_expand() {
        let mut m = HoverMachine::new();
        let out = m.update(input(0.0, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(out.visible);
        assert!(!out.just_expanded);
        assert_eq!(out.next_deadline, None);
    }

    #[test]
    fn cursor_inside_preview_expands_and_reports_just_expanded_once() {
        let mut m = HoverMachine::new();
        let first = m.update(input(0.0, at(IN_PREVIEW)));
        assert_eq!(first.mode, ShellMode::Expanded);
        assert!(first.just_expanded);
        assert_eq!(first.next_deadline, None);
        let second = m.update(input(0.016, at(IN_PREVIEW)));
        assert_eq!(second.mode, ShellMode::Expanded);
        assert!(!second.just_expanded);
    }

    #[test]
    fn after_expanding_the_whole_container_keeps_it_open() {
        let mut m = expanded_machine(0.0);
        // 容器內、preview 外：展開後有效範圍已是整個容器。
        for i in 1..=20 {
            let out = m.update(input(i as f64 * 0.5, at(IN_CONTAINER_ONLY)));
            assert_eq!(out.mode, ShellMode::Expanded);
            assert!(!out.just_expanded);
            assert_eq!(out.next_deadline, None);
        }
    }

    #[test]
    fn leaving_collapses_only_after_150ms() {
        let t = 10.0;
        let mut m = expanded_machine(t - 1.0);
        let out = m.update(input(t, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(t + 0.150));
        let out = m.update(input(t + 0.149, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(t + 0.150));
        let out = m.update(input(t + 0.150, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(out.visible);
        assert!(!out.just_expanded);
        assert_eq!(out.next_deadline, None);
    }

    #[test]
    fn re_entering_within_150ms_cancels_the_collapse() {
        let t = 5.0;
        let mut m = expanded_machine(t - 1.0);
        m.update(input(t, at(OUTSIDE)));
        let out = m.update(input(t + 0.100, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert!(!out.just_expanded);
        assert_eq!(out.next_deadline, None);
        // 原本的期限過了也不會收合。
        let out = m.update(input(t + 0.200, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, None);
    }

    #[test]
    fn jitter_across_the_container_edge_does_not_flicker() {
        let mut m = expanded_machine(0.0);
        let inside = p(359.0, 90.0);
        let outside = p(361.0, 90.0);
        let mut now = 1.0;
        let mut just_expanded_count = 1; // 初次展開
        for _ in 0..3 {
            let out = m.update(input(now, outside));
            assert_eq!(out.mode, ShellMode::Expanded);
            now += 0.080;
            let out = m.update(input(now, inside));
            assert_eq!(out.mode, ShellMode::Expanded);
            assert_eq!(out.next_deadline, None);
            just_expanded_count += out.just_expanded as u32;
            now += 0.080;
        }
        assert_eq!(just_expanded_count, 1);
    }

    #[test]
    fn preview_height_change_under_a_stationary_cursor_expands_at_most_once() {
        let mut m = HoverMachine::new();
        let cursor = p(100.0, 40.0); // 在 32 高的 preview 之下、56 高的 preview 之內
        let mut expansions = 0;
        let mut modes = Vec::new();
        let mut now = 0.0;
        // 歌詞條出現前：游標在 preview 外，不展開。
        for _ in 0..5 {
            let out = m.update(input(now, cursor));
            expansions += out.just_expanded as u32;
            modes.push(out.mode);
            now += 0.016;
        }
        assert_eq!(modes, vec![ShellMode::Preview; 5]);
        // preview 高度 32 -> 56，游標靜止。
        for _ in 0..200 {
            let mut i = input(now, cursor);
            i.preview_rect = preview(56.0);
            let out = m.update(i);
            expansions += out.just_expanded as u32;
            modes.push(out.mode);
            now += 0.016;
        }
        assert_eq!(expansions, 1);
        // 之後從頭到尾 Expanded，沒有展開→收合→展開。
        assert!(modes[5..].iter().all(|m| *m == ShellMode::Expanded));
        // 歌詞條收起（56 -> 32）：展開中 preview_rect 無關，維持展開，不再有 just_expanded。
        for _ in 0..50 {
            let out = m.update(input(now, cursor));
            expansions += out.just_expanded as u32;
            assert_eq!(out.mode, ShellMode::Expanded);
            now += 0.016;
        }
        assert_eq!(expansions, 1);
    }

    #[test]
    fn preview_shrinking_while_collapsed_leaves_a_cursor_below_the_new_rect_not_hovering() {
        // 游標在 y = 70：56 高與 32 高的 preview 都不含它。歌詞條收起（56 -> 32）時，
        // 收合狀態下沒有任何東西會展開，也不會有 just_expanded。
        let cursor = p(100.0, 70.0);
        let mut m = HoverMachine::new();
        let mut i = input(0.0, cursor);
        i.preview_rect = preview(56.0);
        assert_eq!(m.update(i).mode, ShellMode::Preview);
        let out = m.update(input(0.1, cursor));
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
    }

    #[test]
    fn fullscreen_foreground_hides_and_forces_preview() {
        let mut m = expanded_machine(0.0);
        // 先讓它進入待收合，確認全螢幕會清掉。
        m.update(input(1.0, at(OUTSIDE)));
        let mut i = input(1.05, at(IN_PREVIEW));
        i.fullscreen_foreground = true;
        let out = m.update(i);
        assert!(!out.visible);
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
        assert_eq!(out.next_deadline, None);
        // 隱藏期間游標在 preview 內也不展開。
        let mut i = input(2.0, at(IN_PREVIEW));
        i.fullscreen_foreground = true;
        let out = m.update(i);
        assert!(!out.visible);
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
    }

    #[test]
    fn leaving_fullscreen_shows_again_in_preview() {
        let mut m = expanded_machine(0.0);
        let mut i = input(1.0, at(OUTSIDE));
        i.fullscreen_foreground = true;
        m.update(i);
        let out = m.update(input(2.0, at(OUTSIDE)));
        assert!(out.visible);
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
        assert_eq!(out.next_deadline, None);
        // 離開全螢幕時游標剛好在 preview 內：從這個樣本起照常運作，會展開。
        let mut m = expanded_machine(0.0);
        let mut i = input(1.0, at(IN_PREVIEW));
        i.fullscreen_foreground = true;
        m.update(i);
        let out = m.update(input(1.1, at(IN_PREVIEW)));
        assert!(out.visible);
        assert_eq!(out.mode, ShellMode::Expanded);
        assert!(out.just_expanded);
    }

    #[test]
    fn mouse_none_counts_as_outside() {
        let t = 3.0;
        let mut m = expanded_machine(t - 1.0);
        let out = m.update(input(t, None));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(t + 0.150));
        let out = m.update(input(t + 0.150, None));
        assert_eq!(out.mode, ShellMode::Preview);
        // 收合時 None 不會展開。
        let out = m.update(input(t + 1.0, None));
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
    }

    #[test]
    fn second_exit_after_a_cancelled_collapse_starts_a_fresh_150ms() {
        let mut m = expanded_machine(0.0);
        m.update(input(1.0, at(OUTSIDE)));
        m.update(input(1.100, at(IN_CONTAINER_ONLY))); // 取消
        let out = m.update(input(1.140, at(OUTSIDE))); // 重新離開
        assert_eq!(out.next_deadline, Some(1.140 + 0.150));
        // 舊期限（1.150）到了也不收合。
        let out = m.update(input(1.150, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        let out = m.update(input(1.140 + 0.149, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        let out = m.update(input(1.140 + 0.150, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Preview);
    }

    #[test]
    fn exit_while_already_pending_does_not_extend_the_deadline() {
        let mut m = expanded_machine(0.0);
        let first = m.update(input(1.0, at(OUTSIDE)));
        assert_eq!(first.next_deadline, Some(1.0 + 0.150));
        // 在待收合中游標換到別處（仍在外面）、或回報 None，期限都不變。
        let out = m.update(input(1.05, p(600.0, 300.0)));
        assert_eq!(out.next_deadline, Some(1.0 + 0.150));
        let out = m.update(input(1.10, None));
        assert_eq!(out.next_deadline, Some(1.0 + 0.150));
        let out = m.update(input(1.0 + 0.150, None));
        assert_eq!(out.mode, ShellMode::Preview);
    }

    #[test]
    fn collapse_then_immediately_re_entering_preview_re_expands_with_just_expanded_again() {
        let mut m = expanded_machine(0.0);
        m.update(input(1.0, at(OUTSIDE)));
        let out = m.update(input(1.150, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Preview);
        // 收合後的 hover 範圍又是 preview_rect：容器內但 preview 外不展開。
        let out = m.update(input(1.160, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Preview);
        let out = m.update(input(1.170, at(IN_PREVIEW)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert!(out.just_expanded);
    }

    #[test]
    fn stationary_cursor_repeated_updates_produce_no_repeated_just_expanded() {
        let mut m = HoverMachine::new();
        let mut count = 0;
        for i in 0..1000 {
            let out = m.update(input(i as f64 * 0.016, at(IN_PREVIEW)));
            count += out.just_expanded as u32;
            assert_eq!(out.mode, ShellMode::Expanded);
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn sparse_sampling_past_the_deadline_collapses_first_then_applies_the_new_sample() {
        // 取樣稀疏：離開後下一個樣本已超過期限，游標又回到 preview 內。
        // Swift 的 DispatchWorkItem 早已觸發（收合），之後的 enter 是一次全新的展開。
        let mut m = expanded_machine(0.0);
        m.update(input(1.0, at(OUTSIDE)));
        let out = m.update(input(1.5, at(IN_PREVIEW)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert!(out.just_expanded);
        assert_eq!(out.next_deadline, None);
        // 同樣情形但回到的是「容器內、preview 外」：已收合，不展開。
        let mut m = expanded_machine(0.0);
        m.update(input(1.0, at(OUTSIDE)));
        let out = m.update(input(1.5, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Preview);
        assert!(!out.just_expanded);
    }

    #[test]
    fn deadline_wins_a_tie_with_re_entry_at_exactly_150ms() {
        let mut m = expanded_machine(0.0);
        m.update(input(1.0, at(OUTSIDE)));
        let out = m.update(input(1.0 + 0.150, at(IN_CONTAINER_ONLY)));
        assert_eq!(out.mode, ShellMode::Preview);
    }

    #[test]
    fn non_monotonic_now_counts_as_no_time_passing() {
        let mut m = expanded_machine(10.0);
        let out = m.update(input(20.0, at(OUTSIDE)));
        assert_eq!(out.next_deadline, Some(20.0 + 0.150));
        // 時間倒退：不收合、期限不變、不 panic。
        let out = m.update(input(5.0, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(20.0 + 0.150));
        let out = m.update(input(-1.0e300, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        // 倒退之後的離開仍用最大的 now 當離開時間。
        let mut m = expanded_machine(10.0);
        let out = m.update(input(2.0, at(OUTSIDE)));
        assert_eq!(out.next_deadline, Some(10.0 + 0.150));
        let out = m.update(input(10.149, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Expanded);
        let out = m.update(input(10.150, at(OUTSIDE)));
        assert_eq!(out.mode, ShellMode::Preview);
    }

    #[test]
    fn nan_and_infinite_inputs_are_safe() {
        // NaN 滑鼠座標 = 在外面，不展開。
        let mut m = HoverMachine::new();
        for pt in [p(f64::NAN, 5.0), p(5.0, f64::NAN), p(f64::NAN, f64::NAN)] {
            let out = m.update(input(0.0, pt));
            assert_eq!(out.mode, ShellMode::Preview);
            assert!(!out.just_expanded);
        }
        // 展開中 NaN 滑鼠 = 離開，開始計時。
        let mut m = expanded_machine(1.0);
        let out = m.update(input(2.0, p(f64::NAN, f64::NAN)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(2.0 + 0.150));
        // NaN／無限大的 now：沒有時間流逝（不收合、不改期限）。
        let out = m.update(input(f64::NAN, p(f64::NAN, 0.0)));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(2.0 + 0.150));
        let out = m.update(input(f64::INFINITY, None));
        assert_eq!(out.mode, ShellMode::Expanded);
        assert_eq!(out.next_deadline, Some(2.0 + 0.150));
        let out = m.update(input(f64::NEG_INFINITY, None));
        assert_eq!(out.mode, ShellMode::Expanded);
        // 之後正常時間照常收合（沒有被無限大卡死）。
        let out = m.update(input(2.150, None));
        assert_eq!(out.mode, ShellMode::Preview);
        // 第一個樣本就是 NaN now 也不 panic。
        let mut m = HoverMachine::new();
        let out = m.update(input(f64::NAN, at(IN_PREVIEW)));
        assert_eq!(out.mode, ShellMode::Expanded);
        // NaN 的矩形 = 沒有 hover。
        let mut m = HoverMachine::new();
        let mut i = input(0.0, at(IN_PREVIEW));
        i.preview_rect = Rect {
            x: f64::NAN,
            ..preview(32.0)
        };
        assert_eq!(m.update(i).mode, ShellMode::Preview);
    }

    /// 極簡 LCG（不引入外部 crate），固定種子可重現。
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn unit(&mut self) -> f64 {
            (self.next() % 1_000_000) as f64 / 1_000_000.0
        }
    }

    #[test]
    fn soak_random_samples_hold_the_invariants() {
        let mut rng = Lcg(0x5EED_CAFE_F00D_0001);
        let mut m = HoverMachine::new();
        let mut now = 100.0_f64;
        let mut max_now = now;
        let mut prev: Option<HoverOutput> = None;
        let mut expansions = 0u32;
        let mut collapses = 0u32;
        for _ in 0..10_000 {
            // 時間：多半往前 0..0.3 秒，偶爾倒退或不動。
            now += match rng.next() % 10 {
                0 => -rng.unit() * 2.0,
                1 => 0.0,
                _ => rng.unit() * 0.3,
            };
            let mouse = match rng.next() % 8 {
                0 => None,
                _ => Some(Point {
                    x: -40.0 + rng.unit() * 440.0,
                    y: -20.0 + rng.unit() * 240.0,
                }),
            };
            let preview_h = [32.0, 56.0, 60.0][(rng.next() % 3) as usize];
            let fullscreen = rng.next().is_multiple_of(50);
            max_now = max_now.max(now);
            let out = m.update(HoverInput {
                now,
                mouse,
                preview_rect: preview(preview_h),
                container: CONTAINER,
                fullscreen_foreground: fullscreen,
            });

            // 展開一定可見；全螢幕一定不可見且為 Preview。
            if out.mode == ShellMode::Expanded {
                assert!(out.visible);
            }
            assert_eq!(out.visible, !fullscreen);
            if fullscreen {
                assert_eq!(out.mode, ShellMode::Preview);
                assert!(!out.just_expanded);
                assert_eq!(out.next_deadline, None);
            }
            // 期限只在展開且待收合時存在，且一定在未來（>= 最大 now 的 +0.150 內）。
            if let Some(d) = out.next_deadline {
                assert_eq!(out.mode, ShellMode::Expanded);
                assert!(d.is_finite());
                // 離開時間（= d - 150 ms）不可能晚於目前看過的最大 now。
                assert!(d - LEAVE_DELAY <= max_now + 1e-9);
            }
            // just_expanded 只出現在 Preview -> Expanded，或「已過期限才重新展開」。
            if out.just_expanded {
                assert_eq!(out.mode, ShellMode::Expanded);
                match prev {
                    Some(pv) if pv.mode == ShellMode::Expanded => {
                        let d = pv
                            .next_deadline
                            .expect("expanded->expanded needs a pending deadline");
                        assert!(
                            now >= d,
                            "just_expanded while still expanded and not expired"
                        );
                    }
                    _ => {}
                }
                expansions += 1;
            }
            // 沒有 just_expanded 時，Preview -> Expanded 不可能發生。
            if let Some(pv) = prev {
                if pv.mode == ShellMode::Preview && out.mode == ShellMode::Expanded {
                    assert!(out.just_expanded);
                }
                if pv.mode == ShellMode::Expanded && out.mode == ShellMode::Preview {
                    collapses += 1;
                }
            }
            prev = Some(out);
        }
        // 確認 soak 真的走過了各種轉移（不是空轉）。
        assert!(expansions > 50, "expansions = {expansions}");
        assert!(collapses > 50, "collapses = {collapses}");
    }
}
