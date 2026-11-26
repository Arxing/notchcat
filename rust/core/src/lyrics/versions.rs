//! 同一首歌的多份時間軸歌詞，合併成幾個「時間版本」（Rust 版新增）。
//!
//! 歌庫裡一首歌常有很多筆紀錄，時間軸幾乎一樣的算同一個版本。比對時兩邊文字相同的行
//! （忽略繁簡、大小寫、全半形、空白與標點）按出現次序配對：配到的行數達較短那份的六成，
//! 且時間差的中位數在 1 秒以內，就是同一版本。

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use super::identity::fold;
use super::{LyricLine, Lyrics, LyricsHit};

/// 同版本配對行時間差中位數的上限（秒）。
const MAX_MEDIAN_GAP: f64 = 1.0;
/// 比較時間差時容許的浮點誤差（秒），遠小於 LRC 時間戳的最小單位（毫秒）。
const TIME_EPSILON: f64 = 1e-6;

/// 比對用的形式：（行的比對鍵，第幾次出現）對到該行時間。
type LineKeys = HashMap<(String, usize), f64>;

/// 比對鍵只留摺疊後的文字與數字；什麼都不剩的行（「♪」之類）不參與比對。
fn line_keys(lines: &[LyricLine]) -> LineKeys {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut keys = LineKeys::new();
    for line in lines {
        let text: String = fold(&line.text)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        if text.is_empty() {
            continue;
        }
        let occurrence = seen.entry(text.clone()).or_default();
        keys.insert((text, *occurrence), line.time);
        *occurrence += 1;
    }
    keys
}

fn keys_match(a: &LineKeys, b: &LineKeys) -> bool {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut gaps: Vec<f64> = shorter
        .iter()
        .filter_map(|(key, time)| longer.get(key).map(|other| (time - other).abs()))
        .collect();
    // 六成以整數比較（配到數 / 較短行數 >= 3 / 5），剛好六成不受浮點誤差影響。
    if gaps.is_empty() || gaps.len() * 5 < shorter.len() * 3 {
        return false;
    }
    gaps.sort_by(f64::total_cmp);
    let middle = gaps.len() / 2;
    let median = if gaps.len() % 2 == 1 {
        gaps[middle]
    } else {
        (gaps[middle - 1] + gaps[middle]) / 2.0
    };
    median <= MAX_MEDIAN_GAP + TIME_EPSILON
}

/// 兩份時間軸歌詞是否算同一個版本（規則見模組說明）。沒有可比的行時一律不算。
pub fn same_version(a: &[LyricLine], b: &[LyricLine]) -> bool {
    keys_match(&line_keys(a), &line_keys(b))
}

/// 把候選合併成版本，回傳各版本的代表，預設版本在第一個。
///
/// - 只看時間軸歌詞；同一網址只算一次。
/// - 依輸入順序分組：和各組第一個成員比，都不同就另開一組。
/// - 版本順序：含曲長吻合（非放寬）紀錄的在前，再依紀錄筆數由多到少，同序者最早輸入的在前。
/// - 代表：組內非放寬者優先，再取曲長差最小者（未知當 0），同序取最早輸入者。
pub fn merge_versions(hits: Vec<LyricsHit>) -> Vec<LyricsHit> {
    let mut urls = HashSet::new();
    // 每組：第一個成員的比對鍵，加上依輸入順序排列的全部成員。
    let mut groups: Vec<(LineKeys, Vec<LyricsHit>)> = Vec::new();
    for hit in hits {
        let Lyrics::Synced(lines) = &hit.lyrics else {
            continue;
        };
        if !urls.insert(hit.url.clone()) {
            continue;
        }
        let keys = line_keys(lines);
        match groups
            .iter_mut()
            .find(|(first, _)| keys_match(first, &keys))
        {
            Some((_, members)) => members.push(hit),
            None => groups.push((keys, vec![hit])),
        }
    }
    // 穩定排序：同序時維持建組順序，也就是最早輸入者在前。
    groups.sort_by_key(|(_, members)| {
        (
            members.iter().all(|hit| hit.loose_duration),
            Reverse(members.len()),
        )
    });
    groups
        .into_iter()
        .filter_map(|(_, members)| representative(members))
        .collect()
}

/// 組內曲長最接近的一筆（`min_by` 在同序時回傳第一個）。
fn representative(members: Vec<LyricsHit>) -> Option<LyricsHit> {
    members.into_iter().min_by(|a, b| {
        let gap = |hit: &LyricsHit| hit.duration_gap.unwrap_or(0.0);
        a.loose_duration
            .cmp(&b.loose_duration)
            .then_with(|| gap(a).total_cmp(&gap(b)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Lyrics;

    fn lines(shift: f64, texts: &[&str]) -> Vec<LyricLine> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| LyricLine {
                time: 10.0 * i as f64 + shift,
                text: (*t).into(),
            })
            .collect()
    }

    fn hit(id: u32, l: Vec<LyricLine>) -> LyricsHit {
        LyricsHit::new(Lyrics::Synced(l), format!("https://test.example/{id}"))
    }

    fn urls(hits: &[LyricsHit]) -> Vec<&str> {
        hits.iter().map(|h| h.url.as_str()).collect()
    }

    const A: [&str; 5] = [
        "泛黃的春聯",
        "還殘留在牆上",
        "依稀可見幾個字",
        "歲歲平安",
        "米缸",
    ];
    const A_SIMPLIFIED: [&str; 5] = [
        "泛黄的春联",
        "还残留在 墙上，",
        "依稀可见几个字",
        "岁岁平安",
        "米缸",
    ];

    #[test]
    fn small_timing_differences_are_one_version() {
        assert!(same_version(&lines(0.0, &A), &lines(1.0, &A)));
        assert!(!same_version(&lines(0.0, &A), &lines(1.1, &A)));
        assert!(!same_version(&lines(0.0, &A), &lines(5.0, &A)));
    }

    /// 時間戳是十進位小數，相減有浮點誤差（`4.4 - 3.4 > 1.0`）：寫起來剛好差 1 秒的仍算同一版本。
    #[test]
    fn exactly_one_second_apart_survives_floating_point_error() {
        assert!(same_version(&lines(3.4, &A[..1]), &lines(4.4, &A[..1])));
        assert!(!same_version(&lines(3.4, &A[..1]), &lines(4.41, &A[..1])));
    }

    #[test]
    fn script_spacing_punctuation_and_credit_lines_do_not_split_a_version() {
        let mut with_credits = lines(0.2, &A_SIMPLIFIED);
        with_credits.insert(
            0,
            LyricLine {
                time: 0.0,
                text: "作词 : 方文山".into(),
            },
        );
        assert!(same_version(&lines(0.0, &A), &with_credits));
    }

    #[test]
    fn too_few_matching_lines_are_a_different_version() {
        let other = lines(0.0, &["泛黃的春聯", "還殘留在牆上", "別的", "不同", "句子"]);
        assert!(!same_version(&lines(0.0, &A), &other)); // 2/5 < 60%
        assert!(!same_version(&[], &lines(0.0, &A)));
    }

    #[test]
    fn sixty_percent_of_the_shorter_one_is_enough() {
        let other = lines(
            0.0,
            &[
                "泛黃的春聯",
                "還殘留在牆上",
                "依稀可見幾個字",
                "不同",
                "句子",
                "多出來的",
                "一行",
            ],
        );
        assert!(same_version(&lines(0.0, &A), &other)); // 3/5，比較短的那份
        assert!(same_version(&other, &lines(0.0, &A)));
    }

    #[test]
    fn lines_without_letters_or_digits_do_not_count() {
        // 歌詞後面接十行只有符號的間奏。
        let padded = |texts: &[&str], symbol: &str| {
            let mut all = lines(0.0, texts);
            all.extend((0..10).map(|i| LyricLine {
                time: 100.0 + f64::from(i),
                text: symbol.into(),
            }));
            all
        };
        // 間奏行不能把兩份不同的歌詞湊成同一版本……
        let different = ["甲", "乙", "丙", "丁", "戊"];
        assert!(!same_version(
            &padded(&A, "♪ …… ♪"),
            &padded(&different, "♪ …… ♪")
        ));
        // ……符號寫法不同也不能壓低配對比例。
        assert!(same_version(&padded(&A, "♪ …… ♪"), &padded(&A, "～")));
    }

    #[test]
    fn the_median_time_difference_decides() {
        // 一行差很多不影響。
        let mut outlier = lines(0.0, &A);
        outlier[4].time += 60.0;
        assert!(same_version(&lines(0.0, &A), &outlier));
        // 偶數個配對取中間兩個的平均：(0.5 + 1.75) / 2 > 1。
        let mut split = lines(0.0, &A[..4]);
        split[1].time += 0.5;
        split[2].time += 1.75;
        split[3].time += 1.75;
        assert!(!same_version(&lines(0.0, &A[..4]), &split));
        // (0.25 + 1.5) / 2 <= 1。
        let mut near = lines(0.0, &A[..4]);
        near[1].time += 0.25;
        near[2].time += 1.5;
        near[3].time += 1.5;
        assert!(same_version(&lines(0.0, &A[..4]), &near));
    }

    #[test]
    fn repeated_lines_pair_in_order() {
        let chorus = ["啦", "啦", "啦", "啦"];
        assert!(same_version(&lines(0.0, &chorus), &lines(0.5, &chorus)));
    }

    #[test]
    fn the_largest_group_comes_first() {
        let got = merge_versions(vec![
            hit(1, lines(5.0, &A)),
            hit(2, lines(0.0, &A)),
            hit(3, lines(0.4, &A)),
        ]);
        assert_eq!(
            urls(&got),
            ["https://test.example/2", "https://test.example/1"]
        );
    }

    #[test]
    fn a_group_with_a_close_duration_outranks_a_bigger_loose_one() {
        let loose = |id, s| {
            let mut h = hit(id, lines(s, &A));
            h.loose_duration = true;
            h
        };
        let got = merge_versions(vec![loose(1, 0.0), loose(2, 0.1), hit(3, lines(5.0, &A))]);
        assert_eq!(urls(&got)[0], "https://test.example/3");
    }

    #[test]
    fn the_representative_has_the_closest_duration() {
        let gap = |id, g| {
            let mut h = hit(id, lines(0.0, &A));
            h.duration_gap = Some(g);
            h
        };
        assert_eq!(
            urls(&merge_versions(vec![gap(1, 3.0), gap(2, 0.5), gap(3, 0.5)])),
            ["https://test.example/2"]
        );
    }

    #[test]
    fn a_close_record_represents_its_group_before_a_loose_one_and_unknown_counts_as_zero() {
        let record = |id, loose, gap| {
            let mut h = hit(id, lines(0.0, &A));
            h.loose_duration = loose;
            h.duration_gap = gap;
            h
        };
        let got = merge_versions(vec![
            record(1, true, None),
            record(2, false, Some(4.0)),
            record(3, false, None),
        ]);
        assert_eq!(urls(&got), ["https://test.example/3"]);
    }

    #[test]
    fn the_same_url_is_counted_once_and_plain_lyrics_are_ignored() {
        let got = merge_versions(vec![
            hit(1, lines(0.0, &A)),
            hit(2, lines(5.0, &A)),
            hit(2, lines(5.0, &A)),
            LyricsHit::new(Lyrics::Plain("x".into()), "https://test.example/9"),
        ]);
        assert_eq!(
            urls(&got),
            ["https://test.example/1", "https://test.example/2"]
        );
        assert!(merge_versions(Vec::new()).is_empty());
    }
}
