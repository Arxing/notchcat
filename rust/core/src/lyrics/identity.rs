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
//!   依賴備註：`ferrous-opencc` 匯出未使用的 `extern "C"` FFI 符號（`opencc_create`／`opencc_destroy`／
//!   `opencc_convert`／`opencc_free_string`），並對建置時產生的字典資料使用 `unsafe`；我們只走安全的 Rust
//!   API（`OpenCC::from_config`／`convert`），風險判斷為低。
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

/// 曲長比對的容忍秒數。
const DURATION_TOLERANCE: f64 = 5.0;
/// 沒有吻合的候選時，放寬到這個秒數（官方 MV 常比歌庫收錄的版本長或短幾秒）。
const LOOSE_DURATION_TOLERANCE: f64 = 10.0;

/// 候選歌詞的曲長與查詢有多吻合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DurationFit {
    /// 差 5 秒以內，或任一邊不知道曲長。
    Close,
    /// 差超過 5 秒、10 秒以內：只在沒有吻合的候選時使用。
    Loose,
}

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

/// 繁→簡轉換器，只建一次。字典內建於套件，建立不應失敗；若失敗，debug 建置以 `debug_assert!` 直接報錯
/// （測試與開發時立刻發現），release 建置退化為不轉換（只失去簡繁折疊，其餘比對照常）。
/// core 沒有日誌套件，所以 release 的退化是無聲的。
pub(super) static T2S: LazyLock<Option<OpenCC>> = LazyLock::new(|| {
    let converter = OpenCC::from_config(BuiltinConfig::T2s);
    debug_assert!(
        converter.is_ok(),
        "ferrous-opencc 的內建 T2s 設定應該一定建得起來：{:?}",
        converter.as_ref().err()
    );
    converter.ok()
});

/// 漢字（統一表意文字、擴充區與相容區）。
pub(super) fn is_han(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{3FFFF}')
}

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

    /// 兩邊曲長差幾秒（絕對值）。任一邊缺少、不大於 0 或是 NaN 都視為「未知」，回 `None`。
    pub fn duration_gap(&self, other: Option<f64>) -> Option<f64> {
        let (duration, other) = (self.duration?, other?);
        // NaN 的比較一律為 false，因此也落入「未知」。
        (duration > 0.0 && other > 0.0).then(|| (duration - other).abs())
    }

    /// 容忍而非相等：一首歌的另一個版本常與原版差幾秒。曲長未知（見 [`Self::duration_gap`]）
    /// 算吻合。差太多時為 `None`。
    pub fn duration_fit(&self, other: Option<f64>) -> Option<DurationFit> {
        let Some(gap) = self.duration_gap(other) else {
            return Some(DurationFit::Close);
        };
        if gap <= DURATION_TOLERANCE {
            Some(DurationFit::Close)
        } else if gap <= LOOSE_DURATION_TOLERANCE {
            Some(DurationFit::Loose)
        } else {
            None
        }
    }

    /// 曲長是否吻合（5 秒以內或未知）。
    pub fn duration_matches(&self, other: Option<f64>) -> bool {
        self.duration_fit(other) == Some(DurationFit::Close)
    }

    /// 簡體中文的查詢形式：歌名與演出者轉成簡體，中英並列的演出者名（`周杰倫 Jay Chou`）只留含漢字的部分。
    /// 有些歌庫（LRCLIB）的搜尋不做繁簡轉換，帶著英文名也找不到中文紀錄。
    /// 與 [`Self::normalized`] 相同時為 `None`（不必再查一次）。
    pub fn simplified_chinese(&self) -> Option<TrackIdentity> {
        let converter = T2S.as_ref()?;
        let n = self.normalized();
        let chinese: Vec<&str> = n
            .artist
            .split_whitespace()
            .filter(|word| word.chars().any(is_han))
            .collect();
        let artist = if chinese.is_empty() {
            n.artist.clone()
        } else {
            chinese.join(" ")
        };
        let simplified = TrackIdentity {
            title: converter.convert(&n.title),
            artist: converter.convert(&artist),
            ..n.clone()
        };
        (simplified != n).then_some(simplified)
    }

    /// 紀錄列出的演出者（一首歌可以有好幾位）是否和查詢的演出者寬鬆相符；任一位相符即可。
    ///
    /// 查詢端用正規化後的演出者（和送給來源、當快取鍵的相同：多位時取第一位）。兩邊和歌名一樣摺疊
    /// （簡繁、全半形、大小寫），去掉空白與標點，再比三種部分：整個名字、其中的漢字、漢字以外的字；
    /// 不足兩個字的部分不比，任一對部分一方包含另一方就算相符。
    /// 紀錄沒有任何帶文字的名字（空清單、空字串）時沒有資訊可判斷，算相符。
    pub fn matches_artist<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> bool {
        let wanted = artist_parts(&artist_text(&self.normalized().artist));
        let mut informed = false;
        for name in names {
            let name = artist_text(name);
            if name.is_empty() {
                continue;
            }
            informed = true;
            let found = artist_parts(&name).iter().any(|theirs| {
                wanted
                    .iter()
                    .any(|ours| ours.contains(theirs.as_str()) || theirs.contains(ours.as_str()))
            });
            if found {
                return true;
            }
        }
        !informed
    }

    /// 不要用 `==` 比對提供者回來的歌名：同一首歌可能以簡體、不同大小寫或全形字元回來。
    /// 兩邊先折成同一種文字再比。
    pub fn loosely_matches(a: &str, b: &str) -> bool {
        fold(a) == fold(b)
    }
}

/// [`TrackIdentity::loosely_matches`] 的比對形式；歌詞版本合併也用它比對行文字。
pub(super) fn fold(raw: &str) -> String {
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

/// 演出者名的比對形式：同歌名的摺疊，再只留文字與數字。摺疊結果是分解形式，重新組合（NFC）
/// 讓帶濁點的假名、韓文音節各算一個字。
fn artist_text(raw: &str) -> String {
    fold(raw).nfc().filter(|c| c.is_alphanumeric()).collect()
}

/// [`artist_text`] 結果裡拿來比的部分：整個名字、漢字、漢字以外；不足兩個字的丟掉。
fn artist_parts(text: &str) -> Vec<String> {
    let (han, other): (String, String) = text.chars().partition(|c| is_han(*c));
    [text.to_string(), han, other]
        .into_iter()
        .filter(|part| part.chars().count() >= 2)
        .collect()
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

    #[test]
    fn duration_fit_is_close_within_five_seconds_and_loose_within_ten() {
        let track = make("T", "A", Some(200.0));
        for close in [Some(200.0), Some(205.0), Some(195.0), None, Some(0.0)] {
            assert_eq!(
                track.duration_fit(close),
                Some(DurationFit::Close),
                "{close:?}"
            );
        }
        for loose in [205.1, 210.0, 194.9, 190.0] {
            assert_eq!(
                track.duration_fit(Some(loose)),
                Some(DurationFit::Loose),
                "{loose}"
            );
        }
        for far in [210.1, 189.9, 400.0] {
            assert_eq!(track.duration_fit(Some(far)), None, "{far}");
        }
        assert_eq!(
            make("T", "A", None).duration_fit(Some(400.0)),
            Some(DurationFit::Close)
        );
    }

    #[test]
    fn duration_gap_is_the_absolute_difference_or_unknown() {
        let track = make("T", "A", Some(200.0));
        assert_eq!(track.duration_gap(Some(197.0)), Some(3.0));
        assert_eq!(track.duration_gap(Some(450.0)), Some(250.0));
        assert_eq!(track.duration_gap(Some(200.0)), Some(0.0));
        for unknown in [None, Some(0.0), Some(-1.0), Some(f64::NAN)] {
            assert_eq!(track.duration_gap(unknown), None, "{unknown:?}");
            assert_eq!(
                make("T", "A", unknown).duration_gap(Some(200.0)),
                None,
                "{unknown:?}"
            );
        }
    }

    #[test]
    fn simplified_chinese_converts_the_text_and_keeps_only_the_chinese_artist_name() {
        let query = make("爸 我回來了", "周杰倫 Jay Chou", Some(229.6));
        assert_eq!(
            query.simplified_chinese(),
            Some(make("爸 我回来了", "周杰伦", Some(229.6)))
        );
        for (artist, expected) in [
            ("周杰伦 Jay Chou", "周杰伦"),
            ("周杰倫 (Jay Chou)", "周杰伦"),
            ("G.E.M. 鄧紫棋", "邓紫棋"),
            ("劉若英", "刘若英"),
        ] {
            let got = make("T", artist, None).simplified_chinese();
            assert_eq!(got.map(|t| t.artist).as_deref(), Some(expected), "{artist}");
        }
    }

    /// 轉了跟原本一樣就不必再查一次。
    #[test]
    fn simplified_chinese_is_none_when_nothing_would_change() {
        assert_eq!(make("晴天", "周杰伦", None).simplified_chinese(), None);
        assert_eq!(make("Song", "Queen", None).simplified_chinese(), None);
        // 只差正規化會拿掉的部分（後綴、次要演出者）也算沒變。
        assert_eq!(
            make("Song (Live)", "A & B", None).simplified_chinese(),
            None
        );
    }

    // ---- 演出者比對（Rust 版新增）

    fn artist_matches(query: &str, record: &str) -> bool {
        make("T", query, None).matches_artist([record])
    }

    #[test]
    fn artist_matching_compares_the_whole_name_and_its_han_and_non_han_parts() {
        let query = "周杰倫 Jay Chou";
        for record in [
            "周杰伦",
            "Jay Chou",
            "周杰倫 (Jay Chou)",
            "ＪＡＹ　ＣＨＯＵ",
            "jay-chou",
            "周 杰 倫",
        ] {
            assert!(artist_matches(query, record), "{record}");
        }
        for record in ["恶童ETON", "Jay Park", "陈奕迅"] {
            assert!(!artist_matches(query, record), "{record}");
        }
    }

    #[test]
    fn artist_matching_is_containment_either_way() {
        assert!(artist_matches("周杰倫", "周杰倫 Jay Chou"));
        assert!(artist_matches("Jay Chou", "周杰伦 (Jay Chou)"));
        assert!(artist_matches("周杰倫 Official Channel", "周杰伦 Jay"));
        assert!(artist_matches("鄧紫棋 G.E.M.", "GEM"));
        assert!(artist_matches("宇多田ヒカル", "宇多田ヒカル"));
    }

    /// 單一字母或單一漢字的部分幾乎什麼都包含得到，所以不拿來比。
    #[test]
    fn artist_parts_shorter_than_two_characters_are_ignored() {
        // 非漢字部分只剩「x」：不能因此配上任何含 x 的名字。
        assert!(!artist_matches("周杰倫 X", "X Japan"));
        assert!(!artist_matches("周杰倫", "周"));
        assert!(!artist_matches("A", "Abba"));
    }

    #[test]
    fn a_record_without_artist_information_counts_as_matching() {
        let query = make("T", "周杰倫", None);
        assert!(query.matches_artist([]));
        for blank in ["", "  ", " - / ", "　"] {
            assert!(query.matches_artist([blank]), "{blank:?}");
        }
        // 空白名字不是資訊：和不相符的名字並列時，仍以有內容的名字判斷。
        assert!(!query.matches_artist(["", "恶童ETON"]));
    }

    #[test]
    fn any_listed_artist_may_match() {
        let query = make("T", "周杰倫", None);
        assert!(query.matches_artist(["费玉清", "周杰伦"]));
        assert!(!query.matches_artist(["费玉清", "陈奕迅"]));
    }

    /// 和歌名一樣用正規化後的查詢：多位演出者以第一位為準，寫法不同的合作名單也認得。
    #[test]
    fn artist_matching_uses_the_primary_query_artist() {
        let query = "Taylor Swift, Ed Sheeran";
        assert!(artist_matches(query, "Taylor Swift feat. Ed Sheeran"));
        assert!(artist_matches(query, "Taylor Swift"));
        assert!(!artist_matches(query, "Ed Sheeran"));
    }
}
