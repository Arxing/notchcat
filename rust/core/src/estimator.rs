//! 為無法回報位置的來源估算播放位置（移植自 Swift `PlaybackEstimator.swift`）。
//!
//! 只有在觀察期間看到換曲，才知道位置：新曲目從第一次看到的那一刻起算 0。
//! 觀察開始時就已在播放的曲目起點未知，`anchor()` 對它回傳 `None`，絕不是 0。
//! 暫停凍結位置，恢復後從凍結處續算。

use crate::types::PositionAnchor;

#[derive(Debug, Default)]
pub struct PlaybackEstimator {
    current_title: Option<String>,
    current: Option<PositionAnchor>,
}

impl PlaybackEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, title: Option<&str>, is_playing: bool, now: f64) {
        let Some(title) = title else {
            self.current_title = None;
            self.current = None;
            return;
        };

        if self.current_title.as_deref() != Some(title) {
            let is_change = self.current_title.is_some();
            self.current_title = Some(title.to_owned());
            self.current = is_change.then_some(PositionAnchor {
                elapsed: 0.0,
                timestamp: now,
                rate: if is_playing { 1.0 } else { 0.0 },
            });
            return;
        }

        let Some(anchor) = self.current else { return };
        let rate = if is_playing { 1.0 } else { 0.0 };
        if rate == anchor.rate {
            return;
        }
        self.current = Some(PositionAnchor {
            elapsed: anchor.position_at(now, None),
            timestamp: now,
            rate,
        });
    }

    /// 無法估算位置時為 `None`。
    pub fn anchor(&self) -> Option<PositionAnchor> {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: f64 = 1_000_000.0;

    fn a(elapsed: f64, timestamp: f64, rate: f64) -> PositionAnchor {
        PositionAnchor {
            elapsed,
            timestamp,
            rate,
        }
    }

    #[test]
    fn first_observation_has_no_anchor() {
        let mut e = PlaybackEstimator::new();
        assert_eq!(e.anchor(), None);
        e.observe(Some("A"), true, T0);
        assert_eq!(e.anchor(), None);
        e.observe(Some("A"), true, T0 + 30.0);
        assert_eq!(e.anchor(), None);
    }

    #[test]
    fn track_change_while_running_yields_anchor_at_zero() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        let change = T0 + 40.0;
        e.observe(Some("B"), true, change);
        assert_eq!(e.anchor(), Some(a(0.0, change, 1.0)));
        assert_eq!(e.anchor().unwrap().position_at(change + 7.0, None), 7.0);
    }

    #[test]
    fn pause_freezes_position() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        let change = T0 + 10.0;
        e.observe(Some("B"), true, change);
        let pause = change + 25.0;
        e.observe(Some("B"), false, pause);
        let anchor = e.anchor().unwrap();
        assert_eq!(anchor, a(25.0, pause, 0.0));
        assert_eq!(anchor.position_at(pause + 10.0, None), 25.0);
    }

    #[test]
    fn resume_continues_from_frozen_position() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        let change = T0 + 10.0;
        e.observe(Some("B"), true, change);
        let pause = change + 25.0;
        e.observe(Some("B"), false, pause);
        let resume = pause + 60.0;
        e.observe(Some("B"), true, resume);
        let anchor = e.anchor().unwrap();
        assert_eq!(anchor, a(25.0, resume, 1.0));
        assert_eq!(anchor.position_at(resume + 5.0, None), 30.0);
    }

    #[test]
    fn same_track_reobserved_keeps_anchor() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        let change = T0 + 10.0;
        e.observe(Some("B"), true, change);
        let before = e.anchor();
        e.observe(Some("B"), true, change + 1.0);
        e.observe(Some("B"), true, change + 2.0);
        assert_eq!(e.anchor(), before);
    }

    #[test]
    fn unknown_start_stays_unknown_across_pause_and_resume() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        e.observe(Some("A"), false, T0 + 5.0);
        assert_eq!(e.anchor(), None);
        e.observe(Some("A"), true, T0 + 9.0);
        assert_eq!(e.anchor(), None);
    }

    #[test]
    fn no_track_clears_anchor_and_next_track_starts_unknown() {
        let mut e = PlaybackEstimator::new();
        e.observe(Some("A"), true, T0);
        e.observe(Some("B"), true, T0 + 10.0);
        assert!(e.anchor().is_some());
        e.observe(None, false, T0 + 20.0);
        assert_eq!(e.anchor(), None);
        e.observe(Some("C"), true, T0 + 30.0);
        assert_eq!(e.anchor(), None);
    }
}
