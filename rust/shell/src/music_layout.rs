//! 音樂畫面的版面常數與純運算（與 GPUI 無關，可單元測試）。
//!
//! 常數照抄 Swift 版（`Sources/NotchCatKit/Modules/Music/Views/*.swift`、`UI/NotchContentView.swift`、
//! `UI/ModuleAppBar.swift`），每個常數的註解寫出處；Swift 沒有、本版自訂的值另外標明「本版」。
//! 長度單位都是點（GPUI 的 `px` 即點，Retina 由縮放係數處理）。

use musicat_core::lyrics::Lyrics;
use musicat_core::music::presentation::playback_position;
use musicat_core::shell_logic::geometry::{ease_in_out, EXPANDED_HEIGHT};
use musicat_core::types::TrackSnapshot;

use crate::EXPANDED_TOP_PADDING;

// ---------------------------------------------------------------- Preview（MusicPreviewView.swift）

/// 左翼外緣到封面的距離（`coverInset`）。
pub const PREVIEW_COVER_INSET: f64 = 8.0;
/// 右翼外緣到音柱的距離（`barsInset`）。
pub const PREVIEW_BARS_INSET: f64 = 12.0;
/// 翼列裡封面上下各留的空白（`coverVerticalInset`）。
pub const PREVIEW_COVER_VERTICAL_INSET: f64 = 4.0;

// ---------------------------------------------------------------- 封面（CoverView.swift）

/// 封面圓角（`RoundedRectangle(cornerRadius: 6)`）。
pub const COVER_CORNER_RADIUS: f64 = 6.0;
/// 沒有封面時佔位底色：白色不透明度（`Color.white.opacity(0.12)`）。
pub const COVER_PLACEHOLDER_OPACITY: f64 = 0.12;
/// 佔位音符字級 = 邊長 × 0.45（`proxy.size.width * 0.45`）。
pub const COVER_PLACEHOLDER_GLYPH_SCALE: f64 = 0.45;
/// 佔位音符的白色不透明度（`Color.white.opacity(0.6)`）。
pub const COVER_PLACEHOLDER_GLYPH_OPACITY: f64 = 0.6;
/// 佔位與模組切換列的音符字形（Swift 為 SF Symbol `music.note`；GPUI 沒有 SF Symbols，本版用文字 U+266A）。
pub const MUSIC_NOTE_GLYPH: &str = "\u{266A}";

// ---------------------------------------------------------------- 音柱（EqualizerBars.swift）

/// 音柱寬（`barWidth`）。
pub const EQ_BAR_WIDTH: f64 = 3.0;
/// 音柱間距（`barSpacing`）。
pub const EQ_BAR_SPACING: f64 = 2.0;
/// 音柱區高（`height`）。
pub const EQ_HEIGHT: f64 = 14.0;
/// 各柱一次「升或降」的秒數（`durations`；`repeatForever(autoreverses: true)`，一個來回是兩倍）。
pub const EQ_DURATIONS: [f64; 4] = [0.45, 0.62, 0.38, 0.54];
/// 各柱的最高比例（`peaks`）。
pub const EQ_PEAKS: [f64; 4] = [1.0, 0.75, 0.9, 0.6];
/// 靜止時的比例（`restingScale`）。
pub const EQ_RESTING_SCALE: f64 = 0.25;
/// 音柱的白色不透明度（`Color.white.opacity(0.9)`）。
pub const EQ_BAR_OPACITY: f64 = 0.9;
/// 本版：播放中音柱的重繪頻率（Swift 由 Core Animation 以顯示器頻率畫）。12 fps 足以表現 0.38–0.62 秒的起伏。
/// 實測（release、螢幕醒著、Preview 播放中 15 秒）15 fps 約 4.5% CPU，超過 HANDOFF 第 13 項的 3% 目標，所以降到 12。
pub const EQ_FRAME_SECONDS: f64 = 1.0 / 12.0;
/// 本版：跑馬燈捲動時的重繪頻率。以顯示器頻率（120 Hz）重繪實測約 11–18% CPU；30 fps 時每格前進 1 點。
pub const MARQUEE_FRAME_SECONDS: f64 = 1.0 / 30.0;

// ---------------------------------------------------------------- 歌詞條（LyricStripView.swift）

/// 歌詞條下方圓角（`bottomCornerRadius`，與外殼 Preview 圓角相同）。
pub const STRIP_CORNER_RADIUS: f64 = 10.0;
/// 歌詞條左右內距（`horizontalPadding`）。
pub const STRIP_HORIZONTAL_PADDING: f64 = 12.0;
/// 歌詞條字級（`.system(size: 12, weight: .medium)`）。
pub const STRIP_FONT_SIZE: f64 = 12.0;
/// 顯示歌詞行時的文字不透明度（`isLine ? 0.95 : 0.7`）。
pub const STRIP_LINE_OPACITY: f64 = 0.95;
/// 顯示曲名時的文字不透明度。
pub const STRIP_TITLE_OPACITY: f64 = 0.7;
/// 多行歌詞條裡，前後文行（非目前行）的文字不透明度。
pub const STRIP_CONTEXT_OPACITY: f64 = 0.45;
/// 歌詞條重讀目前行的週期（`TimelineView(.periodic(by: 0.25))`）。
pub const STRIP_TICK_SECONDS: f64 = 0.25;

// ---------------------------------------------------------------- Expanded 外框（NotchContentView／ModuleAppBar）

/// 外殼給 Expanded 內容的高度：180 − 上方 8（`.padding(.top, 8)`）= 172。
pub const EXPANDED_CONTENT_HEIGHT: f64 = EXPANDED_HEIGHT - EXPANDED_TOP_PADDING;
/// 模組切換列高：Swift 為 `Image(systemName:)` 的自然高度，實測約 15 點（HANDOFF 第 14 項；
/// `MusicExpandedView` 註解寫的約 20 點是舊估計）。本版固定為 15，讓版面預算是確定的。
pub const APP_BAR_HEIGHT: f64 = 15.0;
/// 切換列與下方內容的間距（`VStack(spacing: 8)`）。
pub const APP_BAR_SPACING: f64 = 8.0;
/// 切換列圖示之間的間距（`HStack(spacing: 12)`）。
pub const APP_BAR_ICON_SPACING: f64 = 12.0;
/// 切換列圖示字級（SF Symbol 的預設字級 `.body` = 13 點）。
pub const APP_BAR_ICON_SIZE: f64 = 13.0;
/// 未選取圖示的不透明度（`opacity(isSelected ? 1 : 0.4)`）。
pub const APP_BAR_UNSELECTED_OPACITY: f64 = 0.4;
/// 沒有選取模組時的說明（`DefaultShellContent`）。
pub const DEFAULT_EXPANDED_TEXT: &str = "沒有進行中的內容，點上方圖示開啟模組";
/// 說明文字字級（`.system(size: 12, weight: .medium)`）與不透明度（`.white.opacity(0.7)`）。
pub const DEFAULT_EXPANDED_FONT_SIZE: f64 = 12.0;
pub const DEFAULT_EXPANDED_OPACITY: f64 = 0.7;
/// 音樂頁可用的高度：172 − 15 − 8 = 149（HANDOFF 第 14 項）。
pub const MUSIC_PAGE_HEIGHT: f64 = EXPANDED_CONTENT_HEIGHT - APP_BAR_HEIGHT - APP_BAR_SPACING;

// ---------------------------------------------------------------- 音樂頁（MusicExpandedView.swift）

/// 左右內距（`.padding(.horizontal, 16)`）。
pub const PAGE_HORIZONTAL_PADDING: f64 = 16.0;
/// 下方內距（`.padding(.bottom, 8)`）。
pub const PAGE_BOTTOM_PADDING: f64 = 8.0;
/// 封面邊長（`coverSide`）。
pub const EXPANDED_COVER_SIDE: f64 = 68.0;
/// 封面與資訊欄的間距（`HStack(spacing: 12)`）。
pub const HEADER_SPACING: f64 = 12.0;
/// 標頭與歌詞的間距（`VStack(spacing: 6)`）。
pub const SECTION_SPACING: f64 = 6.0;
/// 歌詞區可用高度：149 − 8 − 68 − 6 = 67。
pub const LYRICS_VIEWPORT_HEIGHT: f64 =
    MUSIC_PAGE_HEIGHT - PAGE_BOTTOM_PADDING - EXPANDED_COVER_SIDE - SECTION_SPACING;
/// 曲名：13 點 semibold、白色，行高 16（Swift 註解「Title (16pt line)」）。
pub const TITLE_FONT_SIZE: f64 = 13.0;
pub const TITLE_LINE_HEIGHT: f64 = 16.0;
/// 演出者：11 點、白色 0.65，行高 14（「artist (14pt line)」）。
pub const ARTIST_FONT_SIZE: f64 = 11.0;
pub const ARTIST_LINE_HEIGHT: f64 = 14.0;
pub const ARTIST_OPACITY: f64 = 0.65;
/// 曲名區與進度條之間的最小空白（`Spacer(minLength: 2)`）。
pub const INFO_MIN_SPACER: f64 = 2.0;
/// 進度條高（`.frame(height: 3)`）、與控制列的間距（`.padding(.bottom, 4)`）。
pub const PROGRESS_HEIGHT: f64 = 3.0;
pub const PROGRESS_BOTTOM_PADDING: f64 = 4.0;
/// 進度條底軌與填色的白色不透明度（`opacity(0.2)`、`opacity(0.85)`）。
pub const PROGRESS_TRACK_OPACITY: f64 = 0.2;
pub const PROGRESS_FILL_OPACITY: f64 = 0.85;
/// 本版：Expanded 播放中重讀進度與目前歌詞行的週期（Swift 進度條 1 秒、歌詞 0.25 秒；本版合併為 0.25 秒）。
pub const EXPANDED_TICK_SECONDS: f64 = 0.25;
/// 控制列高（`controlRowHeight`）；左側等寬的空白與右側的折疊開關都是這個邊長。
pub const CONTROL_ROW_HEIGHT: f64 = 24.0;
/// 控制按鈕寬（`.frame(width: 28)`）與按鈕間距（`HStack(spacing: 14)`）。
pub const CONTROL_BUTTON_WIDTH: f64 = 28.0;
pub const CONTROL_BUTTON_SPACING: f64 = 14.0;
/// 上一首／下一首字形大小（`size: 13`）、播放／暫停（`size: 16`）。
pub const SKIP_GLYPH_SIZE: f64 = 13.0;
pub const PLAY_GLYPH_SIZE: f64 = 16.0;
/// 按鈕可用／停用時的白色不透明度（`enabled ? 0.95 : 0.3`）。
pub const CONTROL_ENABLED_OPACITY: f64 = 0.95;
pub const CONTROL_DISABLED_OPACITY: f64 = 0.3;
/// 折疊開關的箭頭大小（`size: 10, weight: .semibold`）與不透明度（`opacity(0.6)`）。
pub const FOLD_GLYPH_SIZE: f64 = 10.0;
pub const FOLD_OPACITY: f64 = 0.6;
/// 本版：歌詞來源標籤（控制列左側空位，Swift 沒有）。字級 9、行高 12（不超過控制列高 24）、白色 0.45（低對比）。
pub const SOURCE_LABEL_FONT_SIZE: f64 = 9.0;
pub const SOURCE_LABEL_LINE_HEIGHT: f64 = 12.0;
pub const SOURCE_LABEL_OPACITY: f64 = 0.45;
/// 本版：標籤裡主機名稱最多幾個字元（以字元計，超過以 `…` 截斷；畫面上仍會再依可用寬度裁切）。
pub const SOURCE_LABEL_MAX_CHARS: usize = 24;
/// 本版：標籤結尾的「外部連結」箭頭（含前面的空白）。
pub const SOURCE_LABEL_SUFFIX: &str = " \u{2197}";
/// 沒有曲目時的一行字（`Text("目前沒有播放")`，13 點、白色 0.6）。
pub const NO_TRACK_TEXT: &str = "目前沒有播放";
pub const NO_TRACK_FONT_SIZE: f64 = 13.0;
pub const NO_TRACK_OPACITY: f64 = 0.6;
/// 歌詞字級（`LyricStyle.font` 11 點；目前行 semibold）。
pub const LYRIC_FONT_SIZE: f64 = 11.0;
/// 本版：每行歌詞固定的行高（Swift 由文字自然高度決定，11 點系統字約 13 點）。固定行高讓捲動位置可以純運算。
pub const LYRIC_LINE_HEIGHT: f64 = 14.0;
/// 行距（`VStack(spacing: 3)`）。
pub const LYRIC_LINE_SPACING: f64 = 3.0;
/// 目前行與其他行的不透明度（`isCurrent ? 1 : 0.45`）；純文字歌詞 0.75。
pub const LYRIC_CURRENT_OPACITY: f64 = 1.0;
pub const LYRIC_OTHER_OPACITY: f64 = 0.45;
pub const LYRIC_PLAIN_OPACITY: f64 = 0.75;
/// 換行時捲動到目前行的動畫長度（`.easeInOut(duration: 0.3)`）。
pub const LYRIC_SCROLL_SECONDS: f64 = 0.3;

/// 一翼的寬：`(外形寬 − 瀏海寬) / 2`，不小於 0（Swift 的 `frame(maxWidth: .infinity)` 兩側平分）。
pub fn wing_width(total_width: f64, notch_width: f64) -> f64 {
    let w = (total_width - notch_width) / 2.0;
    if w.is_finite() && w > 0.0 {
        w
    } else {
        0.0
    }
}

/// Preview 封面邊長：`max(0, min(列高 − 2 × 4, 翼寬 − 8))`（MusicPreviewView 的 `side`）。
pub fn preview_cover_side(wing_width: f64, row_height: f64) -> f64 {
    // `f64::min` 遇到 NaN 會回傳另一邊，所以先擋掉非有限的輸入。
    if !(wing_width.is_finite() && row_height.is_finite()) {
        return 0.0;
    }
    let side =
        (row_height - 2.0 * PREVIEW_COVER_VERTICAL_INSET).min(wing_width - PREVIEW_COVER_INSET);
    if side.is_finite() && side > 0.0 {
        side
    } else {
        0.0
    }
}

/// 第 `index` 根音柱在 `t` 秒時的高度比例。播放中以 ease-in-out 在靜止比例與峰值之間來回
/// （一次升或降 `EQ_DURATIONS[index]` 秒，同 Swift 的 `repeatForever(autoreverses: true)`）；
/// 沒在播放時停在靜止比例（Swift 另有 0.2 秒的落下動畫，本版直接靜止）。非有限的 `t` 視為 0。
pub fn equalizer_scale(index: usize, t: f64, playing: bool) -> f64 {
    let (Some(&duration), Some(&peak)) = (EQ_DURATIONS.get(index), EQ_PEAKS.get(index)) else {
        return EQ_RESTING_SCALE;
    };
    if !playing {
        return EQ_RESTING_SCALE;
    }
    let t = if t.is_finite() && t > 0.0 { t } else { 0.0 };
    let phase = (t / duration) % 2.0;
    let tri = if phase <= 1.0 { phase } else { 2.0 - phase };
    EQ_RESTING_SCALE + (peak - EQ_RESTING_SCALE) * ease_in_out(tri)
}

/// 音柱區總寬：4 根柱加 3 個間距（`EqualizerBars.width`）= 18。
pub fn equalizer_width() -> f64 {
    let n = EQ_DURATIONS.len() as f64;
    n * EQ_BAR_WIDTH + (n - 1.0) * EQ_BAR_SPACING
}

/// 進度條填色的寬：比例夾到 `0..=1`（NaN 或沒有比例時為 0）乘上底軌寬（負的底軌寬當 0）。
pub fn progress_fill_width(fraction: Option<f64>, track_width: f64) -> f64 {
    let f = fraction
        .filter(|f| !f.is_nan())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let w = if track_width.is_finite() && track_width > 0.0 {
        track_width
    } else {
        0.0
    };
    f * w
}

/// 控制按鈕群的寬：3 顆按鈕加 2 個間距（= 112）。
pub fn control_buttons_width() -> f64 {
    3.0 * CONTROL_BUTTON_WIDTH + 2.0 * CONTROL_BUTTON_SPACING
}

/// 控制列左右兩側區域的寬（`(資訊欄寬 − 按鈕群寬) / 2`，兩側相等所以按鈕群置中）。
/// 左側是來源標籤的空位，右側是折疊開關所在的區域。非有限或不足時為 0。
pub fn control_side_widths(info_width: f64) -> (f64, f64) {
    let side = (info_width - control_buttons_width()) / 2.0;
    if side.is_finite() && side > 0.0 {
        (side, side)
    } else {
        (0.0, 0.0)
    }
}

/// 歌詞來源標籤的文字：網址的主機加上 ` ↗`（例如 `lrclib.net ↗`）。
///
/// 不用 `url` crate：取 `https://` 之後到第一個 `/`、`?`、`#` 之前的部分，去掉 `userinfo@` 與 `:埠號`
/// （方括號的 IPv6 主機保留原樣），再去掉開頭一次的 `www.`；不改大小寫。主機超過
/// [`SOURCE_LABEL_MAX_CHARS`] 個字元（以字元計，不以位元組）時保留前 `MAX − 1` 個字元加 `…`。
/// 不是 `https://` 開頭、沒有主機、主機含空白／控制字元／反斜線或落單的方括號 → `None`。任何輸入都不 panic。
pub fn source_label(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        let end = host_port.find(']')?;
        let (host, after) = host_port.split_at(end + 1);
        if !(after.is_empty() || after.starts_with(':')) || host.len() <= 2 {
            return None;
        }
        host
    } else {
        let host = host_port.split(':').next().unwrap_or("");
        if host.contains(['[', ']']) {
            return None;
        }
        host
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let mut label: String = if host.chars().count() > SOURCE_LABEL_MAX_CHARS {
        let mut cut: String = host.chars().take(SOURCE_LABEL_MAX_CHARS - 1).collect();
        cut.push('\u{2026}');
        cut
    } else {
        host.to_string()
    };
    label.push_str(SOURCE_LABEL_SUFFIX);
    Some(label)
}

/// Expanded 的歌詞區要怎麼畫（`ExpandedLyricsView`）。
#[derive(Clone, PartialEq, Debug)]
pub enum LyricsDisplay<'a> {
    /// 沒有歌詞：不畫歌詞區（標頭在空間中垂直置中）。
    None,
    /// 純文字：靜態全文（可用滾輪捲動）。
    Plain(&'a str),
    /// 同步歌詞但位置未知：靜態全文，沒有目前行。
    Static(&'a [musicat_core::lyrics::LyricLine]),
    /// 同步歌詞且位置已知：跟著播放捲動、標示目前行（尚未到第一行時為 `None`）。
    Following {
        lines: &'a [musicat_core::lyrics::LyricLine],
        current: Option<usize>,
    },
}

/// 依快照、歌詞與時間決定歌詞區（`ExpandedLyricsView` 的分支）。位置未知時絕不拿 0 充數。
pub fn lyrics_display<'a>(
    snapshot: &TrackSnapshot,
    lyrics: Option<&'a Lyrics>,
    now: f64,
) -> LyricsDisplay<'a> {
    match lyrics {
        None => LyricsDisplay::None,
        Some(Lyrics::Plain(text)) => LyricsDisplay::Plain(text),
        Some(l @ Lyrics::Synced(lines)) => match playback_position(snapshot, now) {
            Some(position) => LyricsDisplay::Following {
                lines,
                current: l.line_index(position),
            },
            None => LyricsDisplay::Static(lines),
        },
    }
}

/// 一行歌詞（含行距）佔的高度。
pub const LYRIC_PITCH: f64 = LYRIC_LINE_HEIGHT + LYRIC_LINE_SPACING;

/// 跟隨模式下，歌詞欄相對歌詞區頂端的位移（點，<= 0）：把目前行置中（Swift `scrollTo(anchor: .center)`），
/// 但不捲過內容的頭尾（同 ScrollView 的邊界）。沒有目前行（尚未到第一行）時停在頂端。
pub fn lyric_scroll_offset(current: Option<usize>, line_count: usize, viewport_height: f64) -> f64 {
    let Some(index) = current else {
        return 0.0;
    };
    if line_count == 0 || !(viewport_height.is_finite() && viewport_height > 0.0) {
        return 0.0;
    }
    let index = index.min(line_count - 1) as f64;
    let content = line_count as f64 * LYRIC_PITCH - LYRIC_LINE_SPACING;
    let centred = viewport_height / 2.0 - (index * LYRIC_PITCH + LYRIC_LINE_HEIGHT / 2.0);
    let min = (viewport_height - content).min(0.0);
    centred.clamp(min, 0.0)
}

/// 捲動動畫：`started` 之後 `elapsed` 秒時，從 `from` 到 `to` 的 ease-in-out 位置。
/// `elapsed` 非有限或超過 `duration` 時就是 `to`；`duration` 不大於 0 時直接是 `to`。
pub fn animated_offset(from: f64, to: f64, elapsed: f64, duration: f64) -> f64 {
    if duration.is_nan() || duration <= 0.0 || !elapsed.is_finite() || elapsed >= duration {
        return to;
    }
    from + (to - from) * ease_in_out(elapsed / duration)
}

/// 本版：重繪節拍。`None` = 不需要定時重繪（狀態改變時另有通知）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TickInputs {
    /// 面板實際可見（外殼的 `Visible` 事件）。
    pub visible: bool,
    /// 目前是 Expanded（否則 Preview）。
    pub expanded: bool,
    /// Preview：active 模組是音樂，且正在播放（音柱要動、歌詞條隨時間換行）。
    pub preview_playing: bool,
    /// Expanded：選取的是音樂頁、正在播放，且有隨時間變化的東西（進度條或跟隨中的歌詞）。
    pub expanded_live: bool,
    /// 目前模式中有跑馬燈正在捲動（文字放不下）。
    pub marquee_scrolling: bool,
}

/// 依可見性與模式決定定時重繪的週期（秒）。隱藏時一律不重繪。
pub fn tick_interval(i: TickInputs) -> Option<f64> {
    if !i.visible {
        return None;
    }
    let base = if i.expanded {
        i.expanded_live.then_some(EXPANDED_TICK_SECONDS)
    } else {
        // 音柱的 12 fps 已涵蓋歌詞條的 4 Hz。
        i.preview_playing.then_some(EQ_FRAME_SECONDS)
    };
    if i.marquee_scrolling {
        Some(base.map_or(MARQUEE_FRAME_SECONDS, |b| b.min(MARQUEE_FRAME_SECONDS)))
    } else {
        base
    }
}

/// 控制列與折疊開關的字形（取代 SF Symbols：`backward.fill`、`play.fill`、`pause.fill`、`forward.fill`、
/// `chevron.up`、`chevron.down`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
    ChevronUp,
    ChevronDown,
}

/// 字形的一段：多邊形（填色）或折線（描邊）。座標以字形方框左上角為原點。
#[derive(Clone, PartialEq, Debug)]
pub struct GlyphShape {
    pub points: Vec<(f64, f64)>,
    /// true = 封閉多邊形、填色；false = 開放折線、描邊。
    pub filled: bool,
}

/// 箭頭描邊寬（約為 semibold 10 點的筆畫）。
pub const CHEVRON_STROKE: f64 = 1.6;

/// 字形方框大小（寬、高），以字級 `size` 為準。
pub fn glyph_box(glyph: Glyph, size: f64) -> (f64, f64) {
    let size = if size.is_finite() && size > 0.0 {
        size
    } else {
        0.0
    };
    match glyph {
        Glyph::Previous | Glyph::Next => (size * 1.2, size * 0.75),
        Glyph::Play | Glyph::Pause => (size * 0.8, size * 0.85),
        Glyph::ChevronUp | Glyph::ChevronDown => (size, size * 0.6),
    }
}

/// 字形的形狀；所有點都在 [`glyph_box`] 之內。
pub fn glyph_shapes(glyph: Glyph, size: f64) -> Vec<GlyphShape> {
    let (w, h) = glyph_box(glyph, size);
    let fill = |points: Vec<(f64, f64)>| GlyphShape {
        points,
        filled: true,
    };
    match glyph {
        Glyph::Play => vec![fill(vec![(0.0, 0.0), (w, h / 2.0), (0.0, h)])],
        Glyph::Pause => {
            let bar = w * 0.34;
            vec![
                fill(vec![(0.0, 0.0), (bar, 0.0), (bar, h), (0.0, h)]),
                fill(vec![(w - bar, 0.0), (w, 0.0), (w, h), (w - bar, h)]),
            ]
        }
        Glyph::Next => {
            let half = w / 2.0;
            vec![
                fill(vec![(0.0, 0.0), (half, h / 2.0), (0.0, h)]),
                fill(vec![(half, 0.0), (w, h / 2.0), (half, h)]),
            ]
        }
        Glyph::Previous => {
            let half = w / 2.0;
            vec![
                fill(vec![(half, 0.0), (0.0, h / 2.0), (half, h)]),
                fill(vec![(w, 0.0), (half, h / 2.0), (w, h)]),
            ]
        }
        Glyph::ChevronUp | Glyph::ChevronDown => {
            // 描邊有寬度：折線往內縮半個筆畫，讓描出來的邊也在方框內。
            let i = (CHEVRON_STROKE / 2.0).min(w / 2.0).min(h / 2.0);
            let (top, bottom) = (i, h - i);
            let (tip, ends) = if glyph == Glyph::ChevronUp {
                (top, bottom)
            } else {
                (bottom, top)
            };
            vec![GlyphShape {
                points: vec![(i, ends), (w / 2.0, tip), (w - i, ends)],
                filled: false,
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicat_core::lyrics::LyricLine;
    use musicat_core::shell_logic::geometry::EXPANDED_CORNER_RADIUS;
    use musicat_core::types::PositionAnchor;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn expanded_budget_matches_handoff_item_14() {
        close(EXPANDED_CONTENT_HEIGHT, 172.0);
        close(MUSIC_PAGE_HEIGHT, 149.0);
        close(LYRICS_VIEWPORT_HEIGHT, 67.0);
        // 資訊欄：曲名 16 + 演出者 14 + 最小空白 2 + 進度 3 + 4 + 控制列 24 = 63 <= 68（封面高）。
        let info = TITLE_LINE_HEIGHT
            + ARTIST_LINE_HEIGHT
            + INFO_MIN_SPACER
            + PROGRESS_HEIGHT
            + PROGRESS_BOTTOM_PADDING
            + CONTROL_ROW_HEIGHT;
        assert!(info <= EXPANDED_COVER_SIDE, "{info}");
    }

    #[test]
    fn expanded_content_stays_inside_the_rounded_bottom_corners() {
        // GPUI 0.2.2 只能矩形裁切：內容矩形（左右各縮 16、底部縮 8）的角必須落在外殼下方 24 點圓角之內。
        let r = EXPANDED_CORNER_RADIUS;
        let x = PAGE_HORIZONTAL_PADDING; // 內容最左
        let y = EXPANDED_HEIGHT - PAGE_BOTTOM_PADDING; // 內容最下（外殼座標）
        let (cx, cy) = (r, EXPANDED_HEIGHT - r); // 左下圓角的圓心
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        assert!(
            x >= cx || y <= cy || d <= r,
            "corner point is outside: d={d}"
        );
        // 切換列在上方，離圓角很遠；只需檢查頁面的下緣。
    }

    #[test]
    fn wing_width_splits_the_rest_evenly() {
        close(wing_width(345.0, 185.0), 80.0);
        close(wing_width(360.0, 200.0), 80.0);
        close(wing_width(100.0, 200.0), 0.0);
        close(wing_width(f64::NAN, 200.0), 0.0);
    }

    #[test]
    fn preview_cover_side_follows_swift() {
        // 瀏海高 32：列高 − 8 = 24；翼寬 80 − 8 = 72 → 24。
        close(preview_cover_side(80.0, 32.0), 24.0);
        // 很窄的翼：翼寬 − 8。
        close(preview_cover_side(20.0, 32.0), 12.0);
        // 動畫中、列高接近 0：不為負。
        close(preview_cover_side(80.0, 3.0), 0.0);
        close(preview_cover_side(f64::NAN, 32.0), 0.0);
    }

    #[test]
    fn equalizer_rests_when_paused_and_bounces_while_playing() {
        for i in 0..4 {
            close(equalizer_scale(i, 1.234, false), EQ_RESTING_SCALE);
            close(equalizer_scale(i, 0.0, true), EQ_RESTING_SCALE);
            close(equalizer_scale(i, EQ_DURATIONS[i], true), EQ_PEAKS[i]);
            close(
                equalizer_scale(i, 2.0 * EQ_DURATIONS[i], true),
                EQ_RESTING_SCALE,
            );
            for k in 0..100 {
                let s = equalizer_scale(i, k as f64 * 0.037, true);
                assert!((EQ_RESTING_SCALE..=EQ_PEAKS[i] + 1e-9).contains(&s), "{s}");
            }
        }
        close(equalizer_scale(0, f64::NAN, true), EQ_RESTING_SCALE);
        close(equalizer_scale(9, 1.0, true), EQ_RESTING_SCALE);
        close(equalizer_width(), 18.0);
    }

    #[test]
    fn progress_fill_is_clamped() {
        close(progress_fill_width(Some(0.5), 200.0), 100.0);
        close(progress_fill_width(Some(1.5), 200.0), 200.0);
        close(progress_fill_width(Some(-0.5), 200.0), 0.0);
        close(progress_fill_width(Some(f64::NAN), 200.0), 0.0);
        close(progress_fill_width(None, 200.0), 0.0);
        close(progress_fill_width(Some(0.5), -10.0), 0.0);
    }

    fn snapshot(position: Option<PositionAnchor>) -> TrackSnapshot {
        TrackSnapshot {
            title: "Song".into(),
            artist: Some("Artist".into()),
            album: None,
            duration: Some(200.0),
            is_playing: true,
            position,
            artwork: None,
            source_id: "test".into(),
        }
    }

    fn synced() -> Lyrics {
        Lyrics::Synced(
            (0..5)
                .map(|i| LyricLine {
                    time: 10.0 * (i + 1) as f64,
                    text: format!("line {i}"),
                })
                .collect(),
        )
    }

    #[test]
    fn lyrics_display_branches_like_swift() {
        let anchor = PositionAnchor {
            elapsed: 25.0,
            timestamp: 1000.0,
            rate: 1.0,
        };
        let s = snapshot(Some(anchor));
        assert_eq!(lyrics_display(&s, None, 1000.0), LyricsDisplay::None);
        let plain = Lyrics::Plain("a\nb".into());
        assert_eq!(
            lyrics_display(&s, Some(&plain), 1000.0),
            LyricsDisplay::Plain("a\nb")
        );
        let synced = synced();
        let Lyrics::Synced(lines) = &synced else {
            unreachable!()
        };
        assert_eq!(
            lyrics_display(&s, Some(&synced), 1000.0),
            LyricsDisplay::Following {
                lines,
                current: Some(1)
            }
        );
        // 尚未到第一行：仍跟隨，但沒有目前行。
        assert_eq!(
            lyrics_display(&s, Some(&synced), 1000.0 - 20.0),
            LyricsDisplay::Following {
                lines,
                current: Some(1)
            },
            "timestamp in the future never rewinds below elapsed"
        );
        let early = snapshot(Some(PositionAnchor {
            elapsed: 3.0,
            ..anchor
        }));
        assert_eq!(
            lyrics_display(&early, Some(&synced), 1000.0),
            LyricsDisplay::Following {
                lines,
                current: None
            }
        );
        // 位置未知：靜態全文，絕不當成 0。
        assert_eq!(
            lyrics_display(&snapshot(None), Some(&synced), 1000.0),
            LyricsDisplay::Static(lines)
        );
    }

    #[test]
    fn scroll_offset_centres_the_current_line_within_bounds() {
        let vp = LYRICS_VIEWPORT_HEIGHT; // 67
                                         // 沒有目前行：頂端。
        close(lyric_scroll_offset(None, 20, vp), 0.0);
        // 前幾行：不捲過頂端。
        close(lyric_scroll_offset(Some(0), 20, vp), 0.0);
        close(lyric_scroll_offset(Some(1), 20, vp), 0.0);
        // 中間：目前行的中心在歌詞區中心。
        let off = lyric_scroll_offset(Some(10), 20, vp);
        close(off + 10.0 * LYRIC_PITCH + LYRIC_LINE_HEIGHT / 2.0, vp / 2.0);
        // 最後：不捲過底端（內容底緣貼齊歌詞區底緣）。
        let content = 20.0 * LYRIC_PITCH - LYRIC_LINE_SPACING;
        close(lyric_scroll_offset(Some(19), 20, vp), vp - content);
        close(lyric_scroll_offset(Some(99), 20, vp), vp - content);
        // 內容比歌詞區矮：永遠在頂端。
        close(lyric_scroll_offset(Some(2), 3, vp), 0.0);
        close(lyric_scroll_offset(Some(0), 0, vp), 0.0);
        close(lyric_scroll_offset(Some(5), 20, f64::NAN), 0.0);
    }

    #[test]
    fn animated_offset_eases_to_the_target() {
        close(animated_offset(0.0, -100.0, 0.0, 0.3), 0.0);
        close(animated_offset(0.0, -100.0, 0.15, 0.3), -50.0);
        close(animated_offset(0.0, -100.0, 0.3, 0.3), -100.0);
        close(animated_offset(0.0, -100.0, 5.0, 0.3), -100.0);
        close(animated_offset(0.0, -100.0, f64::NAN, 0.3), -100.0);
        close(animated_offset(0.0, -100.0, 0.1, 0.0), -100.0);
    }

    #[test]
    fn tick_interval_only_while_visible_and_live() {
        let base = TickInputs {
            visible: true,
            expanded: false,
            preview_playing: true,
            expanded_live: false,
            marquee_scrolling: false,
        };
        assert_eq!(tick_interval(base), Some(EQ_FRAME_SECONDS));
        // 跑馬燈捲動時改用較快的 30 fps（Preview 與 Expanded、播放與否都一樣），隱藏時仍不重繪。
        let scrolling = TickInputs {
            marquee_scrolling: true,
            ..base
        };
        assert_eq!(tick_interval(scrolling), Some(MARQUEE_FRAME_SECONDS));
        assert_eq!(
            tick_interval(TickInputs {
                expanded: true,
                preview_playing: false,
                ..scrolling
            }),
            Some(MARQUEE_FRAME_SECONDS)
        );
        assert_eq!(
            tick_interval(TickInputs {
                visible: false,
                ..scrolling
            }),
            None
        );
        assert_eq!(
            tick_interval(TickInputs {
                visible: false,
                ..base
            }),
            None
        );
        assert_eq!(
            tick_interval(TickInputs {
                preview_playing: false,
                ..base
            }),
            None
        );
        let expanded = TickInputs {
            expanded: true,
            ..base
        };
        assert_eq!(tick_interval(expanded), None);
        assert_eq!(
            tick_interval(TickInputs {
                expanded_live: true,
                ..expanded
            }),
            Some(EXPANDED_TICK_SECONDS)
        );
        const { assert!(EQ_FRAME_SECONDS <= STRIP_TICK_SECONDS) };
        const { assert!(MARQUEE_FRAME_SECONDS <= EQ_FRAME_SECONDS) };
    }

    #[test]
    fn glyph_shapes_stay_inside_their_box() {
        for glyph in [
            Glyph::Previous,
            Glyph::Play,
            Glyph::Pause,
            Glyph::Next,
            Glyph::ChevronUp,
            Glyph::ChevronDown,
        ] {
            for size in [10.0, 13.0, 16.0] {
                let (w, h) = glyph_box(glyph, size);
                assert!(w > 0.0 && h > 0.0);
                let shapes = glyph_shapes(glyph, size);
                assert!(!shapes.is_empty());
                for s in &shapes {
                    assert!(s.points.len() >= if s.filled { 3 } else { 2 });
                    for &(x, y) in &s.points {
                        assert!(
                            (0.0..=w).contains(&x) && (0.0..=h).contains(&y),
                            "{glyph:?}"
                        );
                    }
                }
            }
        }
        // 箭頭方向：向上的尖端在上方。
        let up = &glyph_shapes(Glyph::ChevronUp, 10.0)[0].points;
        assert!(up[1].1 < up[0].1);
        let down = &glyph_shapes(Glyph::ChevronDown, 10.0)[0].points;
        assert!(down[1].1 > down[0].1);
        assert!(glyph_shapes(Glyph::Play, f64::NAN)
            .iter()
            .all(|s| s.points.iter().all(|p| p.0 == 0.0 && p.1 == 0.0)));
    }

    #[test]
    fn source_label_uses_the_host_and_an_arrow() {
        assert_eq!(
            source_label("https://lrclib.net/api/get/1").as_deref(),
            Some("lrclib.net ↗")
        );
        assert_eq!(
            source_label("https://music.163.com/song?id=7").as_deref(),
            Some("music.163.com ↗")
        );
        assert_eq!(
            source_label("https://lrclib.net").as_deref(),
            Some("lrclib.net ↗")
        );
        assert!(SOURCE_LABEL_SUFFIX.starts_with(' '));
        assert!(source_label("https://lrclib.net/x")
            .unwrap()
            .ends_with(SOURCE_LABEL_SUFFIX));
    }

    #[test]
    fn www_prefix_is_dropped() {
        assert_eq!(
            source_label("https://www.example.com/a").as_deref(),
            Some("example.com ↗")
        );
        // 只去掉開頭的一次；主機只剩 "www." 時沒有東西可顯示。
        assert_eq!(
            source_label("https://www.www.example.com").as_deref(),
            Some("www.example.com ↗")
        );
        assert_eq!(source_label("https://www."), None);
        // 不改大小寫，也不把 "wwwx." 當成前綴。
        assert_eq!(
            source_label("https://wwwx.Example.com").as_deref(),
            Some("wwwx.Example.com ↗")
        );
    }

    #[test]
    fn port_userinfo_query_and_fragment_are_ignored() {
        for url in [
            "https://lrclib.net:443/api/get/1",
            "https://user:pw@lrclib.net/api",
            "https://lrclib.net?q=1",
            "https://lrclib.net#frag",
            "https://a@b@lrclib.net:8443?x#y",
            "https://lrclib.net:",
        ] {
            assert_eq!(source_label(url).as_deref(), Some("lrclib.net ↗"), "{url}");
        }
        // 方括號的 IPv6 主機保留原樣（裡面的冒號不是埠號）。
        assert_eq!(
            source_label("https://[::1]:8443/x").as_deref(),
            Some("[::1] ↗")
        );
        assert_eq!(
            source_label("https://[2001:db8::1]/x").as_deref(),
            Some("[2001:db8::1] ↗")
        );
        // 只有 userinfo 或只有埠號：沒有主機。
        assert_eq!(source_label("https://user@/x"), None);
        assert_eq!(source_label("https://:443/x"), None);
        // `@` 出現在路徑裡不算 userinfo。
        assert_eq!(
            source_label("https://lrclib.net/a@b").as_deref(),
            Some("lrclib.net ↗")
        );
    }

    #[test]
    fn very_long_host_is_truncated_with_an_ellipsis() {
        let host = format!("{}.example.com", "a".repeat(200));
        let label = source_label(&format!("https://{host}/x")).unwrap();
        assert!(
            label.chars().count() <= SOURCE_LABEL_MAX_CHARS + 2,
            "{label}"
        );
        assert!(
            label.ends_with(&format!("…{SOURCE_LABEL_SUFFIX}")),
            "{label}"
        );
        assert!(label.starts_with("aaaa"));
        // 剛好 MAX 個字元不截斷；多一個才截斷。
        let exact = "b".repeat(SOURCE_LABEL_MAX_CHARS);
        assert_eq!(
            source_label(&format!("https://{exact}")).unwrap(),
            format!("{exact}{SOURCE_LABEL_SUFFIX}")
        );
        let over = "b".repeat(SOURCE_LABEL_MAX_CHARS + 1);
        let label = source_label(&format!("https://{over}")).unwrap();
        assert_eq!(label.chars().count(), SOURCE_LABEL_MAX_CHARS + 2);
        assert!(label.contains('…'));
    }

    #[test]
    fn non_ascii_host_does_not_panic_or_split_a_char() {
        assert_eq!(
            source_label("https://歌詞.例子.台灣/x").as_deref(),
            Some("歌詞.例子.台灣 ↗")
        );
        // 多位元組字元剛好落在截斷邊界上（以字元截斷，不以位元組）。
        for prefix in ["", "a", "ab", "abc"] {
            for ch in ["é", "歌", "😀", "a\u{301}"] {
                let host = format!("{prefix}{}", ch.repeat(40));
                let label = source_label(&format!("https://{host}/p")).unwrap();
                assert!(
                    label.chars().count() <= SOURCE_LABEL_MAX_CHARS + 2,
                    "{label}"
                );
                assert!(label.ends_with(SOURCE_LABEL_SUFFIX));
            }
        }
        // 各種奇怪字串都不 panic。
        let weird = [
            "https://\u{0}",
            "https://\u{feff}/",
            "https://[",
            "https://]",
            "https://[::1",
            "https://[]:1",
            "https://@",
            "https://@@@",
            "https://::::",
            "https://?",
            "https://#",
            "https://\\x",
            "https:// /",
            "https://é:é@é:é/é",
            "HTTPS://lrclib.net",
            "https:https://",
            "\u{10ffff}https://x",
        ];
        for url in weird {
            let _ = source_label(url);
        }
        // 所有長度的前綴都不 panic（切在多位元組字元中間的位元組位置之前就以字元處理）。
        let long = "https://ä.歌😀.www.example.com:44/路?q#f";
        for (i, _) in long.char_indices() {
            let _ = source_label(&long[..i]);
        }
    }

    #[test]
    fn garbage_returns_none() {
        for url in [
            "",
            "https://",
            "not a url",
            "https:///x",
            "http://lrclib.net",
            "javascript:alert(1)",
            "https://?q",
            "https://#f",
            "https://www./x",
            "https:// /x",
            "https://\t/x",
        ] {
            assert_eq!(source_label(url), None, "{url:?}");
        }
    }

    #[test]
    fn strip_rows_fit_inside_the_strip_for_every_line_count() {
        use musicat_core::music::module::{lyric_strip_height, LYRIC_STRIP_ROW_HEIGHT};
        // 列高就是 12 pt 字加 4 的行高（跑馬燈用同一個值）。
        assert_eq!(LYRIC_STRIP_ROW_HEIGHT, STRIP_FONT_SIZE + 4.0);
        for lines in 1u8..=3 {
            let rows = f64::from(lines) * LYRIC_STRIP_ROW_HEIGHT;
            let height = lyric_strip_height(lines);
            // 上下各至少留 4 pt，字不會貼到下方圓角。
            assert!(height - rows >= 8.0, "lines = {lines}");
            assert!(STRIP_CORNER_RADIUS <= height);
        }
        const { assert!(STRIP_CONTEXT_OPACITY < STRIP_TITLE_OPACITY) };
    }

    #[test]
    fn source_label_fits_the_control_row_without_moving_the_buttons() {
        // 標籤一行，行高不超過控制列（不會撐高列）。
        const { assert!(SOURCE_LABEL_LINE_HEIGHT <= CONTROL_ROW_HEIGHT) };
        const { assert!(SOURCE_LABEL_FONT_SIZE <= SOURCE_LABEL_LINE_HEIGHT) };
        const { assert!(SOURCE_LABEL_OPACITY > 0.0 && SOURCE_LABEL_OPACITY < CONTROL_ENABLED_OPACITY) };
        // 按鈕群：3 顆 28 寬加 2 個 14 間距。
        close(control_buttons_width(), 112.0);
        for notch in [185.0, 200.0, 150.0] {
            // 資訊欄寬 = 外形寬（瀏海 + 160）− 左右內距 − 封面 − 間距。
            let info = notch + 160.0
                - 2.0 * PAGE_HORIZONTAL_PADDING
                - EXPANDED_COVER_SIDE
                - HEADER_SPACING;
            let (left, right) = control_side_widths(info);
            // 左右兩側等寬 → 按鈕群置中，與加標籤前（左 24 方塊 + flex_1、右 flex_1 + 24 折疊開關）相同。
            close(left, right);
            close(left + control_buttons_width() + right, info);
            close(
                left,
                CONTROL_ROW_HEIGHT
                    + (info - control_buttons_width() - 2.0 * CONTROL_ROW_HEIGHT) / 2.0,
            );
            // 右側仍放得下折疊開關；左側的標籤空位就是 `left`（標籤以 overflow hidden + 省略號截斷，不會推擠按鈕）。
            assert!(right >= CONTROL_ROW_HEIGHT, "{notch}");
            assert!(left > 0.0);
        }
        // 退化寬度不 panic、不為負。
        let (l, r) = control_side_widths(f64::NAN);
        assert!(l == 0.0 && r == 0.0);
        let (l, r) = control_side_widths(10.0);
        assert!(l == 0.0 && r == 0.0);
    }
}
