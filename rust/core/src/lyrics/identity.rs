//! 曲目識別與寬鬆比對（移植自 Swift `TrackIdentity.swift`）。
//!
//! # 簡繁折疊套件的選擇（Task 5 評估紀錄）
//!
//! Swift 版用 `CFStringTransform("Hans-Hant")` 把兩邊轉成同一種文字再比。Rust 沒有系統
//! 轉換器，需要一個純 Rust、可在 Windows 編譯、授權相容（本專案 `MIT OR Apache-2.0`）的套件。
//!
//! - `zhconv` 0.4.2：功能上可行——關掉預設的 `compress`（會拉進 C 的 `zstd-sys`）後是純 Rust，
//!   `("後來有很多人去過那裡", "后来有很多人去过那里")` 折疊後相等、`頭髮 → 头发`、`愛你/愛她` 不混。
//!   **但它的授權是 `GPL-2.0-or-later`**（資料取自 MediaWiki 的 ZhConversion），與本專案授權
//!   不相容，連結進發行檔會把整個發行檔拖進 GPL。因此不採用。
//! - `fast2s` 0.3.1（MIT）：只有逐字對照（約 4 千字）、僅繁→簡、少量上下文規則、無詞組；
//!   依賴很輕。可行的備案。
//! - `ferrous-opencc` 0.4.0（Apache-2.0）：OpenCC 的純 Rust 實作，字典內建（不讀外部檔案），
//!   有詞組層級的 `T2s`。**採用。** 代價是依賴樹較大（`fst`、`rkyv`、`phf` 等，約 40 個 crate）、
//!   編譯較久；本機實測 release 範例二進位約 0.9 MB，首次 `OpenCC::from_config` 約 0.7 ms。
//!
//! 折疊方向：兩邊都轉成**簡體**（`T2s`）。繁→簡是多對一，不需要簡→繁那種依上下文猜
//! `后/後`、`发/發/髮` 的方向；兩邊同轉所以對照結果對稱。簡體輸入原樣通過。
//!
//! # 其餘折疊步驟
//!
//! 全半形：NFKC（`ＡＢＣ → ABC`；也會折疊其他相容字元，如 `ﬁ → fi`、`² → 2`，比 Foundation 的
//! `widthInsensitive` 稍廣，對比對歌名無害）。大小寫：`to_lowercase`。去重音符號：NFD 後
//! 只丟掉**拉丁系**組合符號區塊（U+0300–036F 等）。不丟全部 `Mn`：日文濁點／半濁點
//! （U+3099／309A）是字義的一部分，丟掉會讓 `が == か`、`ぱ == ば`，查到別首歌的歌詞。

use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use ferrous_opencc::config::BuiltinConfig;
use ferrous_opencc::OpenCC;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// 歌詞提供者被要求查詢的對象。
///
/// 與 `TrackSnapshot` 分開，因為查詢只在乎「是哪首歌」而不在乎播放狀態，
/// 而下面的正規化規則完全是為查詢服務的。
///
/// `Hash`／`Eq` 手寫：`duration` 以位元（`to_bits`）參與，但先把 `-0.0` 併成 `0.0`、
/// 所有 NaN 併成同一個 NaN。如此 `Eq` 反身性成立（`NaN == NaN`），能安全當 `HashMap` 鍵。
/// Swift 的合成 `==` 讓 NaN 不等於自己；那樣的鍵永遠查不到，故刻意不照搬。
#[derive(Clone, Debug)]
pub struct TrackIdentity {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// 來源沒有回報長度時為 `None`。小於等於 0 的值照原樣保存，只是 [`Self::duration_matches`] 視為未知。
    pub duration: Option<f64>,
}

/// `duration` 的比對／雜湊用位元：`-0.0` 併入 `0.0`，所有 NaN 併成一個。
fn duration_key(duration: Option<f64>) -> Option<u64> {
    duration.map(|d| {
        if d.is_nan() {
            f64::NAN.to_bits()
        } else if d == 0.0 {
            0.0_f64.to_bits()
        } else {
            d.to_bits()
        }
    })
}

impl PartialEq for TrackIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title
            && self.artist == other.artist
            && self.album == other.album
            && duration_key(self.duration) == duration_key(other.duration)
    }
}

impl Eq for TrackIdentity {}

impl Hash for TrackIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.title.hash(state);
        self.artist.hash(state);
        self.album.hash(state);
        duration_key(self.duration).hash(state);
    }
}

/// `trimmingCharacters(in: .whitespaces)`：Unicode Zs 加 tab，**不含**換行類字元。
fn is_inline_whitespace(c: char) -> bool {
    c == '\t'
        || (c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ))
}

/// 結尾括號後綴：`(feat. …)`、`[Remastered 2011]`、`（Live）`、`(Explicit)`。
/// 關鍵字後必須是單字邊界，`Liverpool`、`Feather`、`Explicitly`、`Ftw` 不會被吃掉。
static SUFFIX_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[（【]\s*(?:(?:feat|ft)\b\.?|remaster(?:ed)?\b|live\b|explicit\b)[^\)\]）】]*[\)\]）】]",
    )
    .expect("suffix pattern is valid")
});

/// 繁→簡轉換器，只建一次。建立失敗（不應發生，字典內建）時退化為不轉換。
static T2S: LazyLock<Option<OpenCC>> =
    LazyLock::new(|| OpenCC::from_config(BuiltinConfig::T2s).ok());

/// 拉丁系組合符號區塊（見模組說明：不含日文濁點）。
fn is_latin_combining_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// 大小寫、全半形、重音符號不敏感的形式，不含簡繁折疊與 trim；
/// 供標題正規化的演出者比對共用（對應 Swift 的 `folding(options:locale:)`）。
pub(crate) fn fold_text(raw: &str) -> String {
    raw.nfkc()
        .collect::<String>()
        .to_lowercase()
        .nfd()
        .filter(|c| !is_latin_combining_mark(*c))
        .collect()
}

impl TrackIdentity {
    /// 較寬鬆的查詢形式。精確查詢可能只因為系統回報的標題少了資料庫帶有的
    /// `(feat. ...)` 後綴（或相反）而落空。
    pub fn normalized(&self) -> TrackIdentity {
        let stripped = SUFFIX_PATTERN.replace_all(&self.title, "");
        let stripped_title = stripped.trim_matches(is_inline_whitespace);

        let primary_artist = self
            .artist
            .split(['/', '&', ',', ';'])
            .next()
            .unwrap_or(&self.artist)
            .trim_matches(is_inline_whitespace);

        TrackIdentity {
            title: if stripped_title.is_empty() {
                self.title.clone()
            } else {
                stripped_title.to_string()
            },
            artist: if primary_artist.is_empty() {
                self.artist.clone()
            } else {
                primary_artist.to_string()
            },
            album: self.album.clone(),
            duration: self.duration,
        }
    }

    /// 正規化後的標題與演出者，小寫，不含長度；以 `\u{1}` 連接。
    pub fn cache_key(&self) -> String {
        let n = self.normalized();
        format!("{}\u{1}{}", n.title.to_lowercase(), n.artist.to_lowercase())
    }

    /// 容忍而非相等：一首歌的另一個版本常與原版差幾秒。任一邊缺少或不大於 0
    /// 都視為「未知」，永遠不會拒絕候選。NaN 同樣視為未知。
    pub fn duration_matches(&self, other: Option<f64>) -> bool {
        let (Some(duration), Some(other)) = (self.duration, other) else {
            return true;
        };
        // 用否定的 `>` 寫法讓 NaN 也落入「未知」。
        if !(duration > 0.0 && other > 0.0) {
            return true;
        }
        (duration - other).abs() <= 5.0
    }

    /// 不要用 `==` 比對提供者回來的歌名：同一首歌可能以簡體、不同大小寫或全形字元回來。
    /// 兩邊先折成同一種文字再比。
    pub fn loosely_matches(a: &str, b: &str) -> bool {
        fold(a) == fold(b)
    }
}

fn fold(raw: &str) -> String {
    // 先 NFKC 再轉簡體：NFKC 會把相容漢字（如 U+F900 區）併到統一漢字，讓字典查得到。
    let composed: String = raw.nfkc().collect();
    let simplified = match T2S.as_ref() {
        Some(cc) => cc.convert(&composed),
        None => composed,
    };
    fold_text(&simplified)
        .trim_matches(char::is_whitespace)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn make(title: &str, artist: &str, duration: Option<f64>) -> TrackIdentity {
        TrackIdentity {
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            duration,
        }
    }

    /// Swift 測試的預設值：artist "A"、duration 200。
    fn t(title: &str) -> TrackIdentity {
        make(title, "A", Some(200.0))
    }

    fn hash_of(id: &TrackIdentity) -> u64 {
        let mut h = DefaultHasher::new();
        id.hash(&mut h);
        h.finish()
    }

    #[test]
    fn strips_feat_suffix() {
        assert_eq!(t("善良是 (feat. CHS)").normalized().title, "善良是");
        assert_eq!(t("Song (ft. Someone)").normalized().title, "Song");
        assert_eq!(t("Song [Remastered 2011]").normalized().title, "Song");
    }

    #[test]
    fn strips_full_width_bracket_suffixes() {
        assert_eq!(t("歌名（feat. X）").normalized().title, "歌名");
        assert_eq!(t("歌名【Remastered】").normalized().title, "歌名");
        assert_eq!(t("歌名（Live）").normalized().title, "歌名");
    }

    /// 關鍵字必須是完整單字："Live" 不得吃掉 "Liverpool"。
    #[test]
    fn suffix_keywords_need_word_boundary() {
        assert_eq!(
            t("Song (Liverpool Mix)").normalized().title,
            "Song (Liverpool Mix)"
        );
        assert_eq!(t("Song (Feather)").normalized().title, "Song (Feather)");
        assert_eq!(
            t("Song (Explicitly Yours)").normalized().title,
            "Song (Explicitly Yours)"
        );
        assert_eq!(t("Song (Ftw)").normalized().title, "Song (Ftw)");
    }

    #[test]
    fn leaves_clean_title_alone() {
        assert_eq!(
            t("後來有很多人去過那裡").normalized().title,
            "後來有很多人去過那裡"
        );
    }

    #[test]
    fn takes_primary_artist() {
        assert_eq!(
            make("T", "someshiit 山姆/CHS", Some(200.0))
                .normalized()
                .artist,
            "someshiit 山姆"
        );
        assert_eq!(make("T", "A & B", Some(200.0)).normalized().artist, "A");
        assert_eq!(make("T", "A, B", Some(200.0)).normalized().artist, "A");
    }

    #[test]
    fn duration_tolerance_is_five_seconds() {
        let track = make("T", "A", Some(200.0));
        assert!(track.duration_matches(Some(200.0)));
        assert!(track.duration_matches(Some(204.9)));
        assert!(track.duration_matches(Some(195.1)));
        assert!(!track.duration_matches(Some(210.0)));
        assert!(!track.duration_matches(Some(190.0)));
    }

    /// 任一邊不知道曲長都不得拒絕候選。
    #[test]
    fn unknown_duration_always_matches() {
        assert!(make("T", "A", Some(200.0)).duration_matches(None));
        assert!(make("T", "A", Some(200.0)).duration_matches(Some(0.0)));
        assert!(make("T", "A", None).duration_matches(Some(200.0)));
        assert!(make("T", "A", Some(0.0)).duration_matches(Some(200.0)));
        assert!(make("T", "A", None).duration_matches(None));
    }

    #[test]
    fn loose_match_ignores_simplified_traditional() {
        assert!(TrackIdentity::loosely_matches(
            "後來有很多人去過那裡",
            "后来有很多人去过那里"
        ));
        assert!(TrackIdentity::loosely_matches("Song", "song"));
        assert!(TrackIdentity::loosely_matches("ＡＢＣ", "abc"));
        assert!(!TrackIdentity::loosely_matches("Song", "Different"));
    }

    #[test]
    fn cache_key_ignores_duration() {
        assert_eq!(
            make("T", "A", Some(200.0)).cache_key(),
            make("T", "A", Some(260.0)).cache_key()
        );
        assert_eq!(
            make("T", "A", Some(200.0)).cache_key(),
            make("T", "A", None).cache_key()
        );
    }

    #[test]
    fn cache_key_collapses_case_and_feat_suffix() {
        assert_eq!(
            make("Song (feat. X)", "Artist", Some(200.0)).cache_key(),
            make("song", "ARTIST", Some(200.0)).cache_key()
        );
    }

    // ---- 以下為 Rust 版新增（Swift 沒有的案例） ----

    #[test]
    fn loosely_matches_ignores_script_case_and_width() {
        assert!(TrackIdentity::loosely_matches("愛你", "爱你"));
        assert!(TrackIdentity::loosely_matches("ＡＢＣ", "abc"));
        assert!(TrackIdentity::loosely_matches("Beyoncé", "beyonce"));
        assert!(!TrackIdentity::loosely_matches("愛你", "愛她"));
    }

    #[test]
    fn loosely_matches_trims_and_handles_decomposed_diacritics() {
        assert!(TrackIdentity::loosely_matches("  Song \n", "song"));
        // e + U+0301（已分解）與 é（已組合）視為相同。
        assert!(TrackIdentity::loosely_matches("Beyonce\u{301}", "Beyoncé"));
        assert!(TrackIdentity::loosely_matches("", "   "));
    }

    /// 去重音符號只動拉丁系組合符號：日文濁點是字義的一部分，不能被吃掉。
    #[test]
    fn loosely_matches_keeps_kana_voicing_marks_significant() {
        assert!(!TrackIdentity::loosely_matches("が", "か"));
        assert!(!TrackIdentity::loosely_matches("ぱ", "ば"));
        assert!(TrackIdentity::loosely_matches("が", "が"));
    }

    #[test]
    fn loosely_matches_does_not_panic_on_odd_input() {
        assert!(TrackIdentity::loosely_matches("😀", "😀"));
        assert!(!TrackIdentity::loosely_matches("😀", "😁"));
        assert!(TrackIdentity::loosely_matches("\u{301}", ""));
        assert!(TrackIdentity::loosely_matches(
            "a\u{1F468}\u{200D}\u{1F469}",
            "A\u{1F468}\u{200D}\u{1F469}"
        ));
    }

    #[test]
    fn cache_key_joins_normalized_lowercase_title_and_artist_with_separator() {
        assert_eq!(
            make("Song (Feat. X)", "ARTIST/Other", Some(1.0)).cache_key(),
            "song\u{1}artist"
        );
    }

    #[test]
    fn normalized_falls_back_when_stripping_leaves_nothing() {
        // 標題整個是後綴：不得變空字串。
        assert_eq!(t("(feat. X)").normalized().title, "(feat. X)");
        // 演出者以分隔符開頭：第一段為空，回退原值。
        assert_eq!(make("T", "/B", None).normalized().artist, "/B");
        assert_eq!(make("T", "", None).normalized().artist, "");
    }

    #[test]
    fn normalized_keeps_album_and_duration() {
        let id = TrackIdentity {
            title: "S (Live)".into(),
            artist: "A;B".into(),
            album: Some("Alb".into()),
            duration: Some(-3.0),
        };
        let n = id.normalized();
        assert_eq!(n.album.as_deref(), Some("Alb"));
        assert_eq!(n.duration, Some(-3.0));
        assert_eq!(n.title, "S");
        assert_eq!(n.artist, "A");
    }

    #[test]
    fn normalized_does_not_panic_on_emoji_and_combining_characters() {
        let id = make("😀 (feat. 🎤)\u{301}", "🎤\u{301}/😀", Some(10.0));
        let n = id.normalized();
        assert_eq!(n.artist, "🎤\u{301}");
        let _ = id.cache_key();
        let _ = make("\u{301}", "\u{200D}", None).normalized();
    }

    #[test]
    fn duration_non_positive_or_nan_means_unknown() {
        assert!(make("T", "A", Some(-5.0)).duration_matches(Some(200.0)));
        assert!(make("T", "A", Some(200.0)).duration_matches(Some(-1.0)));
        assert!(make("T", "A", Some(f64::NAN)).duration_matches(Some(200.0)));
        assert!(make("T", "A", Some(200.0)).duration_matches(Some(f64::NAN)));
        assert!(make("T", "A", Some(200.0)).duration_matches(Some(205.0)));
        assert!(!make("T", "A", Some(200.0)).duration_matches(Some(205.1)));
    }

    #[test]
    fn identity_equality_and_hash_use_duration_bits() {
        let a = make("T", "A", Some(200.0));
        let b = make("T", "A", Some(200.0));
        let c = make("T", "A", Some(200.5));
        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
        assert_ne!(a, c);
        assert_ne!(a, make("T", "A", None));
        assert_ne!(make("T", "A", Some(0.0)), make("T", "A", None));
    }

    /// 與 Swift 的 `Double ==` 一致：`0.0 == -0.0`。NaN 與自己相等（Swift 不是，
    /// 但那樣 `Hash + Eq` 不成立，用作 `HashMap` 鍵會出現永遠找不到的項目）。
    #[test]
    fn identity_equality_normalises_negative_zero_and_nan() {
        let pz = make("T", "A", Some(0.0));
        let nz = make("T", "A", Some(-0.0));
        assert_eq!(pz, nz);
        assert_eq!(hash_of(&pz), hash_of(&nz));

        let n1 = make("T", "A", Some(f64::NAN));
        let n2 = make("T", "A", Some(-f64::NAN));
        assert_eq!(n1, n2);
        assert_eq!(hash_of(&n1), hash_of(&n2));
        assert_eq!(n1, n1.clone());
    }

    #[test]
    fn identity_works_as_hash_map_key() {
        let mut m = std::collections::HashMap::new();
        m.insert(make("T", "A", Some(f64::NAN)), 1);
        assert_eq!(m.get(&make("T", "A", Some(f64::NAN))), Some(&1));
        assert_eq!(m.get(&make("T", "A", Some(1.0))), None);
    }
}
