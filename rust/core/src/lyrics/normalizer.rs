//! 把播放器回報的內容轉成歌詞提供者查得動的形式（移植自 Swift `TrackNormalizer.swift`）。
//!
//! 瀏覽器內的播放器回報的是影片標題，常常內嵌頻道名稱、合作演出者與 MV 風格標籤，
//! 例如頻道 `蘇裔非` 的 `蘇裔非 - 抱我!  ft. 莊岱諺`。
//!
//! 正規表達式照 Swift 版逐條搬移，沒有用到 look-around（`regex` 不支援），所以不需要改結構。
//! 與 ICU（`NSRegularExpression`）的差異：`\b`、`\s`、`(?i)` 在 `regex` 都是 Unicode 版，
//! 對本檔的輸入（ASCII 關鍵字加全形括號）行為一致；`\s` 另含 U+0085 與 U+000B，影響可忽略。
//!
//! Rust 版另外認得官方 MV 的標題 `歌手【歌名 English】Official MV`（Swift 版沒有）：整串拿去查一定落空。

use std::sync::LazyLock;

use regex::Regex;

use super::identity::{fold_text, is_han, TrackIdentity};
use crate::types::TrackSnapshot;

/// 標題缺漏或沒有演出者時回傳 `None`：沒有演出者就沒有可靠的查詢，所以不嘗試。
pub fn identity_from(snapshot: &TrackSnapshot) -> Option<TrackIdentity> {
    let raw_title = snapshot.title.trim();
    if raw_title.is_empty() {
        return None;
    }
    let artist = snapshot.artist.as_deref()?.trim();
    if artist.is_empty() {
        return None;
    }

    Some(TrackIdentity {
        title: clean_title(raw_title, artist),
        artist: artist.to_string(),
        album: snapshot.album.clone(),
        duration: snapshot.duration,
    })
}

static DASH_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+[-–—]\s+").expect("dash pattern is valid"));

/// 括號群組（ASCII 或全形）內任何位置含有合作演出者段落。整個群組一起移除，不留下開括號。
static BRACKETED_FEAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[\(\[（【][^\)\]）】]*\b(?:ft\.|feat\.|featuring\s)[^\)\]）】]*[\)\]）】]")
        .expect("bracketed feat pattern is valid")
});

/// 裸的 `ft.` / `feat.` / `featuring` 加上其後的全部內容。
static FEAT_SEGMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\s*\b(?:ft\.|feat\.|featuring\s).*$").expect("feat segment pattern is valid")
});

/// 描述上傳本身而非歌曲的括號標籤。
static VIDEO_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[（【]\s*(?:official\s+)?(?:music\s+video|lyric\s+video|lyrics?\s+video|video|mv|m/v|lyrics?|audio|visualizer|官方[^\)\]）】]*|歌詞[^\)\]）】]*|歌词[^\)\]）】]*)\s*[\)\]）】]",
    )
    .expect("video tag pattern is valid")
});

/// 官方 MV 的整個標題：開頭、`【…】`、結尾 `Official MV`（或 `Official Music Video`）。
static OFFICIAL_MV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^([^【】]+)【([^【】]+)】\s*official\s+(?:music\s+video|mv|m/v)$")
        .expect("official mv pattern is valid")
});

static WHITESPACE_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("whitespace pattern is valid"));

fn clean_title(title: &str, artist: &str) -> String {
    let mut result = official_mv_song(title, artist)
        .or_else(|| split_off_artist(title, artist))
        .unwrap_or(title)
        .to_string();
    result = BRACKETED_FEAT.replace_all(&result, "").into_owned();
    result = FEAT_SEGMENT.replace_all(&result, "").into_owned();
    result = VIDEO_TAG.replace_all(&result, "").into_owned();
    let collapsed = WHITESPACE_RUN.replace_all(&result, " ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        title.to_string()
    } else {
        trimmed.to_string()
    }
}

/// `A - B` 中的 `B`，條件是 `A` 等於演出者或被演出者包含；標題不是這個形狀時為 `None`。
/// 切點來自 `regex` 的位元組位移，必在字元邊界上，所以對 emoji／組合字元不會 panic。
fn split_off_artist<'a>(title: &'a str, artist: &str) -> Option<&'a str> {
    let separator = DASH_SEPARATOR.find(title)?;
    let head = &title[..separator.start()];
    let tail = title[separator.end()..].trim();
    let squashed_head = squash(head);
    if squashed_head.is_empty() || tail.is_empty() || !squash(artist).contains(&squashed_head) {
        return None;
    }
    Some(tail)
}

/// `歌手【歌名】Official MV` 裡的歌名；標題不是這個形狀、開頭不是演出者（或其一部分）、
/// 或括號裡沒有字時為 `None`。
fn official_mv_song<'a>(title: &'a str, artist: &str) -> Option<&'a str> {
    let captures = OFFICIAL_MV.captures(title)?;
    let head = squash(captures.get(1)?.as_str());
    if head.is_empty() || !squash(artist).contains(&head) {
        return None;
    }
    let name = captures.get(2)?.as_str().trim();
    (!name.is_empty()).then(|| without_english_subtitle(name))
}

/// `愛在西元前 Love before AD` → `愛在西元前`（帶著副標查不到歌詞）。只在最後一個漢字之後、
/// 從空白起的部分含英文字母時切掉；沒有漢字的歌名、結尾是漢字或數字的歌名原樣保留。
fn without_english_subtitle(name: &str) -> &str {
    let Some((index, last)) = name.char_indices().rfind(|(_, c)| is_han(*c)) else {
        return name;
    };
    let end = index + last.len_utf8();
    let tail = &name[end..];
    match tail.find(char::is_whitespace) {
        Some(space) if tail[space..].contains(|c: char| c.is_ascii_alphabetic()) => {
            &name[..end + space]
        }
        _ => name,
    }
}

/// 大小寫、全半形、空白不敏感的形式，用於演出者比對。
fn squash(text: &str) -> String {
    fold_text(text)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        title: &str,
        artist: Option<&str>,
        album: Option<&str>,
        duration: Option<f64>,
    ) -> TrackSnapshot {
        TrackSnapshot {
            title: title.to_string(),
            artist: artist.map(str::to_string),
            album: album.map(str::to_string),
            duration,
            is_playing: true,
            position: None,
            artwork: None,
            source_id: "test".to_string(),
        }
    }

    fn title(title: &str, artist: Option<&str>) -> Option<String> {
        identity_from(&snapshot(title, artist, None, Some(200.0))).map(|i| i.title)
    }

    /// 取自 Chrome 上 YouTube 的真實樣本（注意 "ft." 前有兩個空格）。
    #[test]
    fn you_tube_title_splits_off_artist() {
        assert_eq!(
            title("蘇裔非 - 抱我!  ft. 莊岱諺", Some("蘇裔非")).as_deref(),
            Some("抱我!")
        );
    }

    #[test]
    fn collapses_extra_whitespace() {
        assert_eq!(
            title("Some   Song    Name", Some("X")).as_deref(),
            Some("Some Song Name")
        );
        assert_eq!(
            title("X  -   Some   Song", Some("X")).as_deref(),
            Some("Some Song")
        );
    }

    #[test]
    fn strips_mv_brackets() {
        assert_eq!(title("X - Song (MV)", Some("X")).as_deref(), Some("Song"));
        assert_eq!(
            title("X - Song [Official Video]", Some("X")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("X - Song (Lyrics)", Some("X")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("Song (Official Music Video)", Some("X")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("X - Song feat. Y [Official Video]", Some("X")).as_deref(),
            Some("Song")
        );
    }

    #[test]
    fn strips_feat_segment() {
        assert_eq!(
            title("Song feat. Someone Else", Some("X")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("Song Ft. Someone", Some("X")).as_deref(),
            Some("Song")
        );
    }

    #[test]
    fn no_dash_keeps_original_title() {
        assert_eq!(title("抱我!", Some("蘇裔非")).as_deref(), Some("抱我!"));
        assert_eq!(
            title("Hyphen-ated Title", Some("X")).as_deref(),
            Some("Hyphen-ated Title")
        );
    }

    #[test]
    fn dash_but_artist_does_not_match_keeps_original() {
        assert_eq!(
            title("Live - Tonight Show", Some("Someone Else")).as_deref(),
            Some("Live - Tonight Show")
        );
    }

    #[test]
    fn artist_match_ignores_case_and_spacing() {
        assert_eq!(
            title("some artist - Song", Some("Some Artist")).as_deref(),
            Some("Song")
        );
        assert_eq!(title("A - Song", Some("A & B")).as_deref(), Some("Song"));
    }

    #[test]
    fn missing_artist_returns_nil() {
        assert!(identity_from(&snapshot("Song", None, None, Some(200.0))).is_none());
        assert!(identity_from(&snapshot("Song", Some(""), None, Some(200.0))).is_none());
        assert!(identity_from(&snapshot("Song", Some("   "), None, Some(200.0))).is_none());
    }

    #[test]
    fn missing_title_returns_nil() {
        assert!(identity_from(&snapshot("", Some("X"), None, Some(200.0))).is_none());
        assert!(identity_from(&snapshot("  ", Some("X"), None, Some(200.0))).is_none());
    }

    #[test]
    fn clean_music_app_title_untouched() {
        assert_eq!(
            title("後來有很多人去過那裡", Some("someshiit 山姆")).as_deref(),
            Some("後來有很多人去過那裡")
        );
        assert_eq!(
            title("Bohemian Rhapsody", Some("Queen")).as_deref(),
            Some("Bohemian Rhapsody")
        );
    }

    #[test]
    fn carries_artist_album_and_duration() {
        let id = identity_from(&snapshot("T", Some("Artist"), Some("Album"), Some(123.0)));
        assert_eq!(
            id,
            Some(TrackIdentity {
                title: "T".into(),
                artist: "Artist".into(),
                album: Some("Album".into()),
                duration: Some(123.0),
            })
        );
    }

    #[test]
    fn strips_feat_inside_any_bracket_group_without_dangling_opener() {
        assert_eq!(title("歌名（feat. X）", Some("Y")).as_deref(), Some("歌名"));
        assert_eq!(title("歌名【ft. X】", Some("Y")).as_deref(), Some("歌名"));
        assert_eq!(
            title("Song (Remix feat. X)", Some("Y")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("Song [feat. X] [Official Video]", Some("Y")).as_deref(),
            Some("Song")
        );
    }

    #[test]
    fn empty_result_falls_back_to_original_title() {
        assert_eq!(title("X - (MV)", Some("X")).as_deref(), Some("X - (MV)"));
    }

    #[test]
    fn cleaning_is_idempotent() {
        let first = title("蘇裔非 - 抱我!  ft. 莊岱諺", Some("蘇裔非"));
        assert_eq!(first.as_deref(), Some("抱我!"));
        assert_eq!(
            title(first.as_deref().unwrap_or(""), Some("蘇裔非")).as_deref(),
            Some("抱我!")
        );
    }

    #[test]
    fn only_first_dash_splits_off_artist() {
        assert_eq!(title("A - B - C", Some("A")).as_deref(), Some("B - C"));
    }

    // ---- 以下為 Rust 版新增（Swift 沒有的案例） ----

    /// 官方 MV 的「歌手【中文歌名 English】Official MV」：只取中文歌名。
    #[test]
    fn official_mv_title_yields_the_chinese_song_name() {
        let artist = Some("周杰倫 Jay Chou");
        for (raw, name) in [
            (
                "周杰倫 Jay Chou【最偉大的作品 Greatest Works of Art】Official MV",
                "最偉大的作品",
            ),
            (
                "周杰倫 Jay Chou【愛在西元前 Love before AD】Official MV",
                "愛在西元前",
            ),
            // 結尾標籤不分大小寫、也接受 Official Music Video；開頭只寫歌手名的一部分也算。
            ("周杰倫【晴天】 official music video", "晴天"),
        ] {
            assert_eq!(title(raw, artist).as_deref(), Some(name), "{raw}");
        }
    }

    /// 中文後面沒有英文副標時，括號裡的字整個是歌名。
    #[test]
    fn official_mv_title_keeps_names_without_an_english_subtitle() {
        let artist = Some("周杰倫 Jay Chou");
        for name in ["Mojito", "Now You See Me", "晴天 2.0", "我的 Superman 夢"] {
            let raw = format!("周杰倫 Jay Chou【{name}】Official MV");
            assert_eq!(title(&raw, artist).as_deref(), Some(name), "{raw}");
        }
    }

    /// 開頭是歌手、`【…】`、結尾 Official MV，三個條件缺一就不套用。
    #[test]
    fn mv_rule_needs_the_artist_prefix_and_the_official_mv_ending() {
        let artist = Some("周杰倫 Jay Chou");
        for raw in [
            "好聽歌曲【晴天】Official MV",
            "【晴天】Official MV",
            "周杰倫 Jay Chou【晴天】",
            "周杰倫 Jay Chou【晴天】Live",
            "周杰倫 Jay Chou【晴天】Official MV Reaction",
            "周杰倫 Jay Chou【 】Official MV",
        ] {
            assert_eq!(title(raw, artist).as_deref(), Some(raw), "{raw}");
        }
    }

    #[test]
    fn identity_from_does_not_panic_on_emoji_and_combining_characters() {
        let inputs = [
            ("😀 - 🎤 ft. 🎶", "😀"),
            ("e\u{301} - e\u{301}\u{301} (feat. \u{301})", "e\u{301}"),
            ("\u{301}", "\u{301}"),
            ("👨\u{200D}👩\u{200D}👧 - 歌 (MV)", "👨\u{200D}👩\u{200D}👧"),
            ("A - ", "A"),
            (" - A", "A"),
            ("（", "x"),
            ("(feat.", "x"),
            ("ｆｅａｔ． X", "x"),
        ];
        for (t, a) in inputs {
            let _ = identity_from(&snapshot(t, Some(a), None, None));
        }
        // 空白（含全形、不換行空白、換行）的標題與演出者一律視為缺漏。
        assert!(identity_from(&snapshot("\u{3000}\u{a0}\n", Some("x"), None, None)).is_none());
        assert!(identity_from(&snapshot("x", Some("\u{3000}\n"), None, None)).is_none());
    }

    #[test]
    fn emoji_titles_keep_their_text_after_cleaning() {
        assert_eq!(title("😀 - 歌 (MV)", Some("😀")).as_deref(), Some("歌"));
        assert_eq!(title("歌😀 ft. 🎤", Some("X")).as_deref(), Some("歌😀"));
    }

    #[test]
    fn non_positive_and_missing_duration_are_carried_as_given() {
        for d in [Some(0.0), Some(-1.0), None] {
            let id = identity_from(&snapshot("T", Some("A"), None, d)).unwrap();
            assert_eq!(id.duration, d);
        }
    }

    #[test]
    fn trims_artist_and_title_whitespace() {
        let id = identity_from(&snapshot("  Song \n", Some("\t Artist  "), None, None)).unwrap();
        assert_eq!(id.title, "Song");
        assert_eq!(id.artist, "Artist");
    }

    #[test]
    fn artist_matching_ignores_full_width_forms() {
        // 全形字母與半形視為相同（Swift 的 widthInsensitive）。
        assert_eq!(title("ＡＢＣ - Song", Some("abc")).as_deref(), Some("Song"));
    }

    #[test]
    fn dash_variants_split_only_when_surrounded_by_whitespace() {
        assert_eq!(title("A – Song", Some("A")).as_deref(), Some("Song"));
        assert_eq!(title("A — Song", Some("A")).as_deref(), Some("Song"));
        assert_eq!(title("A-Song", Some("A")).as_deref(), Some("A-Song"));
    }

    #[test]
    fn video_tags_in_chinese_and_full_width_brackets_are_stripped() {
        assert_eq!(
            title("歌名（官方完整版MV）", Some("Y")).as_deref(),
            Some("歌名")
        );
        assert_eq!(title("歌名【歌词版】", Some("Y")).as_deref(), Some("歌名"));
        assert_eq!(
            title("歌名 [歌詞 Lyrics]", Some("Y")).as_deref(),
            Some("歌名")
        );
        // 不是上傳標籤的括號內容保留。
        assert_eq!(
            title("Song (Acoustic)", Some("Y")).as_deref(),
            Some("Song (Acoustic)")
        );
    }

    #[test]
    fn featuring_keyword_needs_trailing_space_and_word_boundary() {
        assert_eq!(
            title("Song featuring Someone", Some("Y")).as_deref(),
            Some("Song")
        );
        assert_eq!(
            title("Craft. Song", Some("Y")).as_deref(),
            Some("Craft. Song")
        );
        assert_eq!(
            title("Soft. Song", Some("Y")).as_deref(),
            Some("Soft. Song")
        );
    }
}
