//! 曲目快照與播放位置型別（移植自 Swift `NowPlayingSnapshot.swift`）。
//!
//! 時間一律是 Unix 秒（`f64`），取代 Swift 的 `TimeInterval`／`Date`。

use std::sync::Arc;

/// 封面的識別字串，可選擇攜帶位元組。
#[derive(Clone, PartialEq, Debug)]
pub struct ArtworkRef {
    pub id: String,
    pub data: Option<Arc<[u8]>>,
}

/// 某一時刻已知的播放位置，加上它前進的速率。目前位置一律推算，不存放。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PositionAnchor {
    pub elapsed: f64,
    pub timestamp: f64,
    pub rate: f64,
}

impl PositionAnchor {
    /// `max(0, elapsed + max(0, now - timestamp) * rate)`，給定 `cap` 時不超過它。
    /// `now` 早於 `timestamp` 時不會倒退到 `elapsed` 以下。非有限的輸入不會 panic：
    /// NaN 的時間差視為 0（即回傳 `elapsed` 夾到 0），NaN 的 `cap` 視為沒有上限。
    pub fn position_at(&self, now: f64, cap: Option<f64>) -> f64 {
        // `f64::max` 遇到 NaN 會回傳另一個運算元，所以 NaN 的時間差在此變成 0。
        let dt = (now - self.timestamp).max(0.0);
        // 速率為 0 時不乘，避免 `inf * 0` 產生 NaN 而吃掉 `elapsed`。
        let advance = if self.rate == 0.0 {
            0.0
        } else {
            dt * self.rate
        };
        let advanced = (self.elapsed + advance).max(0.0);
        match cap {
            Some(cap) => advanced.min(cap),
            None => advanced,
        }
    }
}

/// 來源回報的目前曲目狀態。
#[derive(Clone, PartialEq, Debug)]
pub struct TrackSnapshot {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
    pub is_playing: bool,
    /// 播放位置未知時為 `None`。
    pub position: Option<PositionAnchor>,
    pub artwork: Option<ArtworkRef>,
    pub source_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: f64 = 1_000.0;

    fn anchor(elapsed: f64, rate: f64) -> PositionAnchor {
        PositionAnchor {
            elapsed,
            timestamp: T,
            rate,
        }
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 0.0001, "{a} != {b}");
    }

    #[test]
    fn advances_with_rate() {
        close(anchor(10.0, 1.0).position_at(T + 5.0, None), 15.0);
    }

    #[test]
    fn honors_non_unit_rate() {
        close(anchor(10.0, 2.0).position_at(T + 5.0, None), 20.0);
    }

    #[test]
    fn paused_freezes() {
        close(anchor(10.0, 0.0).position_at(T + 100.0, None), 10.0);
    }

    #[test]
    fn caps_at_duration() {
        close(
            anchor(230.0, 1.0).position_at(T + 100.0, Some(233.9)),
            233.9,
        );
    }

    #[test]
    fn no_duration_means_no_cap() {
        close(anchor(230.0, 1.0).position_at(T + 100.0, None), 330.0);
    }

    #[test]
    fn timestamp_in_future_does_not_rewind() {
        let p = anchor(10.0, 1.0).position_at(T - 50.0, None);
        close(p, 10.0);
        assert!(p >= 0.0);
    }

    #[test]
    fn negative_elapsed_clamps_to_zero() {
        close(anchor(-5.0, 1.0).position_at(T, None), 0.0);
    }

    #[test]
    fn non_finite_inputs_do_not_panic() {
        close(anchor(10.0, 1.0).position_at(f64::NAN, None), 10.0);
        close(anchor(-5.0, 1.0).position_at(f64::NAN, None), 0.0);
        close(anchor(f64::NAN, 1.0).position_at(T + 5.0, None), 0.0);
        close(anchor(10.0, f64::NAN).position_at(T + 5.0, None), 0.0);
        close(anchor(10.0, 0.0).position_at(f64::INFINITY, None), 10.0);
        close(anchor(10.0, 1.0).position_at(T + 5.0, Some(f64::NAN)), 15.0);
        close(
            anchor(10.0, 1.0).position_at(f64::INFINITY, Some(100.0)),
            100.0,
        );
    }
}
