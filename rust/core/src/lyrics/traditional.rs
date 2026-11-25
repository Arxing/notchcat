//! 歌詞簡轉繁（顯示用）：OpenCC `s2tw`（簡體 → 台灣繁體字形；**不換用語**，「软件」是「軟件」不是「軟體」）。
//!
//! 只有**整份歌詞看起來是簡體中文**時才轉：
//!
//! - **日文歌詞整份不動。** 假名佔（假名 + 漢字）十分之一以上就視為日文；日文的新字體漢字（国、学、会…）
//!   與簡體字形相同，照轉會把原文改壞。
//! - **本來就是繁體的歌詞整份不動。** 對繁體文字再做一次簡轉繁會出錯（「干擾」變「幹擾」、「鄰里」變「鄰裡」）。
//!   判斷方式：數出「只有繁體才用」的字（繁轉簡會改變它）與「簡體寫法」的字（繁轉簡不變、簡轉繁會改變它；
//!   「里、干、后、台」這類兩邊都用的字也算在這邊），後者**多於**前者才轉。簡繁各半的歌詞不轉。
//!
//! 已知限制：一簡對多繁的字靠 OpenCC 的詞庫判斷，詞庫沒收的詞可能選錯（例：「头发干了」會變成「頭髮幹了」）。

use std::collections::HashMap;
use std::sync::LazyLock;

use ferrous_opencc::config::BuiltinConfig;
use ferrous_opencc::OpenCC;

use super::identity::T2S;
use super::{LyricLine, Lyrics};

/// 簡 → 台灣繁體，只建一次（同 [`T2S`]：建立失敗時 debug 建置報錯、release 退化為不轉換）。
static S2TW: LazyLock<Option<OpenCC>> = LazyLock::new(|| {
    let converter = OpenCC::from_config(BuiltinConfig::S2tw);
    debug_assert!(
        converter.is_ok(),
        "ferrous-opencc 的內建 S2tw 設定應該一定建得起來：{:?}",
        converter.as_ref().err()
    );
    converter.ok()
});

/// 要顯示的繁體版歌詞。不需要轉（日文、本來就是繁體、沒有中文）時原樣回傳。時間軸不變。
pub fn to_traditional(lyrics: &Lyrics) -> Lyrics {
    let (Some(s2tw), Some(t2s)) = (S2TW.as_ref(), T2S.as_ref()) else {
        return lyrics.clone();
    };
    let simplified = match lyrics {
        Lyrics::Synced(lines) => {
            is_simplified_chinese(lines.iter().map(|l| l.text.as_str()), t2s, s2tw)
        }
        Lyrics::Plain(text) => is_simplified_chinese([text.as_str()], t2s, s2tw),
    };
    if !simplified {
        return lyrics.clone();
    }
    match lyrics {
        Lyrics::Synced(lines) => Lyrics::Synced(
            lines
                .iter()
                .map(|l| LyricLine {
                    time: l.time,
                    text: s2tw.convert(&l.text),
                })
                .collect(),
        ),
        Lyrics::Plain(text) => Lyrics::Plain(s2tw.convert(text)),
    }
}

/// 見模組說明的兩條規則。
fn is_simplified_chinese<'a>(
    texts: impl IntoIterator<Item = &'a str>,
    t2s: &OpenCC,
    s2tw: &OpenCC,
) -> bool {
    let mut han: HashMap<char, usize> = HashMap::new();
    let (mut han_total, mut kana_total) = (0usize, 0usize);
    for c in texts.into_iter().flat_map(str::chars) {
        if is_han(c) {
            *han.entry(c).or_default() += 1;
            han_total += 1;
        } else if is_kana(c) {
            kana_total += 1;
        }
    }
    if han_total == 0 || kana_total * 10 >= han_total + kana_total {
        return false;
    }
    let (mut simplified, mut traditional) = (0usize, 0usize);
    let mut buffer = [0u8; 4];
    for (c, count) in han {
        let one: &str = c.encode_utf8(&mut buffer);
        if t2s.convert(one) != one {
            traditional += count;
        } else if s2tw.convert(one) != one {
            simplified += count;
        }
    }
    simplified > traditional
}

/// 中日韓統一表意文字（基本區與擴充 A）。
fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}')
}

/// 平假名與片假名（含半形片假名）。不含中點「・」（U+30FB）：中文歌詞的人名常用它。
fn is_kana(c: char) -> bool {
    matches!(c,
        '\u{3041}'..='\u{309F}'
        | '\u{30A1}'..='\u{30FA}'
        | '\u{30FC}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{FF66}'..='\u{FF9D}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(texts: &[&str]) -> Lyrics {
        Lyrics::Synced(
            texts
                .iter()
                .enumerate()
                .map(|(i, t)| LyricLine {
                    time: i as f64 * 2.5,
                    text: (*t).into(),
                })
                .collect(),
        )
    }

    fn unchanged(texts: &[&str]) {
        assert_eq!(to_traditional(&synced(texts)), synced(texts), "{texts:?}");
    }

    #[test]
    fn simplified_synced_lyrics_become_traditional_and_keep_their_times() {
        assert_eq!(
            to_traditional(&synced(&[
                "麻烦你的头 管它天亮天黑",
                "我都没喝咖啡 夏天一直喝矿泉水",
                "心里的面条只要一干二净",
                "复杂的关系里重复发生，皇后在后面",
                "",
            ])),
            synced(&[
                "麻煩你的頭 管它天亮天黑",
                "我都沒喝咖啡 夏天一直喝礦泉水",
                "心裡的麵條只要一乾二淨",
                "複雜的關係裡重複發生，皇后在後面",
                "",
            ])
        );
    }

    #[test]
    fn plain_lyrics_are_converted_too() {
        assert_eq!(
            to_traditional(&Lyrics::Plain("我们的时间\n过得很快".into())),
            Lyrics::Plain("我們的時間\n過得很快".into())
        );
    }

    #[test]
    fn vocabulary_is_not_replaced() {
        assert_eq!(
            to_traditional(&synced(&["这个软件让我们着急"])),
            synced(&["這個軟件讓我們著急"])
        );
    }

    #[test]
    fn traditional_lyrics_are_left_alone() {
        // 逐句再轉一次會壞掉的繁體詞。
        unchanged(&[
            "千里之外 你無聲黑白",
            "干擾我們的鄰里",
            "皇后在後面，頭髮乾了",
            "台灣的么女 於是說了什麼",
        ]);
    }

    #[test]
    fn japanese_lyrics_are_left_alone() {
        unchanged(&["君の名は希望", "学校の国で会いましょう", "夢を見ていた"]);
        // 只有少數行是純漢字也一樣：整份判斷。
        unchanged(&["東京", "国会", "さよならの時間"]);
    }

    #[test]
    fn a_stray_kana_does_not_block_a_chinese_song() {
        assert_eq!(
            to_traditional(&synced(&[
                "你の名字还记得吗",
                "我们的时间过得很快，这些年发生了许多事情"
            ])),
            synced(&[
                "你の名字還記得嗎",
                "我們的時間過得很快，這些年發生了許多事情"
            ])
        );
    }

    #[test]
    fn the_middle_dot_in_names_is_not_kana() {
        assert_eq!(
            to_traditional(&synced(&["迈克・杰克逊・乔丹・"])),
            synced(&["邁克・傑克遜・喬丹・"])
        );
    }

    #[test]
    fn lyrics_without_chinese_are_left_alone() {
        unchanged(&["Hello darkness my old friend", "안녕하세요", "♪", ""]);
        unchanged(&[]);
    }
}
