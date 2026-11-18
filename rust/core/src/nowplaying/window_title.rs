//! 從已知播放器的視窗標題讀出曲目（移植自 Swift `WindowTitleParser.swift`）。純函式，不呼叫系統。

/// 視窗標題後備來源所見的視窗：誰擁有它、標題列寫什麼。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowInfo {
    pub owner: String,
    pub title: String,
}

/// 從視窗標題還原出的曲目。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParsedTrack {
    pub title: String,
    pub is_playing: bool,
}

/// 只認得這些擁有者的視窗。
const KNOWN_OWNERS: [&str; 1] = ["YouTube Music"];
const PREFIX: &str = "YouTube Music - ";
const SUFFIX: &str = " | YouTube Music";
/// 播放中時附在標題尾端的喇叭符號。
const SPEAKER: char = '🔊';

/// 讀出已知播放器視窗標題裡的曲目。
///
/// 只認得 `YouTube Music - <title> | YouTube Music`，播放中時尾端多一個喇叭符號。
/// 其他格式一律回傳 `None` 而不是半截字串：這個格式屬於第三方，隨時可能改變。
///
/// 全程以字元邊界操作（`strip_prefix`／`strip_suffix`），不做位元組切片，任何 Unicode 輸入都不會 panic。
pub fn parse_window_title(w: &WindowInfo) -> Option<ParsedTrack> {
    if !KNOWN_OWNERS.contains(&w.owner.as_str()) {
        return None;
    }

    let mut text = w.title.trim();
    let is_playing = match text.strip_suffix(SPEAKER) {
        Some(rest) => {
            text = rest.trim();
            true
        }
        None => false,
    };

    // 前後綴重疊（如 "YouTube Music - | YouTube Music"）時，剝掉前綴後剩下的字串不夠長，剝後綴會失敗。
    let title = text.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?.trim();
    if title.is_empty() {
        return None;
    }
    Some(ParsedTrack {
        title: title.to_string(),
        is_playing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str) -> WindowInfo {
        window_owned_by(title, "YouTube Music")
    }

    fn window_owned_by(title: &str, owner: &str) -> WindowInfo {
        WindowInfo {
            owner: owner.to_string(),
            title: title.to_string(),
        }
    }

    fn parsed(title: &str, is_playing: bool) -> Option<ParsedTrack> {
        Some(ParsedTrack {
            title: title.to_string(),
            is_playing,
        })
    }

    #[test]
    fn parses_playing_title() {
        assert_eq!(
            parse_window_title(&window("YouTube Music - 假日而已 | YouTube Music 🔊")),
            parsed("假日而已", true)
        );
    }

    #[test]
    fn parses_paused_title_without_speaker() {
        assert_eq!(
            parse_window_title(&window("YouTube Music - 假日而已 | YouTube Music")),
            parsed("假日而已", false)
        );
    }

    #[test]
    fn strips_extra_whitespace() {
        assert_eq!(
            parse_window_title(&window(
                "  YouTube Music -   假日而已  | YouTube Music  🔊  "
            )),
            parsed("假日而已", true)
        );
    }

    #[test]
    fn unknown_owner_returns_nil() {
        assert_eq!(
            parse_window_title(&window_owned_by(
                "YouTube Music - 假日而已 | YouTube Music 🔊",
                "Safari"
            )),
            None
        );
    }

    #[test]
    fn empty_title_returns_nil() {
        assert_eq!(parse_window_title(&window("")), None);
        assert_eq!(parse_window_title(&window("   ")), None);
        assert_eq!(
            parse_window_title(&window("YouTube Music -  | YouTube Music")),
            None
        );
        assert_eq!(
            parse_window_title(&window("YouTube Music - | YouTube Music 🔊")),
            None
        );
    }

    #[test]
    fn unknown_format_returns_nil() {
        assert_eq!(parse_window_title(&window("YouTube Music")), None);
        assert_eq!(
            parse_window_title(&window("YouTube Music - 假日而已")),
            None
        );
        assert_eq!(
            parse_window_title(&window("假日而已 | YouTube Music")),
            None
        );
        assert_eq!(parse_window_title(&window("Home - YouTube Music")), None);
        assert_eq!(
            parse_window_title(&window("YouTube Music - 假日而已 | Spotify")),
            None
        );
    }

    #[test]
    fn title_containing_separators() {
        assert_eq!(
            parse_window_title(&window("YouTube Music - A - B | YouTube Music 🔊")),
            parsed("A - B", true)
        );
        assert_eq!(
            parse_window_title(&window("YouTube Music - A | B | YouTube Music")),
            parsed("A | B", false)
        );
    }

    /// 以字元（而非位元組）運作：emoji、組合符號、全形空白與只有喇叭的標題都不得 panic，也不得切壞字元。
    #[test]
    fn multibyte_and_combining_titles_never_panic() {
        assert_eq!(
            parse_window_title(&window("YouTube Music - 🎵e\u{301}🎶 | YouTube Music 🔊")),
            parsed("🎵e\u{301}🎶", true)
        );
        assert_eq!(
            parse_window_title(&window(
                "\u{3000}YouTube Music - 歌 | YouTube Music\u{3000}"
            )),
            parsed("歌", false)
        );
        for t in [
            "🔊",
            "🔊🔊",
            "\u{301}",
            "YouTube Music - 🔊 | YouTube Music",
            "YouTube Music - \u{301}",
            "YouTube Music - | YouTube Music",
            "é",
        ] {
            let _ = parse_window_title(&window(t));
        }
    }
}
