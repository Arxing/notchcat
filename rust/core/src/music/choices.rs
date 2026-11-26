//! 記住使用者為每首歌選的歌詞版本與時間偏移（Rust 版新增）。
//!
//! 只有純資料與規則；讀寫檔案由 app 負責。
//!
//! 一首歌以 [`TrackIdentity::cache_key`] 加曲長辨認：曲長差 2 秒以內算同一首，未知的曲長
//! （沒有、非有限值、不大於 0）只和同樣未知的配對。所以同名的 MV 版和專輯版分開記。
//!
//! 偏移的單位是秒，正值讓歌詞提早出現。

use std::collections::BTreeMap;

use crate::lyrics::versions::same_version;
use crate::lyrics::{LyricLine, Lyrics, LyricsHit, TrackIdentity};

/// 偏移的上限（秒），提早和延後相同。
pub const OFFSET_LIMIT: f64 = 120.0;
/// 同一首歌曲長差的上限（秒）。
const DURATION_TOLERANCE: f64 = 2.0;
/// 比較曲長差時容許的浮點誤差（秒）。
const TIME_EPSILON: f64 = 1e-6;

/// 一首歌記下的選擇。
#[derive(Clone, PartialEq, Debug)]
pub struct SavedChoice {
    /// [`TrackIdentity::cache_key`]。
    pub key: String,
    /// 曲長（秒）；來源沒有回報時為 `None`。
    pub duration: Option<f64>,
    /// 選定版本的網址（`https://` 開頭）。
    pub url: String,
    /// 選定版本的歌詞行，未套偏移：查詢結果裡找不到該版本時拿來用。
    pub lines: Vec<LyricLine>,
    /// 版本網址對到該版本的偏移（秒）。
    pub offsets: BTreeMap<String, f64>,
}

/// 全部記下的選擇。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ChoiceBook {
    pub songs: Vec<SavedChoice>,
}

/// 兩個曲長算同一首歌時回傳差幾秒（兩邊都未知算 0），否則 `None`。
fn duration_distance(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    let known = |d: Option<f64>| d.filter(|d| d.is_finite() && *d > 0.0);
    match (known(a), known(b)) {
        (Some(a), Some(b)) => {
            Some((a - b).abs()).filter(|gap| *gap <= DURATION_TOLERANCE + TIME_EPSILON)
        }
        (None, None) => Some(0.0),
        _ => None,
    }
}

impl ChoiceBook {
    /// 這首歌記下的選擇；多筆都在容忍內時取曲長最接近的。
    pub fn find(&self, track: &TrackIdentity) -> Option<&SavedChoice> {
        let index = self.position(&track.cache_key(), track.duration)?;
        self.songs.get(index)
    }

    /// 寫入一首歌的選擇：取代同一首歌的那筆（[`Self::find`] 會挑到的那筆），沒有就新增。
    pub fn upsert(&mut self, choice: SavedChoice) {
        match self.position(&choice.key, choice.duration) {
            Some(index) => self.songs[index] = choice,
            None => self.songs.push(choice),
        }
    }

    fn position(&self, key: &str, duration: Option<f64>) -> Option<usize> {
        self.songs
            .iter()
            .enumerate()
            .filter(|(_, song)| song.key == key)
            .filter_map(|(i, song)| Some((i, duration_distance(song.duration, duration)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// 把記住的選擇套到查詢結果上，回傳版本清單與要選的索引。沒有記住的選擇時原樣回傳、選第一個。
///
/// 記住的版本一定被選中，依序：
/// 1. 清單裡有同網址的時間軸版本：選它，內容用新查到的。
/// 2. 有時間軸相同（[`same_version`]）的版本：用存下的版本取代它。
/// 3. 清單是別的時間軸版本：存下的版本附加在最後。
/// 4. 清單是純文字或空的：只剩存下的版本。
pub fn apply_choice(
    mut versions: Vec<LyricsHit>,
    saved: Option<&SavedChoice>,
) -> (Vec<LyricsHit>, usize) {
    let Some(saved) = saved else {
        return (versions, 0);
    };
    // 同網址但這次只剩純文字的紀錄不算找到。
    if let Some(index) = versions
        .iter()
        .position(|hit| hit.url == saved.url && hit.lyrics.is_synced())
    {
        return (versions, index);
    }
    let kept = LyricsHit::new(Lyrics::Synced(saved.lines.clone()), saved.url.clone());
    let same_timing = versions.iter().position(
        |hit| matches!(&hit.lyrics, Lyrics::Synced(lines) if same_version(lines, &saved.lines)),
    );
    if let Some(index) = same_timing {
        versions[index] = kept;
        return (versions, index);
    }
    // 純文字不是版本，不和時間軸版本並列。
    versions.retain(|hit| hit.lyrics.is_synced());
    versions.push(kept);
    let index = versions.len() - 1;
    (versions, index)
}

/// 套用偏移：正值讓歌詞提早出現（每行時間減 `offset`），結果可以是負的。
pub fn shift_lyrics(lines: &[LyricLine], offset: f64) -> Vec<LyricLine> {
    lines
        .iter()
        .map(|line| LyricLine {
            time: line.time - offset,
            text: line.text.clone(),
        })
        .collect()
}

/// 把偏移限制在 ±[`OFFSET_LIMIT`]；NaN 當作沒有偏移。
pub fn clamp_offset(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-OFFSET_LIMIT, OFFSET_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, duration: Option<f64>) -> TrackIdentity {
        TrackIdentity {
            title: title.into(),
            artist: "A".into(),
            album: None,
            duration,
        }
    }

    fn url(id: u32) -> String {
        format!("https://test.example/{id}")
    }

    /// 五行歌詞，整體平移 `shift` 秒：平移超過 1 秒就是另一個版本。
    fn lines(shift: f64) -> Vec<LyricLine> {
        ["一", "二", "三", "四", "五"]
            .iter()
            .enumerate()
            .map(|(i, text)| LyricLine {
                time: 10.0 * i as f64 + shift,
                text: (*text).into(),
            })
            .collect()
    }

    fn hit(id: u32, shift: f64) -> LyricsHit {
        LyricsHit::new(Lyrics::Synced(lines(shift)), url(id))
    }

    fn plain(id: u32) -> LyricsHit {
        LyricsHit::new(Lyrics::Plain("words".into()), url(id))
    }

    fn saved(title: &str, duration: Option<f64>, id: u32, shift: f64) -> SavedChoice {
        SavedChoice {
            key: track(title, duration).cache_key(),
            duration,
            url: url(id),
            lines: lines(shift),
            offsets: BTreeMap::new(),
        }
    }

    fn urls(hits: &[LyricsHit]) -> Vec<String> {
        hits.iter().map(|h| h.url.clone()).collect()
    }

    fn found<'a>(book: &'a ChoiceBook, title: &str, duration: Option<f64>) -> Option<&'a str> {
        book.find(&track(title, duration))
            .map(|choice| choice.url.as_str())
    }

    #[test]
    fn find_matches_key_and_duration_within_two_seconds() {
        let book = ChoiceBook {
            songs: vec![saved("T", Some(200.0), 1, 0.0)],
        };
        let one = url(1);
        for near in [200.0, 202.0, 198.0] {
            assert_eq!(found(&book, "T", Some(near)), Some(one.as_str()), "{near}");
        }
        for far in [202.1, 197.9] {
            assert_eq!(found(&book, "T", Some(far)), None, "{far}");
        }
        // 歌名以正規化後的形式比對。
        assert_eq!(found(&book, "t (Live)", Some(200.0)), Some(one.as_str()));
        assert_eq!(found(&book, "U", Some(200.0)), None);
        // 寫起來剛好差 2 秒的小數（`256.1 - 254.1 > 2.0`）不受浮點誤差影響。
        let decimal = ChoiceBook {
            songs: vec![saved("T", Some(254.1), 1, 0.0)],
        };
        assert_eq!(found(&decimal, "T", Some(256.1)), Some(one.as_str()));
    }

    #[test]
    fn an_unknown_duration_only_matches_an_unknown_one() {
        let unknowns = [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ];
        let known = ChoiceBook {
            songs: vec![saved("T", Some(1.0), 1, 0.0)],
        };
        for unknown in unknowns {
            assert_eq!(found(&known, "T", unknown), None, "{unknown:?}");
        }
        let two = url(2);
        for recorded in unknowns {
            let book = ChoiceBook {
                songs: vec![saved("T", recorded, 2, 0.0)],
            };
            assert_eq!(found(&book, "T", Some(1.0)), None, "{recorded:?}");
            for unknown in unknowns {
                assert_eq!(
                    found(&book, "T", unknown),
                    Some(two.as_str()),
                    "{recorded:?} {unknown:?}"
                );
            }
        }
    }

    #[test]
    fn find_prefers_the_closest_duration() {
        let book = ChoiceBook {
            songs: vec![
                saved("T", Some(200.0), 1, 0.0),
                saved("T", Some(203.0), 2, 0.0),
            ],
        };
        assert_eq!(found(&book, "T", Some(201.2)), Some(url(1).as_str()));
        assert_eq!(found(&book, "T", Some(201.8)), Some(url(2).as_str()));
    }

    #[test]
    fn upsert_replaces_the_same_song_and_keeps_others() {
        let other_song = saved("U", Some(200.0), 2, 0.0);
        let other_length = saved("T", Some(260.0), 3, 0.0);
        let unknown_length = saved("T", None, 4, 0.0);
        let mut book = ChoiceBook::default();
        for choice in [
            saved("T", Some(200.0), 1, 0.0),
            other_song.clone(),
            other_length.clone(),
            unknown_length.clone(),
        ] {
            book.upsert(choice);
        }
        assert_eq!(book.songs.len(), 4);

        // 曲長差一秒仍是同一首：整筆換掉。
        let mut replacement = saved("T", Some(201.0), 5, 5.0);
        replacement.offsets.insert(url(5), 0.5);
        book.upsert(replacement.clone());
        assert_eq!(
            book.songs,
            [
                replacement.clone(),
                other_song.clone(),
                other_length.clone(),
                unknown_length,
            ]
        );

        let replaced_unknown = saved("T", None, 6, 0.0);
        book.upsert(replaced_unknown.clone());
        assert_eq!(
            book.songs,
            [replacement, other_song, other_length, replaced_unknown]
        );
    }

    /// 新曲長落在兩筆的容忍內：只換掉 `find` 會挑到的那筆，另一筆不能無聲消失。
    #[test]
    fn upsert_between_two_near_songs_replaces_only_the_closest() {
        let first = saved("T", Some(200.0), 1, 0.0);
        let mut book = ChoiceBook {
            songs: vec![first.clone(), saved("T", Some(203.0), 2, 0.0)],
        };
        let replacement = saved("T", Some(202.0), 3, 0.0);
        book.upsert(replacement.clone());
        assert_eq!(book.songs, [first, replacement]);
    }

    #[test]
    fn apply_choice_without_a_saved_choice_picks_the_first() {
        let versions = vec![hit(1, 0.0), hit(2, 5.0)];
        assert_eq!(apply_choice(versions.clone(), None), (versions, 0));
        assert_eq!(apply_choice(vec![plain(1)], None), (vec![plain(1)], 0));
        assert_eq!(apply_choice(Vec::new(), None), (Vec::new(), 0));
    }

    #[test]
    fn apply_choice_covers_url_same_timing_missing_and_plain() {
        let versions = || vec![hit(1, 0.0), hit(2, 5.0)];

        // 網址相同：選它，內容用新查到的；即使存下的內容和別的版本時間軸相同。
        let choice = saved("T", None, 2, 0.0);
        let (got, index) = apply_choice(versions(), Some(&choice));
        assert_eq!(urls(&got), [url(1), url(2)]);
        assert_eq!(index, 1);
        assert_eq!(got, versions());

        // 時間軸相同：存下的版本取代那一個。
        let choice = saved("T", None, 9, 5.3);
        let (got, index) = apply_choice(versions(), Some(&choice));
        assert_eq!(urls(&got), [url(1), url(9)]);
        assert_eq!(index, 1);
        assert_eq!(got, [hit(1, 0.0), hit(9, 5.3)]);

        // 找不到：附加在最後。
        let choice = saved("T", None, 9, 20.0);
        let (got, index) = apply_choice(versions(), Some(&choice));
        assert_eq!(urls(&got), [url(1), url(2), url(9)]);
        assert_eq!(index, 2);
        assert_eq!(got, [hit(1, 0.0), hit(2, 5.0), hit(9, 20.0)]);

        // 只查到純文字、或什麼都沒有：只剩存下的版本。
        for found in [vec![plain(1)], Vec::new()] {
            let (got, index) = apply_choice(found, Some(&choice));
            assert_eq!(urls(&got), [url(9)]);
            assert_eq!(index, 0);
            assert_eq!(got, [hit(9, 20.0)]);
        }
    }

    /// 同一筆紀錄這次只剩純文字：網址相同也不能拿它取代記住的時間軸版本。
    #[test]
    fn a_plain_record_at_the_saved_url_does_not_replace_the_saved_lines() {
        let choice = saved("T", None, 1, 0.0);
        assert_eq!(
            apply_choice(vec![plain(1)], Some(&choice)),
            (vec![hit(1, 0.0)], 0)
        );
    }

    #[test]
    fn shift_moves_lines_earlier_for_positive_offsets() {
        let line = |time, text: &str| LyricLine {
            time,
            text: text.into(),
        };
        let original = [line(0.0, "a"), line(10.0, "b")];
        assert_eq!(
            shift_lyrics(&original, 0.5),
            [line(-0.5, "a"), line(9.5, "b")]
        );
        assert_eq!(
            shift_lyrics(&original, -0.5),
            [line(0.5, "a"), line(10.5, "b")]
        );
    }

    #[test]
    fn offsets_are_clamped() {
        assert_eq!(OFFSET_LIMIT, 120.0);
        assert_eq!(clamp_offset(500.0), 120.0);
        assert_eq!(clamp_offset(-500.0), -120.0);
        assert_eq!(clamp_offset(f64::INFINITY), 120.0);
        assert_eq!(clamp_offset(f64::NEG_INFINITY), -120.0);
        assert_eq!(clamp_offset(f64::NAN), 0.0);
        for inside in [-120.0, -0.5, 0.0, 0.5, 120.0] {
            assert_eq!(clamp_offset(inside), inside);
        }
    }
}
