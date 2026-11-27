//! `main` 的組裝裡可以寫成純函式的部分：面板放在哪裡、螢幕變了要做什麼、開發用環境變數、日誌字串。
//! 不碰作業系統與 GPUI，所以在所有平台上編譯並測試；`main.rs` 只剩把這些接到 GPUI 與 AppKit 的膠水。

use std::path::PathBuf;
use std::time::Duration;

use musicat_core::lyrics::Lyrics;
use musicat_core::music::controller::MusicState;
use musicat_core::shell_logic::{
    choose_screen, geometry_for, NotchGeometry, ScreenInfo, VirtualNotch,
};
use musicat_core::types::TrackSnapshot;

use crate::config::{Config, NotchConfig};

/// 螢幕輪詢的間隔（spec：每 2 秒重新列舉一次螢幕）。
pub const SCREEN_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 設定裡的虛擬瀏海大小。
pub fn virtual_notch(cfg: &NotchConfig) -> VirtualNotch {
    VirtualNotch {
        w: cfg.virtual_width,
        h: cfg.virtual_height,
    }
}

/// 面板應該在的位置：`choose_screen`（設定的螢幕不在時退回預設螢幕）加 `geometry_for`。
/// 一個螢幕都沒有時為 `None`。
pub fn target_geometry(screens: &[ScreenInfo], cfg: &Config) -> Option<(String, NotchGeometry)> {
    let screen = choose_screen(screens, cfg.screen.as_deref())?;
    Some((
        screen.id.clone(),
        geometry_for(screen, virtual_notch(&cfg.notch)),
    ))
}

/// 面板要怎麼動。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Placement {
    /// 還沒有面板（啟動時沒有螢幕），現在有螢幕了：開在這裡。
    Open(NotchGeometry),
    /// 面板要移到（或改大小成）這個幾何。
    Move(NotchGeometry),
    /// 不動：位置沒變，或一個螢幕都沒有（**絕不因此結束程式或關掉面板**，等螢幕回來）。
    Stay,
}

/// 由目前的面板幾何（`None` = 還沒開）與新的目標決定動作。
pub fn placement(current: Option<NotchGeometry>, target: Option<NotchGeometry>) -> Placement {
    match (current, target) {
        (_, None) => Placement::Stay,
        (None, Some(t)) => Placement::Open(t),
        (Some(c), Some(t)) if c == t => Placement::Stay,
        (Some(_), Some(t)) => Placement::Move(t),
    }
}

/// 「沒有螢幕、面板也還沒開」這個狀況是否該寫日誌：狀況持續期間只寫一次（`logged` 記住已寫過），
/// 有螢幕可用時重設，下一次再發生才會再寫。輪詢每 2 秒一次，不能每次都印。
pub fn should_log_no_screens(logged: &mut bool, have_target: bool, panel_open: bool) -> bool {
    if have_target {
        *logged = false;
        return false;
    }
    if panel_open || *logged {
        return false;
    }
    *logged = true;
    true
}

/// 記住上一次列舉到的螢幕，回答「這次是否不同」。第一次觀察一律算不同。
#[derive(Default, Debug)]
pub struct ScreenTracker {
    last: Option<Vec<ScreenInfo>>,
}

impl ScreenTracker {
    /// 記下 `screens`；與上一次不同（數量、id、名稱、外框、瀏海任何一項）時回 `true`。
    pub fn observe(&mut self, screens: &[ScreenInfo]) -> bool {
        if self.last.as_deref() == Some(screens) {
            return false;
        }
        self.last = Some(screens.to_vec());
        true
    }

    /// 最近一次列舉到的螢幕（還沒觀察過時為空）。
    pub fn screens(&self) -> &[ScreenInfo] {
        self.last.as_deref().unwrap_or(&[])
    }
}

// ------------------------------------------------------------------ 開發／測試用環境變數

/// `MUSICAT_LOG=1`：印出曲目、歌詞與外殼狀態的日誌（含曲名與演出者）。其他值或沒設 = 關。
pub fn log_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// `MUSICAT_AUTO_QUIT_SECS=<n>`：n 秒後走與選單「結束」相同的路徑結束（開發／測試用）。
/// 必須是有限的正數；其他值（含 0、負數、非數字）= 不自動結束。
pub fn auto_quit_after(value: Option<&str>) -> Option<Duration> {
    let secs: f64 = value?.trim().parse().ok()?;
    (secs.is_finite() && secs > 0.0 && secs < 1e9).then(|| Duration::from_secs_f64(secs))
}

/// 設定檔路徑：`MUSICAT_CONFIG=<path>`（開發／測試用，非空才算）優先，否則平台預設的路徑。
pub fn config_path_with_override(
    override_value: Option<String>,
    default: Option<PathBuf>,
) -> Option<PathBuf> {
    match override_value {
        Some(p) if !p.trim().is_empty() => Some(PathBuf::from(p)),
        _ => default,
    }
}

// ------------------------------------------------------------------ 日誌字串

/// 曲目的一行描述。`detailed`（`MUSICAT_LOG=1`）時才帶曲名與演出者；否則只有狀態與來源，
/// 不洩漏使用者在聽什麼。
pub fn describe_snapshot(snapshot: Option<&TrackSnapshot>, detailed: bool) -> String {
    let Some(s) = snapshot else {
        return "no track".to_string();
    };
    let state = if s.is_playing { "playing" } else { "paused" };
    let position = if s.position.is_some() {
        "position known"
    } else {
        "position unknown"
    };
    if detailed {
        format!(
            "{state}: {:?} by {:?} [{}; {position}; artwork={}]",
            s.title,
            s.artist.as_deref().unwrap_or("?"),
            s.source_id,
            s.artwork.is_some()
        )
    } else {
        format!("{state} [{}; {position}]", s.source_id)
    }
}

/// 歌詞狀態的一行描述（不含歌詞內容）。
pub fn describe_lyrics(state: &MusicState) -> String {
    match &state.lyrics {
        None if state.searching => "searching lyrics".to_string(),
        None => "no lyrics".to_string(),
        Some(Lyrics::Synced(lines)) => format!("synced lyrics, {} lines", lines.len()),
        Some(Lyrics::Plain(text)) => format!("plain lyrics, {} lines", text.lines().count()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicat_core::shell_logic::geometry::{Rect, Size};
    use musicat_core::types::PositionAnchor;

    fn screen(id: &str, x: f64, notch: bool, builtin: bool, primary: bool) -> ScreenInfo {
        ScreenInfo {
            id: id.into(),
            name: format!("screen {id}"),
            frame: Rect {
                x,
                y: 0.0,
                w: 1512.0,
                h: 982.0,
            },
            hardware_notch: notch.then_some(Size { w: 185.0, h: 32.0 }),
            is_primary: primary,
            is_builtin: builtin,
        }
    }

    fn cfg(screen: Option<&str>) -> Config {
        Config {
            screen: screen.map(str::to_string),
            ..Config::default()
        }
    }

    #[test]
    fn target_prefers_the_configured_screen_and_falls_back_when_it_is_gone() {
        let builtin = screen("1", 0.0, true, true, true);
        let external = screen("2", 1512.0, false, false, false);
        let both = [builtin.clone(), external.clone()];

        let (id, g) = target_geometry(&both, &cfg(Some("2"))).unwrap();
        assert_eq!(id, "2");
        assert_eq!(g.center_x, 1512.0 + 756.0);
        assert_eq!(
            (g.notch.w, g.notch.h),
            (200.0, 32.0),
            "virtual notch on the external"
        );

        // 外接螢幕拔掉：回到預設（有瀏海的內建）螢幕；設定不變。
        let (id, g) = target_geometry(std::slice::from_ref(&builtin), &cfg(Some("2"))).unwrap();
        assert_eq!(id, "1");
        assert_eq!((g.notch.w, g.notch.h), (185.0, 32.0));

        // 回來了：移回去。
        assert_eq!(target_geometry(&both, &cfg(Some("2"))).unwrap().0, "2");
        // 自動。
        assert_eq!(target_geometry(&both, &cfg(None)).unwrap().0, "1");
        // 沒有螢幕。
        assert_eq!(target_geometry(&[], &cfg(None)), None);
    }

    #[test]
    fn the_virtual_notch_size_comes_from_the_config() {
        let mut c = cfg(None);
        c.notch.virtual_width = 300.0;
        c.notch.virtual_height = 40.0;
        let (_, g) = target_geometry(&[screen("9", 0.0, false, false, true)], &c).unwrap();
        assert_eq!((g.notch.w, g.notch.h), (300.0, 40.0));
    }

    #[test]
    fn the_no_screens_notice_is_logged_once_until_a_screen_appears() {
        let mut logged = false;
        // 沒有螢幕、面板沒開：第一次要寫，之後（每 2 秒的輪詢）都不寫。
        assert!(should_log_no_screens(&mut logged, false, false));
        for _ in 0..5 {
            assert!(!should_log_no_screens(&mut logged, false, false));
        }
        // 有螢幕了：不寫，並重設；再度沒有螢幕（面板仍沒開）時再寫一次。
        assert!(!should_log_no_screens(&mut logged, true, false));
        assert!(should_log_no_screens(&mut logged, false, false));
        assert!(!should_log_no_screens(&mut logged, false, false));
        // 面板已開時（螢幕暫時消失）不寫，也不佔用「已寫過」。
        let mut logged = false;
        assert!(!should_log_no_screens(&mut logged, false, true));
        assert!(!logged);
    }

    #[test]
    fn placement_opens_moves_or_stays_and_never_closes() {
        let a = target_geometry(&[screen("1", 0.0, true, true, true)], &cfg(None))
            .unwrap()
            .1;
        let b = target_geometry(&[screen("2", 1512.0, false, false, true)], &cfg(None))
            .unwrap()
            .1;
        assert_eq!(placement(None, Some(a)), Placement::Open(a));
        assert_eq!(placement(Some(a), Some(a)), Placement::Stay);
        assert_eq!(placement(Some(a), Some(b)), Placement::Move(b));
        assert_eq!(
            placement(Some(a), None),
            Placement::Stay,
            "no screens: keep the panel"
        );
        assert_eq!(placement(None, None), Placement::Stay);
    }

    #[test]
    fn screen_tracker_reports_changes_only() {
        let mut t = ScreenTracker::default();
        let one = vec![screen("1", 0.0, true, true, true)];
        let mut two = one.clone();
        two.push(screen("2", 1512.0, false, false, false));
        assert!(t.screens().is_empty());
        assert!(t.observe(&one), "first observation counts as a change");
        assert!(!t.observe(&one));
        assert!(t.observe(&two));
        assert_eq!(t.screens().len(), 2);
        assert!(t.observe(&[]), "all screens gone is a change");
        assert!(!t.observe(&[]));
        // 同一個螢幕改了解析度也算。
        let mut resized = one.clone();
        resized[0].frame.w = 1800.0;
        assert!(t.observe(&one));
        assert!(t.observe(&resized));
    }

    #[test]
    fn env_values() {
        assert!(log_enabled(Some("1")));
        for v in [None, Some("0"), Some("true"), Some("")] {
            assert!(!log_enabled(v), "{v:?}");
        }
        assert_eq!(auto_quit_after(Some("25")), Some(Duration::from_secs(25)));
        assert_eq!(
            auto_quit_after(Some(" 1.5 ")),
            Some(Duration::from_millis(1500))
        );
        for v in [
            None,
            Some(""),
            Some("0"),
            Some("-3"),
            Some("abc"),
            Some("inf"),
            Some("nan"),
        ] {
            assert_eq!(auto_quit_after(v), None, "{v:?}");
        }
        let default = Some(PathBuf::from("/default/musicat.toml"));
        assert_eq!(
            config_path_with_override(Some("/tmp/x.toml".into()), default.clone()),
            Some(PathBuf::from("/tmp/x.toml"))
        );
        assert_eq!(
            config_path_with_override(Some("  ".into()), default.clone()),
            default
        );
        assert_eq!(config_path_with_override(None, None), None);
    }

    fn snapshot() -> TrackSnapshot {
        TrackSnapshot {
            title: "Secret Song".into(),
            artist: Some("Secret Artist".into()),
            album: None,
            duration: Some(200.0),
            is_playing: true,
            position: Some(PositionAnchor {
                elapsed: 1.0,
                timestamp: 2.0,
                rate: 1.0,
            }),
            artwork: None,
            source_id: "mediaremote".into(),
        }
    }

    #[test]
    fn snapshot_descriptions_hide_titles_unless_detailed() {
        let s = snapshot();
        let quiet = describe_snapshot(Some(&s), false);
        assert!(!quiet.contains("Secret"), "{quiet}");
        assert!(quiet.contains("playing") && quiet.contains("mediaremote"));
        let loud = describe_snapshot(Some(&s), true);
        assert!(
            loud.contains("Secret Song") && loud.contains("Secret Artist"),
            "{loud}"
        );
        assert_eq!(describe_snapshot(None, true), "no track");
    }

    #[test]
    fn lyrics_descriptions_never_contain_the_text() {
        use musicat_core::lyrics::LyricLine;
        let mut state = MusicState::default();
        assert_eq!(describe_lyrics(&state), "no lyrics");
        state.searching = true;
        assert_eq!(describe_lyrics(&state), "searching lyrics");
        state.searching = false;
        state.lyrics = Some(Lyrics::Synced(vec![
            LyricLine {
                time: 1.0,
                text: "secret line".into(),
            };
            3
        ]));
        assert_eq!(describe_lyrics(&state), "synced lyrics, 3 lines");
        state.lyrics = Some(Lyrics::Plain("a\nsecret\nb".into()));
        assert_eq!(describe_lyrics(&state), "plain lyrics, 3 lines");
    }
}
