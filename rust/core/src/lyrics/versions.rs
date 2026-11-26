//! 同一首歌的多份時間軸歌詞，合併成幾個「時間版本」（Rust 版新增）。
//!
//! 歌庫裡一首歌常有很多筆紀錄，時間軸幾乎一樣的算同一個版本。比對時兩邊文字相同的行
//! （忽略繁簡、大小寫、全半形、空白與標點）按出現次序配對：配到的行數達較短那份的六成，
//! 且時間差的中位數在 1 秒以內，就是同一版本。實際參與比對的行（摺疊後還有文字或數字）下稱「可比行」。
//!
//! 太短的紀錄（截斷版、只有製作名單）容易冒充版本，或把兩個版本黏在一起，所以合併時（[`merge_versions`]）：
//!
//! 1. 只要有任一筆的可比行達 5 行，不足 5 行的紀錄整筆忽略。
//! 2. 依可比行數由多到少分組（同數維持輸入順序），讓較長的紀錄當每組的基準。
//! 3. 代表只從可比行數達組內中位數八成的成員裡挑，依序比：
//!    曲長吻合（非放寬）者；曲長差在這些成員最小已知差距 1 秒內者，其次其他已知者，最後未知者；
//!    和組內其他成員一致度高者；可比行多者；先輸入者。
//!    一致度：對每個其他成員算「時間差 1 秒內的配對行數 ÷ 兩者較多的可比行數」，再取平均。
//! 4. 版本順序：含曲長吻合紀錄的組在前，再依紀錄筆數由多到少，同序者含最早輸入紀錄的在前。
//!
//! 全部紀錄都不足 5 行時沒有更好的依據：依輸入順序分組，代表取非放寬、曲長差最小（未知當 0）、
//! 先輸入者；版本順序同第 4 點。

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use super::identity::fold;
use super::{LyricLine, Lyrics, LyricsHit};

/// 同版本配對行時間差中位數的上限（秒）。
const MAX_MEDIAN_GAP: f64 = 1.0;
/// 比較時間差時容許的浮點誤差（秒），遠小於 LRC 時間戳的最小單位（毫秒）。
const TIME_EPSILON: f64 = 1e-6;
/// 可比行不足這個數的紀錄，在有其他夠長的紀錄時忽略。
const MIN_KEYED_LINES: usize = 5;
/// 挑代表時，曲長差離最小值這麼多秒以內算一樣近。
const LENGTH_GAP_WINDOW: f64 = 1.0;
/// 算一致度時，一對行的時間差在這個秒數以內才算一致。
const MAX_AGREEING_GAP: f64 = 1.0;

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

/// 兩份裡鍵相同的各對行，時間差（秒）。
fn paired_gaps<'a>(a: &'a LineKeys, b: &'a LineKeys) -> impl Iterator<Item = f64> + 'a {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    shorter
        .iter()
        .filter_map(|(key, time)| longer.get(key).map(|other| (time - other).abs()))
}

fn keys_match(a: &LineKeys, b: &LineKeys) -> bool {
    let mut gaps: Vec<f64> = paired_gaps(a, b).collect();
    // 六成以整數比較（配到數 / 較短行數 >= 3 / 5），剛好六成不受浮點誤差影響。
    if gaps.is_empty() || gaps.len() * 5 < a.len().min(b.len()) * 3 {
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

/// 一筆候選：輸入順位、比對鍵（每筆只摺疊一次）與命中本身。
struct Record {
    order: usize,
    keys: LineKeys,
    hit: LyricsHit,
}

impl Record {
    fn is_long(&self) -> bool {
        self.keys.len() >= MIN_KEYED_LINES
    }
}

/// 把候選合併成版本，回傳各版本的代表，預設版本在第一個（規則見模組說明）。
///
/// 只看時間軸歌詞；同一網址只算一次。
pub fn merge_versions(hits: Vec<LyricsHit>) -> Vec<LyricsHit> {
    let mut urls = HashSet::new();
    let mut records: Vec<Record> = Vec::new();
    for hit in hits {
        let Lyrics::Synced(lines) = &hit.lyrics else {
            continue;
        };
        if !urls.insert(hit.url.clone()) {
            continue;
        }
        let keys = line_keys(lines);
        records.push(Record {
            order: records.len(),
            keys,
            hit,
        });
    }
    let any_long = records.iter().any(Record::is_long);
    if any_long {
        records.retain(Record::is_long);
        // 穩定排序：可比行數相同的維持輸入順序。
        records.sort_by_key(|record| Reverse(record.keys.len()));
    }
    let mut groups = group(records);
    groups.sort_by_key(|members| {
        (
            members.iter().all(|record| record.hit.loose_duration),
            Reverse(members.len()),
            members.iter().map(|record| record.order).min(),
        )
    });
    let pick = if any_long {
        representative
    } else {
        short_representative
    };
    groups.into_iter().filter_map(pick).collect()
}

/// 依給定順序分組：和各組第一個成員比，都不同就另開一組。每組至少一個成員。
fn group(records: Vec<Record>) -> Vec<Vec<Record>> {
    let mut groups: Vec<Vec<Record>> = Vec::new();
    for record in records {
        match groups
            .iter_mut()
            .find(|members| keys_match(&members[0].keys, &record.keys))
        {
            Some(members) => members.push(record),
            None => groups.push(vec![record]),
        }
    }
    groups
}

/// 組內最能代表這個版本的一筆（模組說明第 3 點）。
fn representative(mut members: Vec<Record>) -> Option<LyricsHit> {
    let mut counts: Vec<usize> = members.iter().map(|record| record.keys.len()).collect();
    counts.sort_unstable();
    // 中位數的兩倍（偶數個成員是中間兩個的和），讓八成的門檻用整數比較。每組至少一個成員。
    let twice_median = counts[(counts.len() - 1) / 2] + counts[counts.len() / 2];
    // 行數 >= 中位數 × 4/5，兩邊同乘 5：行數 × 5 >= 中位數兩倍 × 2。
    let typical: Vec<usize> = (0..members.len())
        .filter(|&i| members[i].keys.len() * 5 >= twice_median * 2)
        .collect();
    let smallest_gap = typical
        .iter()
        .filter_map(|&i| members[i].hit.duration_gap)
        .min_by(f64::total_cmp);
    let best = typical.into_iter().min_by_key(|&i| {
        let record = &members[i];
        // 0：和最小差距一樣近；1：其他已知差距；2：未知。
        let length_rank: u8 = match (record.hit.duration_gap, smallest_gap) {
            (Some(gap), Some(smallest)) if gap <= smallest + LENGTH_GAP_WINDOW + TIME_EPSILON => 0,
            (Some(_), _) => 1,
            (None, _) => 2,
        };
        (
            record.hit.loose_duration,
            length_rank,
            Reverse(agreement(&members, i)),
            Reverse(record.keys.len()),
            record.order,
        )
    })?;
    Some(members.swap_remove(best).hit)
}

/// 第 `index` 個成員和組內其他成員的平均一致度，量化成整數（十億分之一）以便精確比較同分；
/// 沒有其他成員時為 0。
fn agreement(members: &[Record], index: usize) -> u64 {
    let own = &members[index].keys;
    let total: f64 = members
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != index)
        .map(|(_, other)| {
            let agreeing = paired_gaps(own, &other.keys)
                .filter(|gap| *gap <= MAX_AGREEING_GAP + TIME_EPSILON)
                .count();
            // 只在夠長的紀錄之間呼叫，分母不會是 0。
            agreeing as f64 / own.len().max(other.keys.len()) as f64
        })
        .sum();
    let others = members.len().saturating_sub(1).max(1);
    (total / others as f64 * 1e9).round() as u64
}

/// 全部紀錄都太短時的代表：非放寬者優先，再取曲長差最小者（未知當 0），同序取最早輸入者
/// （這時成員依輸入順序排列，`min_by` 在同序時回傳第一個）。
fn short_representative(members: Vec<Record>) -> Option<LyricsHit> {
    members.into_iter().map(|record| record.hit).min_by(|a, b| {
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
        // 重複行要逐次計數才湊得到六成：按次序配到 3/5，只用文字當鍵只剩 1/3。
        assert!(same_version(
            &lines(0.0, &["啦", "啦", "啦", "甲", "乙"]),
            &lines(0.0, &["啦", "啦", "啦", "丙", "丁"])
        ));
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

    /// `count` 行各不相同的歌詞，第 `i` 行在 `start + 10 * i` 秒。
    fn numbered(start: f64, count: u32) -> Vec<LyricLine> {
        (0..count)
            .map(|i| LyricLine {
                time: start + 10.0 * f64::from(i),
                text: format!("第{i}句"),
            })
            .collect()
    }

    /// `count` 行製作名單，從 0 秒起每秒一行。
    fn credits(count: u32) -> Vec<LyricLine> {
        (0..count)
            .map(|i| LyricLine {
                time: f64::from(i),
                text: format!("製作{i}：某人"),
            })
            .collect()
    }

    fn with_gap(id: u32, l: Vec<LyricLine>, gap: f64) -> LyricsHit {
        let mut h = hit(id, l);
        h.duration_gap = Some(gap);
        h
    }

    // ---- 太短的紀錄

    #[test]
    fn a_truncated_record_does_not_replace_the_full_one() {
        // 完整的五行（曲長差 1 秒）和它的前三行（曲長剛好）。
        let full = || with_gap(1, lines(0.0, &A), 1.0);
        let truncated = || with_gap(2, lines(0.0, &A[..3]), 0.0);
        for input in [vec![full(), truncated()], vec![truncated(), full()]] {
            assert_eq!(urls(&merge_versions(input)), ["https://test.example/1"]);
        }
    }

    #[test]
    fn a_credits_only_record_does_not_merge_two_versions() {
        let shared =
            [(0.0, "作词 : 方文山"), (1.0, "作曲 : 周杰伦")].map(|(time, text)| LyricLine {
                time,
                text: text.into(),
            });
        let full = |shift| [shared.to_vec(), lines(shift, &A)].concat();
        let got = merge_versions(vec![
            hit(1, shared.to_vec()),
            hit(2, full(20.0)),
            hit(3, full(25.0)),
        ]);
        assert_eq!(
            urls(&got),
            ["https://test.example/2", "https://test.example/3"]
        );
    }

    #[test]
    fn four_keyed_lines_are_too_few_next_to_a_record_with_five() {
        // 沒有文字的行不算：四行歌詞加十行間奏符號仍只有四行可比。
        let mut padded = numbered(40.0, 4);
        padded.extend((0..10).map(|i| LyricLine {
            time: 100.0 + f64::from(i),
            text: "♪".into(),
        }));
        let got = merge_versions(vec![hit(1, padded), hit(2, numbered(0.0, 5))]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
    }

    #[test]
    fn short_records_alone_behave_as_before() {
        // 全部都太短：一筆都不丟，依輸入順序分組，代表照舊（未知曲長差當 0）。
        let mut unknown = hit(1, numbered(0.0, 4));
        unknown.duration_gap = None;
        let got = merge_versions(vec![
            unknown,
            with_gap(2, numbered(0.0, 4), 4.0),
            hit(3, numbered(30.0, 2)),
        ]);
        assert_eq!(
            urls(&got),
            ["https://test.example/1", "https://test.example/3"]
        );
        // 分組仍以先輸入者為基準：0.9 秒的那筆在最前面時三筆是同一組。
        let got = merge_versions(vec![
            hit(1, numbered(0.9, 3)),
            hit(2, numbered(0.0, 4)),
            hit(3, numbered(1.8, 4)),
        ]);
        assert_eq!(got.len(), 1);
    }

    // ---- 分組與排序

    #[test]
    fn the_longest_record_anchors_its_group() {
        // 以 0.9 秒的短紀錄為基準會把差 1.8 秒的兩份湊在一起；以長紀錄為基準不會。
        let got = merge_versions(vec![
            hit(1, numbered(0.9, 5)),
            hit(2, numbered(0.0, 10)),
            hit(3, numbered(1.8, 10)),
        ]);
        assert_eq!(
            urls(&got),
            ["https://test.example/2", "https://test.example/3"]
        );
    }

    #[test]
    fn equal_groups_keep_the_original_input_order() {
        let got = merge_versions(vec![hit(1, numbered(0.0, 5)), hit(2, numbered(30.0, 10))]);
        assert_eq!(
            urls(&got),
            ["https://test.example/1", "https://test.example/2"]
        );
    }

    // ---- 代表

    #[test]
    fn the_representative_is_among_the_closest_in_length() {
        let pick = |gaps: [f64; 3]| {
            let records = (1..)
                .zip(gaps)
                .map(|(id, gap)| with_gap(id, lines(0.0, &A), gap))
                .collect();
            merge_versions(records)[0].url.clone()
        };
        // 離最小差距超過 1 秒的不選。
        assert_eq!(pick([3.0, 1.6, 0.5]), "https://test.example/3");
        // 剛好 1 秒內的一樣近，同序取先輸入的。
        assert_eq!(pick([3.0, 1.5, 0.5]), "https://test.example/2");
        // 浮點誤差不影響（1.3 - 0.3 > 1.0 不成立與否都算 1 秒內）。
        assert_eq!(pick([4.4, 4.4, 3.4]), "https://test.example/1");
    }

    #[test]
    fn a_close_record_represents_its_group_before_a_loose_one_and_unknown_ranks_last() {
        let record = |id, loose, gap| {
            let mut h = hit(id, lines(0.0, &A));
            h.loose_duration = loose;
            h.duration_gap = gap;
            h
        };
        let got = merge_versions(vec![
            record(1, true, Some(0.0)),
            record(2, false, None),
            record(3, false, Some(4.0)),
        ]);
        assert_eq!(urls(&got), ["https://test.example/3"]);
        // 只有放寬與未知時，吻合的（未知）仍在放寬之前。
        let got = merge_versions(vec![record(1, true, Some(8.0)), record(2, false, None)]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
    }

    #[test]
    fn an_unknown_length_does_not_beat_a_known_close_one() {
        let got = merge_versions(vec![
            hit(1, lines(0.0, &A)),
            with_gap(2, lines(0.0, &A), 4.0),
        ]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
    }

    #[test]
    fn a_slightly_closer_but_atypical_record_loses_to_the_agreeing_majority() {
        // 第一行時間不對（0.2 秒），其他行一致；曲長比多數近 0.3 秒。
        let mut odd = numbered(30.0, 10);
        odd[0].time = 0.2;
        let got = merge_versions(vec![
            with_gap(1, odd, 0.2),
            with_gap(2, numbered(30.0, 10), 0.5),
            with_gap(3, numbered(30.0, 10), 0.5),
            with_gap(4, numbered(30.0, 10), 0.5),
        ]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
    }

    #[test]
    fn lines_exactly_one_second_apart_agree() {
        // 4.4 - 3.4 在浮點數裡略大於 1；仍算一致，所以同序取先輸入的。
        let mut late = numbered(3.4, 10);
        late[0].time = 4.4;
        let got = merge_versions(vec![
            hit(1, late),
            hit(2, numbered(3.4, 10)),
            hit(3, numbered(3.4, 10)),
        ]);
        assert_eq!(urls(&got), ["https://test.example/1"]);
    }

    #[test]
    fn a_credit_heavy_record_does_not_exclude_the_typical_shorter_ones() {
        let heavy = [credits(10), numbered(30.0, 10)].concat();
        let got = merge_versions(vec![
            with_gap(1, heavy, 0.0),
            with_gap(2, numbered(30.0, 10), 0.5),
            with_gap(3, numbered(30.0, 10), 0.5),
        ]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
    }

    #[test]
    fn a_record_under_eighty_percent_of_the_median_cannot_represent_the_group() {
        let pick = |short: u32| {
            let got = merge_versions(vec![
                with_gap(1, numbered(30.0, 10), 3.0),
                with_gap(2, numbered(30.0, 10), 3.0),
                with_gap(3, numbered(30.0, short), 0.0),
            ]);
            urls(&got).join(" ")
        };
        assert_eq!(pick(7), "https://test.example/1");
        // 剛好八成可以。
        assert_eq!(pick(8), "https://test.example/3");

        // 偶數個成員：中位數是中間兩個的平均（8.5），門檻 6.8。
        let got = merge_versions(vec![
            with_gap(1, numbered(30.0, 10), 4.0),
            with_gap(2, numbered(30.0, 10), 4.0),
            with_gap(3, numbered(30.0, 6), 0.0),
            with_gap(4, numbered(30.0, 7), 1.5),
        ]);
        assert_eq!(urls(&got), ["https://test.example/4"]);
    }

    #[test]
    fn equal_agreement_prefers_the_record_with_more_lines() {
        let got = merge_versions(vec![hit(1, numbered(30.0, 9)), hit(2, numbered(30.0, 10))]);
        assert_eq!(urls(&got), ["https://test.example/2"]);
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
