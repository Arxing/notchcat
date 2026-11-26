//! 歌詞模型與寬鬆的 LRC 解析（移植自 Swift `Lyrics.swift`）。
//!
//! 時間一律是秒（`f64`）。

pub mod identity;
pub mod lrclib;
pub mod netease;
pub mod normalizer;
pub mod provider;
pub mod repository;
pub mod traditional;
pub mod versions;

pub use identity::TrackIdentity;
pub use normalizer::identity_from;

/// 一行帶時間的歌詞。
#[derive(Clone, PartialEq, Debug)]
pub struct LyricLine {
    pub time: f64,
    pub text: String,
}

/// 歌詞：帶時間軸，或純文字（只當保底，不拿來即時顯示）。
#[derive(Clone, PartialEq, Debug)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
}

/// 一次命中：歌詞，加上它的來源網址（讓使用者能開啟來源頁面）。
///
/// `url` 一律是 `https://` 開頭：提供者只會組出自己服務的 https 位址。
/// 它與 `Lyrics` 並排而不是放進 `Lyrics`，所以歌詞的比對與呈現邏輯完全不受影響。
#[derive(Clone, PartialEq, Debug)]
pub struct LyricsHit {
    pub lyrics: Lyrics,
    pub url: String,
    /// 曲長只在放寬的容忍內（見 [`identity::DurationFit::Loose`]）。
    pub loose_duration: bool,
    /// 紀錄曲長和查詢曲長差幾秒；任一邊未知時為 `None`。
    pub duration_gap: Option<f64>,
}

impl LyricsHit {
    pub fn new(lyrics: Lyrics, url: impl Into<String>) -> Self {
        let url = url.into();
        debug_assert!(url.starts_with("https://"), "非 https 網址：{url}");
        Self {
            lyrics,
            url,
            loose_duration: false,
            duration_gap: None,
        }
    }

    /// 依紀錄曲長和查詢的吻合程度標記；差超過放寬的容忍時為 `None`。
    pub(crate) fn matched(
        mut self,
        track: &TrackIdentity,
        record_duration: Option<f64>,
    ) -> Option<Self> {
        let fit = track.duration_fit(record_duration)?;
        self.loose_duration = fit == identity::DurationFit::Loose;
        self.duration_gap = track.duration_gap(record_duration);
        Some(self)
    }

    /// 挑選用的優先序，越大越好：時間軸勝過純文字；同類之中曲長吻合的勝過放寬的。
    fn rank(&self) -> u8 {
        u8::from(self.lyrics.is_synced()) * 2 + u8::from(!self.loose_duration)
    }

    /// 只在 `candidate` 的優先序更高時換掉 `best`；同序保留先找到的。
    pub(crate) fn keep_better(best: &mut Option<Self>, candidate: Self) {
        if best.as_ref().is_none_or(|b| b.rank() < candidate.rank()) {
            *best = Some(candidate);
        }
    }
}

impl Lyrics {
    /// 最後一個時間行可以比曲長晚多少秒。
    pub const DURATION_TOLERANCE: f64 = 5.0;

    pub fn is_synced(&self) -> bool {
        matches!(self, Lyrics::Synced(_))
    }

    /// 這份歌詞是否可信到能逐行顯示。寧可不顯示，也不顯示錯的一行。
    ///
    /// 純文字永遠通過（它從不用於即時顯示）。帶時間軸的歌詞至少要有兩行
    /// （只有一行通常是「純音樂」之類的佔位），且在曲長已知（大於 0）時，
    /// 最後一行不得晚於「曲長 + `DURATION_TOLERANCE`」（更晚通常是不同版本）。
    /// 不套用更嚴的規則：長前奏／尾奏很常見，會誤殺正確的歌詞。
    pub fn is_plausible(&self, duration: Option<f64>) -> bool {
        let Lyrics::Synced(lines) = self else {
            return true;
        };
        if lines.len() < 2 {
            return false;
        }
        if let (Some(duration), Some(last)) = (duration, lines.last()) {
            if duration > 0.0 {
                return last.time <= duration + Self::DURATION_TOLERANCE;
            }
        }
        true
    }

    /// 此刻應顯示的行索引；純文字或尚未到第一行時為 `None`。
    /// 多行共用同一時間戳時，以其中最後一行為準。NaN 位置為 `None`。
    pub fn line_index(&self, position: f64) -> Option<usize> {
        let Lyrics::Synced(lines) = self else {
            return None;
        };
        let first = lines.first()?;
        // 與 Swift 的 `position >= first.time` 守門同義：寫成否定形式，讓 `position` 或第一行時間為 NaN
        // （比較皆為 false）時一律回 `None`；通過之後 `first.time <= position` 成立，下面的結果 >= 1。
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(position >= first.time) {
            return None;
        }
        // 上界：第一個嚴格晚於 `position` 的行；它前一行就是目前行。
        // 時間以 `<= position` 為前綴述詞。
        Some(lines.partition_point(|l| l.time <= position) - 1)
    }
}

/// 同步歌詞全部行文字位元組數的上限（Swift 版沒有）。
///
/// `[t][t]…[t]` 多標籤共用一行文字時，Swift 的 `String` 是寫時複製所以幾乎免費，
/// 但這裡 `LyricLine.text` 是擁有所有權的 `String`，每個標籤都要複製一份整行文字，
/// 惡意輸入（數萬標籤 × 數 MB 文字）會要數十 GB。超過預算視為沒有歌詞。
/// 提供者回應本體上限 4 MiB，合法歌詞離這個預算差好幾個數量級。
const MAX_SYNCED_TEXT_BYTES: usize = 32 * 1024 * 1024;

/// 解析任何提供者給的 LRC 文字。
///
/// 社群歌詞庫品質參差，所以刻意寬鬆：排序所見、丟掉空行，而且沒有真的帶時間戳
/// 就不稱為同步。以位元組手動掃描而非正規表示式，讓惡意輸入（超大數字、百萬字元
/// 單行、成千上萬個標籤）只花線性時間且不會 panic。
///
/// 輸入大小的上限由呼叫端負責：本函式依賴提供者的 4 MiB 回應本體上限（`provider::MAX_BODY_BYTES`），
/// 自己不限制輸入長度，只以 [`MAX_SYNCED_TEXT_BYTES`] 限制輸出的文字總量。
///
/// 空輸入、以及唯一的時間戳都落在沒有歌詞文字的行上時回傳 `None`。完全沒有合法
/// 時間標籤的文字（包含 `[ar:…]` 這類只有中繼資料的標籤）回傳 `Lyrics::Plain`。
pub fn parse_lrc(raw: &str) -> Option<Lyrics> {
    let without_bom = raw.trim_start_matches('\u{FEFF}');
    let trimmed = without_bom.trim_matches(char::is_whitespace);
    if trimmed.is_empty() {
        return None;
    }

    let mut lines: Vec<LyricLine> = Vec::new();
    let mut budget = MAX_SYNCED_TEXT_BYTES;
    let mut saw_time_tag = false;
    for raw_line in trimmed.split(is_newline) {
        let mut rest = raw_line.trim_start_matches(char::is_whitespace);
        let mut times: Vec<f64> = Vec::new();
        while let Some((time, remainder)) = leading_time_tag(rest, b'[', b']') {
            times.push(time);
            rest = remainder.trim_start_matches(char::is_whitespace);
        }
        if times.is_empty() {
            continue;
        }
        saw_time_tag = true;

        let text = strip_word_tags(rest);
        if text.is_empty() {
            continue;
        }
        let needed = text.len().saturating_mul(times.len());
        if needed > budget {
            return None;
        }
        budget -= needed;
        let (last, others) = times.split_last().expect("times is non-empty");
        for &time in others {
            lines.push(LyricLine {
                time,
                text: text.clone(),
            });
        }
        lines.push(LyricLine { time: *last, text });
    }

    if !saw_time_tag {
        return Some(Lyrics::Plain(trimmed.to_string()));
    }
    if lines.is_empty() {
        return None;
    }

    // 以（時間，原始順序）排序，同時間戳保留輸入順序（`sort_by` 本身就是穩定排序）。
    lines.sort_by(|a, b| a.time.total_cmp(&b.time));
    Some(Lyrics::Synced(lines))
}

/// 與 Swift `Character.isNewline` 一致：\n \r \u{0B} \u{0C} \u{85} \u{2028} \u{2029}
/// （"\r\n" 在 Swift 是單一字元；這裡切成兩個分隔符，中間多出的空行不含時間標籤，會被略過）。
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// 與 Swift `.trimmingCharacters(in: .whitespaces)` 一致（Zs 加 tab）。呼叫端的文字已切過
/// 換行字元，所以 `char::is_whitespace` 在此與它等價。
fn trim_horizontal(text: &str) -> &str {
    text.trim_matches(char::is_whitespace)
}

/// 移除增強型 LRC 的字詞時間（`<mm:ss.xx>`），再收合它們留下的空白並修剪。
/// 不含合法時間的角括號（`<3`、`<b>`）留在文字中。沒有任何字詞標籤的文字只修剪。
fn strip_word_tags(text: &str) -> String {
    if !text.contains('<') {
        return trim_horizontal(text).to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    let mut removed_tag = false;
    while let Some(open) = rest.find('<') {
        result.push_str(&rest[..open]);
        let candidate = &rest[open..];
        if let Some((_, remainder)) = leading_time_tag(candidate, b'<', b'>') {
            removed_tag = true;
            rest = remainder;
        } else {
            result.push('<');
            rest = &candidate[1..];
        }
    }
    result.push_str(rest);

    if !removed_tag {
        return trim_horizontal(&result).to_string();
    }
    let mut collapsed = String::with_capacity(result.len());
    let mut previous_was_space = false;
    for c in result.chars() {
        let is_space = c == ' ' || c == '\t';
        if is_space && previous_was_space {
            continue;
        }
        collapsed.push(c);
        previous_was_space = is_space;
    }
    trim_horizontal(&collapsed).to_string()
}

/// 在 `text` 開頭讀一個 `[mm:ss]`、`[mm:ss.x…]` 或 `[mm:ss:x…]` 標籤（或同形的 `<…>`）。
/// 分鐘 1–3 位、秒 1–2 位、小數 1–3 位；其他（正負號、更長的數字、中繼資料鍵）都不是時間標籤。
fn leading_time_tag(text: &str, open: u8, close: u8) -> Option<(f64, &str)> {
    // 標籤全是 ASCII，所以以位元組索引切片不會切到字元中間。
    let bytes = text.as_bytes();
    let mut index = 0;

    fn consume(bytes: &[u8], index: &mut usize, byte: u8) -> bool {
        if bytes.get(*index) == Some(&byte) {
            *index += 1;
            true
        } else {
            false
        }
    }
    fn digits(bytes: &[u8], index: &mut usize, max_count: u32) -> Option<(u32, u32)> {
        let mut value = 0u32;
        let mut count = 0u32;
        while count < max_count {
            match bytes.get(*index) {
                Some(b @ b'0'..=b'9') => {
                    value = value * 10 + u32::from(b - b'0');
                    count += 1;
                    *index += 1;
                }
                _ => break,
            }
        }
        (count > 0).then_some((value, count))
    }

    if !consume(bytes, &mut index, open) {
        return None;
    }
    let (minutes, _) = digits(bytes, &mut index, 3)?;
    if !consume(bytes, &mut index, b':') {
        return None;
    }
    let (seconds, _) = digits(bytes, &mut index, 2)?;

    let mut fraction = 0.0;
    if consume(bytes, &mut index, b'.') || consume(bytes, &mut index, b':') {
        let (value, count) = digits(bytes, &mut index, 3)?;
        fraction = f64::from(value) / 10f64.powi(count as i32);
    }
    if !consume(bytes, &mut index, close) {
        return None;
    }

    let time = f64::from(minutes) * 60.0 + f64::from(seconds) + fraction;
    Some((time, &text[index..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    fn line(time: f64, text: &str) -> LyricLine {
        LyricLine {
            time,
            text: text.to_string(),
        }
    }

    fn synced(raw: &str) -> Vec<LyricLine> {
        match parse_lrc(raw) {
            Some(Lyrics::Synced(lines)) => lines,
            other => {
                let head: String = raw.chars().take(60).collect();
                panic!("expected synced lyrics for {head:?}, got {other:?}")
            }
        }
    }

    #[test]
    fn lyrics_hit_pairs_lyrics_with_its_https_url() {
        let hit = LyricsHit::new(Lyrics::Plain("words".into()), "https://x.example/1");
        assert_eq!(hit.lyrics, Lyrics::Plain("words".into()));
        assert_eq!(hit.url, "https://x.example/1");
    }

    #[test]
    fn matched_records_how_far_off_the_duration_is() {
        let track = TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: None,
            duration: Some(200.0),
        };
        let matched = |record| {
            LyricsHit::new(Lyrics::Plain("words".into()), "https://x.example/1")
                .matched(&track, record)
                .map(|h| (h.loose_duration, h.duration_gap))
        };
        assert_eq!(matched(Some(197.0)), Some((false, Some(3.0))));
        assert_eq!(matched(Some(208.0)), Some((true, Some(8.0))));
        assert_eq!(matched(None), Some((false, None)));
        assert_eq!(matched(Some(211.0)), None);
    }

    fn texts(lines: &[LyricLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    fn times(lines: &[LyricLine]) -> Vec<f64> {
        lines.iter().map(|l| l.time).collect()
    }

    fn assert_close(actual: f64, expected: f64, accuracy: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= accuracy,
            "{what}: {actual} != {expected}"
        );
    }

    // MARK: - Parsing

    #[test]
    fn parses_timestamped_lines() {
        let lines = synced("[00:12.34]first\n[01:05.00]second");
        assert_eq!(lines.len(), 2);
        assert_close(lines[0].time, 12.34, 0.001, "t0");
        assert_eq!(lines[0].text, "first");
        assert_close(lines[1].time, 65.0, 0.001, "t1");
        assert_eq!(lines[1].text, "second");
    }

    #[test]
    fn unsorted_timestamps_are_sorted() {
        assert_eq!(texts(&synced("[01:00.00]b\n[00:10.00]a")), ["a", "b"]);
    }

    #[test]
    fn text_without_any_timestamp_is_plain() {
        assert_eq!(
            parse_lrc("  just words\nmore words \n"),
            Some(Lyrics::Plain("just words\nmore words".into()))
        );
    }

    #[test]
    fn metadata_only_tags_do_not_count_as_synced() {
        let raw = "[ar:Artist]\n[ti:Title]\n[offset:500]\nwords";
        assert_eq!(parse_lrc(raw), Some(Lyrics::Plain(raw.into())));
    }

    #[test]
    fn metadata_lines_alongside_timestamps_are_ignored() {
        let lines = synced("[ar:Artist]\n[ti:Title]\n[length:03:20]\n[00:01.00]hello");
        assert_eq!(lines, vec![line(1.0, "hello")]);
    }

    #[test]
    fn empty_input_yields_nil() {
        assert_eq!(parse_lrc(""), None);
        assert_eq!(parse_lrc("   \n  "), None);
    }

    #[test]
    fn blank_lyric_lines_are_dropped() {
        assert_eq!(texts(&synced("[00:01.00]\n[00:02.00]real")), ["real"]);
    }

    #[test]
    fn repeated_timestamps_on_one_line() {
        let lines = synced("[00:10.00][00:20.00]chorus");
        assert_eq!(times(&lines), [10.0, 20.0]);
        assert_eq!(texts(&lines), ["chorus", "chorus"]);
    }

    #[test]
    fn duplicate_timestamps_keep_both_lines_in_input_order() {
        let lines = synced("[00:10.00]one\n[00:05.00]early\n[00:10.00]two\n[00:10.00]three");
        assert_eq!(texts(&lines), ["early", "one", "two", "three"]);
    }

    #[test]
    fn time_tag_formats() {
        let lines = synced(
            "[00:01.50]centis-one-digit\n\
             [00:02.25]centis\n\
             [00:03.125]millis\n\
             [00:04]bare\n\
             [00:05:50]colon-fraction\n\
             [1:06.5]short-minutes",
        );
        assert_eq!(
            texts(&lines),
            [
                "centis-one-digit",
                "centis",
                "millis",
                "bare",
                "colon-fraction",
                "short-minutes",
            ]
        );
        let expected = [1.5, 2.25, 3.125, 4.0, 5.5, 66.5];
        assert_eq!(lines.len(), expected.len());
        for (l, t) in lines.iter().zip(expected) {
            assert_close(l.time, t, 0.0001, &l.text);
        }
    }

    #[test]
    fn whitespace_around_tags_and_text_is_trimmed() {
        let lines = synced("  [00:01.00]   spaced out   \r\n[00:02.00]\ttabbed\t");
        assert_eq!(texts(&lines), ["spaced out", "tabbed"]);
    }

    #[test]
    fn crlf_and_cr_line_endings() {
        assert_eq!(texts(&synced("[00:01.00]a\r\n[00:02.00]b\r\n")), ["a", "b"]);
        assert_eq!(texts(&synced("[00:01.00]a\r[00:02.00]b")), ["a", "b"]);
    }

    #[test]
    fn non_ascii_text_is_preserved_intact() {
        let lines = synced("[00:01.00]你好，世界 🎵\n[00:02.00]héllo");
        assert_eq!(texts(&lines), ["你好，世界 🎵", "héllo"]);
    }

    #[test]
    fn leading_byte_order_mark_is_ignored() {
        assert_eq!(texts(&synced("\u{FEFF}[00:01.00]hi")), ["hi"]);
    }

    #[test]
    fn time_tag_inside_text_is_not_a_timestamp() {
        let lines = synced("[00:01.00]see [00:30.00] later");
        assert_eq!(lines, vec![line(1.0, "see [00:30.00] later")]);
    }

    #[test]
    fn enhanced_word_tags_are_stripped() {
        assert_eq!(
            synced("[00:01.00]<00:01.00>Hello <00:01.50>world"),
            vec![line(1.0, "Hello world")]
        );
        assert_eq!(
            texts(&synced("[00:01.00]<00:01>a <0:02:50> b  <00:03.125>c")),
            ["a b c"]
        );
    }

    #[test]
    fn word_tags_around_cjk_text() {
        assert_eq!(
            texts(&synced(
                "[00:05.00]<00:05.00>你好<00:05.40>，<00:05.80> 世界 <00:06.20>🎵"
            )),
            ["你好， 世界 🎵"]
        );
    }

    #[test]
    fn non_time_angle_brackets_stay_in_text() {
        assert_eq!(texts(&synced("[00:01.00]I <3 you")), ["I <3 you"]);
        assert_eq!(
            texts(&synced("[00:01.00]<b>bold</b> <00:02.00>text")),
            ["<b>bold</b> text"]
        );
        assert_eq!(
            texts(&synced("[00:01.00]<00:01.00 unclosed <-00:02.00>x <")),
            ["<00:01.00 unclosed <-00:02.00>x <"]
        );
    }

    #[test]
    fn line_of_only_word_tags_is_dropped() {
        assert_eq!(
            synced("[00:01.00]<00:01.00> <00:02.00>\n[00:03.00]kept"),
            vec![line(3.0, "kept")]
        );
        assert_eq!(parse_lrc("[00:01.00]<00:01.00>"), None);
    }

    #[test]
    fn whitespace_between_stacked_leading_tags() {
        assert_eq!(
            synced("[00:10.00] [00:20.00]x"),
            vec![line(10.0, "x"), line(20.0, "x")]
        );
    }

    #[test]
    fn many_word_tags_do_not_crash() {
        let tags = "<00:01.00>a ".repeat(50_000);
        assert_eq!(
            synced(&format!("[00:01.00]{tags}"))[0].text.chars().count(),
            99_999
        );
        let angles = "<".repeat(500_000);
        assert_eq!(
            synced(&format!("[00:01.00]{angles}"))[0]
                .text
                .chars()
                .count(),
            500_000
        );
    }

    // MARK: - Hostile input

    #[test]
    fn huge_minutes_value_does_not_crash() {
        let huge = "[99999999999999999999999999:00.00]x";
        assert_eq!(parse_lrc(huge), Some(Lyrics::Plain(huge.into())));

        let big = synced("[999:59.999]last");
        assert_close(big[0].time, 999.0 * 60.0 + 59.999, 0.0001, "big");
    }

    #[test]
    fn huge_seconds_and_fraction_do_not_crash() {
        let a = "[00:99999999999999999999.00]x";
        assert_eq!(parse_lrc(a), Some(Lyrics::Plain(a.into())));
        let b = "[00:01.99999999999999999999]x";
        assert_eq!(parse_lrc(b), Some(Lyrics::Plain(b.into())));
    }

    #[test]
    fn negative_looking_tags_are_not_timestamps() {
        let raw = "[-00:05.00]x\n[00:-5.00]y\n[+00:05.00]z";
        assert_eq!(parse_lrc(raw), Some(Lyrics::Plain(raw.into())));
    }

    #[test]
    fn malformed_tags_are_not_timestamps() {
        for raw in [
            "[00:01.00",
            "00:01.00]x",
            "[:01.00]x",
            "[00:]x",
            "[00:01.]x",
            "[a0:01.00]x",
            "[00:01.00x]y",
            "[]x",
            "[",
        ] {
            assert_eq!(parse_lrc(raw), Some(Lyrics::Plain(raw.into())), "{raw}");
        }
    }

    #[test]
    fn tags_only_lines_produce_no_lines() {
        assert_eq!(parse_lrc("[00:00.00]"), None);
        assert_eq!(parse_lrc("[00:01.00][00:02.00]\n[00:03.00]   "), None);
    }

    #[test]
    fn empty_text_at_zero_is_dropped_but_real_lines_survive() {
        assert_eq!(synced("[00:00.00]\n[00:05.00]go"), vec![line(5.0, "go")]);
    }

    #[test]
    fn overlong_line_does_not_crash() {
        let long_text = "a".repeat(2_000_000);
        let lines = synced(&format!("[00:01.00]{long_text}"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text.chars().count(), 2_000_000);

        let noise = "[".repeat(500_000);
        assert_eq!(parse_lrc(&noise), Some(Lyrics::Plain(noise.clone())));
    }

    #[test]
    fn many_tags_on_one_line_do_not_crash() {
        let tags = "[00:01.00]".repeat(20_000);
        assert_eq!(synced(&format!("{tags}x")).len(), 20_000);
    }

    // MARK: - lineIndex

    fn abc() -> Lyrics {
        Lyrics::Synced(vec![line(0.0, "a"), line(10.0, "b"), line(20.0, "c")])
    }

    #[test]
    fn line_index_at_position() {
        let lyrics = abc();
        assert_eq!(lyrics.line_index(0.0), Some(0));
        assert_eq!(lyrics.line_index(9.9), Some(0));
        assert_eq!(lyrics.line_index(10.0), Some(1));
        assert_eq!(lyrics.line_index(999.0), Some(2));
    }

    #[test]
    fn line_index_before_first_line_is_nil() {
        let lyrics = Lyrics::Synced(vec![line(5.0, "a")]);
        assert_eq!(lyrics.line_index(1.0), None);
        assert_eq!(lyrics.line_index(-3.0), None);
        assert_eq!(lyrics.line_index(f64::NAN), None);
    }

    #[test]
    fn line_index_on_plain_lyrics_is_nil() {
        assert_eq!(Lyrics::Plain("words".into()).line_index(10.0), None);
    }

    #[test]
    fn line_index_with_duplicate_timestamps_picks_last() {
        let lyrics = Lyrics::Synced(vec![
            line(1.0, "a"),
            line(5.0, "b"),
            line(5.0, "c"),
            line(9.0, "d"),
        ]);
        assert_eq!(lyrics.line_index(5.0), Some(2));
        assert_eq!(lyrics.line_index(6.0), Some(2));
        assert_eq!(lyrics.line_index(f64::INFINITY), Some(3));
    }

    #[test]
    fn is_synced() {
        assert!(Lyrics::Synced(vec![line(0.0, "a")]).is_synced());
        assert!(!Lyrics::Plain("a".into()).is_synced());
    }

    // MARK: - Plausibility

    fn synced_lyrics(last_line_at: f64) -> Lyrics {
        Lyrics::Synced(vec![line(10.0, "a"), line(last_line_at, "b")])
    }

    #[test]
    fn synced_with_two_lines_within_duration_is_plausible() {
        assert!(synced_lyrics(150.0).is_plausible(Some(200.0)));
    }

    #[test]
    fn single_line_synced_is_not_plausible() {
        let placeholder = Lyrics::Synced(vec![line(0.0, "純音樂，請欣賞")]);
        assert!(!placeholder.is_plausible(Some(200.0)));
        assert!(!placeholder.is_plausible(None));
    }

    #[test]
    fn last_line_beyond_duration_plus_tolerance_is_not_plausible() {
        assert!(!synced_lyrics(320.0).is_plausible(Some(200.0)));
    }

    #[test]
    fn last_line_exactly_at_duration_plus_tolerance_is_plausible() {
        assert_eq!(Lyrics::DURATION_TOLERANCE, 5.0);
        assert!(synced_lyrics(205.0).is_plausible(Some(200.0)));
    }

    #[test]
    fn last_line_just_past_tolerance_is_not_plausible() {
        assert!(!synced_lyrics(205.1).is_plausible(Some(200.0)));
    }

    #[test]
    fn unknown_or_zero_duration_only_applies_line_count_rule() {
        let late = synced_lyrics(9_999.0);
        assert!(late.is_plausible(None));
        assert!(late.is_plausible(Some(0.0)));
        let single = Lyrics::Synced(vec![line(1.0, "only")]);
        assert!(!single.is_plausible(None));
        assert!(!single.is_plausible(Some(0.0)));
    }

    #[test]
    fn plain_is_always_plausible() {
        assert!(Lyrics::Plain("words".into()).is_plausible(Some(1.0)));
        assert!(Lyrics::Plain("words".into()).is_plausible(None));
        assert!(Lyrics::Plain(String::new()).is_plausible(Some(0.0)));
    }

    // MARK: - Review-focus additions (Task 4)

    /// 最快的一次耗時（取最小值以濾掉排程雜訊）。
    fn best_of(runs: usize, input: &str) -> Duration {
        (0..runs)
            .map(|_| {
                let start = Instant::now();
                black_box(parse_lrc(black_box(input)));
                start.elapsed()
            })
            .min()
            .expect("at least one run")
    }

    #[test]
    fn parse_lrc_handles_a_one_megabyte_single_line_in_linear_time() {
        const MIB: usize = 1 << 20;
        // 每個輸入都是單行；輸入加倍時耗時不得超過約 3 倍（二次方行為會約 4 倍以上）。
        // 長度以位元組為單位湊到 `len`，再用「前綴 + 重複片段」建構。
        let build = |prefix: &str, unit: &str, len: usize| {
            let mut s = String::with_capacity(len + prefix.len());
            s.push_str(prefix);
            while s.len() < len {
                s.push_str(unit);
            }
            s
        };
        let shapes: [(&str, &str, &str); 5] = [
            ("word tags", "[00:01.00]", "<00:01.00>a "),
            ("angle brackets", "[00:01.00]", "<"),
            ("open brackets", "", "["),
            ("stacked tags", "", "[00:01.00]"),
            ("plain text", "[00:01.00]", "a"),
        ];
        for (name, prefix, unit) in shapes {
            let small = build(prefix, unit, MIB);
            let double = build(prefix, unit, 2 * MIB);
            assert!(!small.contains('\n') && small.len() >= MIB);
            // 先暖機一次，再各取 3 次的最小值。
            black_box(parse_lrc(&small));
            let t1 = best_of(3, &small);
            let t2 = best_of(3, &double);
            // 以 3 倍為界，並容許 1 ms 的量測底噪。
            assert!(
                t2 <= t1 * 3 + Duration::from_millis(1),
                "{name}: 1 MiB took {t1:?}, 2 MiB took {t2:?}"
            );
        }
    }

    #[test]
    fn parse_lrc_treats_huge_minute_numbers_as_not_a_time_tag() {
        for raw in ["[99999999999:00]x", "[99999999999:00]"] {
            assert_eq!(parse_lrc(raw), Some(Lyrics::Plain(raw.into())), "{raw}");
        }
        // 超過三位的分鐘不是時間標籤，即使後面還有合法標籤也只有合法那行被收。
        let lines = synced("[99999999999:00]x\n[00:01.00]y");
        assert_eq!(lines, vec![line(1.0, "y")]);
    }

    #[test]
    fn parse_lrc_without_any_time_tag_is_plain() {
        assert_eq!(
            parse_lrc("la la la\n\n  da da"),
            Some(Lyrics::Plain("la la la\n\n  da da".into()))
        );
    }

    #[test]
    fn metadata_only_tags_are_plain() {
        let raw = "[ar:Artist]\n[al:Album]\n[by:me]\n[offset:+250]";
        assert_eq!(parse_lrc(raw), Some(Lyrics::Plain(raw.into())));
    }

    #[test]
    fn timestamps_on_empty_lines_only_returns_none() {
        assert_eq!(
            parse_lrc("[00:01.00]\n[00:02.00]  \n[00:03.00]<00:03.00>"),
            None
        );
    }

    // MARK: - Unicode semantics (match Swift Character.isNewline / isWhitespace)

    #[test]
    fn unicode_line_separators_split_lines() {
        for sep in ["\u{0B}", "\u{0C}", "\u{85}", "\u{2028}", "\u{2029}"] {
            let lines = synced(&format!("[00:01.00]a{sep}[00:02.00]b"));
            assert_eq!(texts(&lines), ["a", "b"], "separator {:?}", sep);
        }
    }

    #[test]
    fn unicode_whitespace_is_trimmed_around_text() {
        let lines = synced("\u{00A0}[00:01.00]\u{3000}hi\u{00A0}\u{2003}");
        assert_eq!(texts(&lines), ["hi"]);
    }

    #[test]
    fn repeated_byte_order_marks_are_all_ignored() {
        assert_eq!(texts(&synced("\u{FEFF}\u{FEFF}[00:01.00]hi")), ["hi"]);
    }

    /// 第一行的時間是 NaN（手工建構才可能，解析器不會產生）：與 Swift 的 `position >= first.time`
    /// 守門相同，沒有任何位置「到得了」第一行，所以是 `None`，而且不得在 `partition_point - 1` 下溢位。
    #[test]
    fn line_index_with_a_nan_first_line_is_nil_and_does_not_underflow() {
        let lyrics = Lyrics::Synced(vec![line(f64::NAN, "a"), line(1.0, "b")]);
        assert_eq!(lyrics.line_index(5.0), None);
        assert_eq!(lyrics.line_index(f64::INFINITY), None);
        assert_eq!(lyrics.line_index(f64::NAN), None);
    }

    #[test]
    fn line_index_on_empty_synced_lyrics_is_nil() {
        assert_eq!(Lyrics::Synced(vec![]).line_index(5.0), None);
    }

    /// 預算的邊界：輸出文字總量剛好低於 32 MiB 的合法（雖然罕見）輸入仍要被接受，
    /// 多一點點才被拒絕。4096 個標籤 × 每行文字位元組數，輸入本身只有約 40 KB，輸出約 32 MiB
    /// （測試中只配置這一份，不慢）。
    #[test]
    fn input_just_under_the_output_budget_is_still_accepted() {
        const TAGS: usize = 4_096;
        let per_line = MAX_SYNCED_TEXT_BYTES / TAGS - 1; // 8191：TAGS 行共 33_550_336 < 預算
        assert!(TAGS * per_line < MAX_SYNCED_TEXT_BYTES);
        let raw = format!("{}{}", "[00:01.00]".repeat(TAGS), "a".repeat(per_line));

        let lines = synced(&raw);

        assert_eq!(lines.len(), TAGS);
        assert!(lines.iter().all(|l| l.text.len() == per_line));
    }

    #[test]
    fn input_just_over_the_output_budget_is_rejected() {
        const TAGS: usize = 4_096;
        let per_line = MAX_SYNCED_TEXT_BYTES / TAGS + 1; // 超出預算 TAGS 位元組
        let raw = format!("{}{}", "[00:01.00]".repeat(TAGS), "a".repeat(per_line));
        assert_eq!(parse_lrc(&raw), None);
    }

    #[test]
    fn hostile_tag_and_text_amplification_is_rejected_not_exhausted() {
        // 20_000 個標籤乘上 2 MB 文字會需要數十 GB；超過輸出預算時視為沒有歌詞。
        let raw = format!("{}{}", "[00:01.00]".repeat(20_000), "a".repeat(2_000_000));
        assert_eq!(parse_lrc(&raw), None);
    }
}
