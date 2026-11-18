//! 呈現規則（移植自 Swift `MusicPresentation.swift`），與畫面分開以便測試。
//!
//! 時間一律是 Unix 秒（`f64`）。

use crate::lyrics::Lyrics;
use crate::types::TrackSnapshot;

/// 歌詞條要顯示什麼。
#[derive(Clone, PartialEq, Debug)]
pub enum StripContent {
    /// 目前的歌詞行。
    Line(String),
    /// 曲名；沒有目前行可顯示時一律用它。
    Title(String),
    /// 沒有在播放任何東西。
    None,
}

/// `now` 時的播放位置；來源沒有回報 anchor 時為 `None`。位置未知時絕不拿 0 充數。
pub fn playback_position(s: &TrackSnapshot, now: f64) -> Option<f64> {
    s.position.map(|a| a.position_at(now, s.duration))
}

/// 資訊越少，歌詞條越往下退：帶時間軸的歌詞加上已知位置才顯示目前行；
/// 其餘有曲目時顯示曲名（純文字歌詞與尚未到第一行都不進歌詞條）；沒有曲目就是 `None`。
pub fn strip_content(s: Option<&TrackSnapshot>, lyrics: Option<&Lyrics>, now: f64) -> StripContent {
    let Some(s) = s else {
        return StripContent::None;
    };
    if let Some(Lyrics::Synced(lines)) = lyrics {
        if let Some(index) = playback_position(s, now).and_then(|p| lyrics?.line_index(p)) {
            return StripContent::Line(lines[index].text.clone());
        }
    }
    StripContent::Title(s.title.clone())
}

/// 進度條能不能畫：需要「有限且大於 0 的曲長」加上位置 anchor，否則比例只是猜的。
/// （相對 Swift 多要求有限：NaN 本來就不大於 0，無限大曲長則會讓比例變成 NaN。）
pub fn has_progress(s: &TrackSnapshot) -> bool {
    s.duration.is_some_and(|d| d.is_finite() && d > 0.0) && s.position.is_some()
}

/// 播放進度，夾在 `0..=1`；`has_progress` 為 false 時為 `None`。
/// 絕不回傳 NaN：若計算結果是 NaN（例如非有限的 anchor 輸入）回傳 `None`，
/// 因為這代表位置不可信，與「位置未知」同樣處理而不是畫出一個猜的比例。
pub fn progress_fraction(s: &TrackSnapshot, now: f64) -> Option<f64> {
    if !has_progress(s) {
        return None;
    }
    let duration = s.duration?;
    let position = playback_position(s, now)?;
    let fraction = position / duration;
    if fraction.is_nan() {
        return None;
    }
    Some(fraction.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::LyricLine;
    use crate::types::PositionAnchor;

    const T0: f64 = 1_000_000.0;

    fn synced() -> Lyrics {
        Lyrics::Synced(vec![
            LyricLine {
                time: 10.0,
                text: "first".into(),
            },
            LyricLine {
                time: 20.0,
                text: "second".into(),
            },
            LyricLine {
                time: 30.0,
                text: "third".into(),
            },
        ])
    }

    fn anchor(elapsed: f64, rate: f64) -> PositionAnchor {
        PositionAnchor {
            elapsed,
            timestamp: T0,
            rate,
        }
    }

    fn snapshot_with(
        position: Option<PositionAnchor>,
        duration: Option<f64>,
        is_playing: bool,
    ) -> TrackSnapshot {
        TrackSnapshot {
            title: "Song".into(),
            artist: Some("Artist".into()),
            album: None,
            duration,
            is_playing,
            position,
            artwork: None,
            source_id: "test".into(),
        }
    }

    fn snapshot(position: Option<PositionAnchor>) -> TrackSnapshot {
        snapshot_with(position, Some(200.0), true)
    }

    fn line(s: &str) -> StripContent {
        StripContent::Line(s.into())
    }

    fn title() -> StripContent {
        StripContent::Title("Song".into())
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn synced_with_position_shows_current_line() {
        let s = snapshot(Some(anchor(12.0, 1.0)));
        assert_eq!(strip_content(Some(&s), Some(&synced()), T0), line("first"));
        // 10 秒後位置是 22 秒：目前行是第二行。
        assert_eq!(
            strip_content(Some(&s), Some(&synced()), T0 + 10.0),
            line("second")
        );
    }

    #[test]
    fn synced_before_first_line_shows_title() {
        let s = snapshot(Some(anchor(3.0, 1.0)));
        assert_eq!(strip_content(Some(&s), Some(&synced()), T0), title());
    }

    #[test]
    fn synced_without_anchor_shows_title() {
        let s = snapshot(None);
        assert_eq!(strip_content(Some(&s), Some(&synced()), T0), title());
    }

    #[test]
    fn plain_lyrics_show_title() {
        let s = snapshot(Some(anchor(12.0, 1.0)));
        assert_eq!(
            strip_content(Some(&s), Some(&Lyrics::Plain("la la la".into())), T0),
            title()
        );
        // 無論位置在哪，純文字都不會進歌詞條。
        assert_eq!(
            strip_content(
                Some(&s),
                Some(&Lyrics::Plain("la la la".into())),
                T0 + 150.0
            ),
            title()
        );
    }

    #[test]
    fn no_lyrics_shows_title() {
        let s = snapshot(Some(anchor(12.0, 1.0)));
        assert_eq!(strip_content(Some(&s), None, T0), title());
    }

    #[test]
    fn no_snapshot_is_none() {
        assert_eq!(strip_content(None, Some(&synced()), T0), StripContent::None);
        assert_eq!(strip_content(None, None, T0), StripContent::None);
    }

    #[test]
    fn paused_anchor_holds_line() {
        let paused = snapshot_with(Some(anchor(22.0, 0.0)), Some(200.0), false);
        assert_eq!(
            strip_content(Some(&paused), Some(&synced()), T0),
            line("second")
        );
        assert_eq!(
            strip_content(Some(&paused), Some(&synced()), T0 + 60.0),
            line("second")
        );
    }

    #[test]
    fn playback_position_advances_and_caps_at_duration() {
        let s = snapshot(Some(anchor(12.0, 1.0)));
        assert_eq!(playback_position(&s, T0 + 5.0), Some(17.0));
        let short = snapshot_with(Some(anchor(12.0, 1.0)), Some(20.0), true);
        assert_eq!(playback_position(&short, T0 + 100.0), Some(20.0));
    }

    #[test]
    fn playback_position_is_nil_without_anchor() {
        assert_eq!(playback_position(&snapshot(None), T0), None);
    }

    #[test]
    fn progress_fraction_follows_position_over_duration() {
        let playing = snapshot_with(Some(anchor(50.0, 1.0)), Some(200.0), true);
        close(progress_fraction(&playing, T0).unwrap(), 0.25);
        close(progress_fraction(&playing, T0 + 50.0).unwrap(), 0.5);
    }

    #[test]
    fn progress_fraction_clamps_to_unit_range() {
        // 位置已被夾在曲長，所以進度條是滿的。
        let late = snapshot_with(Some(anchor(500.0, 1.0)), Some(200.0), true);
        assert_eq!(progress_fraction(&late, T0), Some(1.0));
        // 負的 elapsed 也不會得到負的比例。
        let early = snapshot_with(Some(anchor(-30.0, 1.0)), Some(200.0), true);
        assert_eq!(progress_fraction(&early, T0), Some(0.0));
    }

    #[test]
    fn progress_fraction_needs_both_duration_and_position() {
        let a = anchor(50.0, 1.0);
        assert_eq!(
            progress_fraction(&snapshot_with(None, Some(200.0), true), T0),
            None
        );
        assert_eq!(
            progress_fraction(&snapshot_with(Some(a), None, true), T0),
            None
        );
        assert_eq!(
            progress_fraction(&snapshot_with(Some(a), Some(0.0), true), T0),
            None
        );
        assert!(!has_progress(&snapshot_with(None, Some(200.0), true)));
        assert!(!has_progress(&snapshot_with(Some(a), None, true)));
        assert!(has_progress(&snapshot_with(Some(a), Some(200.0), true)));
    }

    // 以下是 Swift 版沒有、任務簡報額外要求的案例。

    #[test]
    fn zero_duration_has_no_progress() {
        assert!(!has_progress(&snapshot_with(
            Some(anchor(50.0, 1.0)),
            Some(0.0),
            true
        )));
        assert!(!has_progress(&snapshot_with(
            Some(anchor(50.0, 1.0)),
            Some(-5.0),
            true
        )));
    }

    #[test]
    fn unknown_position_never_shows_a_line_even_with_far_future_now() {
        let s = snapshot(None);
        assert_eq!(
            strip_content(Some(&s), Some(&synced()), T0 + 1_000_000.0),
            title()
        );
    }

    #[test]
    fn progress_fraction_never_returns_nan() {
        let a = anchor(50.0, 1.0);
        for duration in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let s = snapshot_with(Some(a), Some(duration), true);
            assert!(!has_progress(&s), "duration {duration} 不該算有進度");
            assert_eq!(progress_fraction(&s, T0), None);
        }
        // 非有限的 anchor 與 now：結果要嘛是 None，要嘛是 0..=1 的有限值。
        let nasty = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        for &e in &nasty {
            for &r in &nasty {
                for &now in &[T0, f64::NAN, f64::INFINITY] {
                    let s = snapshot_with(
                        Some(PositionAnchor {
                            elapsed: e,
                            timestamp: T0,
                            rate: r,
                        }),
                        Some(200.0),
                        true,
                    );
                    if let Some(f) = progress_fraction(&s, now) {
                        assert!(
                            (0.0..=1.0).contains(&f),
                            "{f} 超出範圍 (e={e}, r={r}, now={now})"
                        );
                    }
                }
            }
        }
    }
}
