//! 選單模型的組裝與事件套用（純函式，不碰任何作業系統 API，也不需要 GPUI）。
//!
//! [`build_menu`] 依目前設定與已連接的螢幕組出 [`MenuModel`]，交給平台層的渲染器畫成原生選單；
//! 使用者點選後渲染器回報 [`MenuEvent`]，[`apply_event`] 把它寫進 [`Config`] 並回傳要執行的副作用
//! （[`Effect`]）。**除了 [`Effect::Quit`] 以外的每個副作用都代表設定剛被改動過**（沒有改動就不會有副作用）。
//! app 不直接呼叫 [`apply_event`]，而是經 [`crate::config::ConfigStore::apply_menu_event`]：它在副本上套用、
//! 只有設定真的改變時才存檔，然後把副作用交回給 `main` 執行。
//!
//! # 選單順序
//!
//! 螢幕選項（「自動」加每個已連接的螢幕）、分隔線、歌詞來源兩個開關、歌詞條開關、
//! （僅 macOS）perl 宿主開關、分隔線、結束。
//!
//! # 「自動」的勾選語意
//!
//! 「自動」只在設定的螢幕是 `None` 時勾選。設定裡記的螢幕目前沒接（被拔掉、闔上）時，
//! **「自動」與所有螢幕選項都不勾**：設定檔仍記著使用者的選擇（螢幕回來就移回去），
//! 選單不能假裝使用者選了「自動」。面板此時實際上在預設螢幕，那是 `choose_screen` 的退路，不是設定。

use musicat_core::menu::{MenuEvent, MenuItem, MenuModel};
use musicat_core::shell_logic::ScreenInfo;

use crate::config::{Config, Os};

/// 歌詞來源 LRCLIB 的開關 id。
pub const TOGGLE_LRCLIB: &str = "lyrics.lrclib";
/// 歌詞來源網易雲音樂的開關 id。
pub const TOGGLE_NETEASE: &str = "lyrics.netease";
/// 歌詞條顯示 1／2／3 行的選項 id（三個互斥的勾選項，順序即行數 1、2、3）。
pub const TOGGLE_LYRIC_LINES: [&str; 3] = [
    "music.lyric_lines.1",
    "music.lyric_lines.2",
    "music.lyric_lines.3",
];
/// macOS perl 宿主的開關 id。
pub const TOGGLE_PERL_HOST: &str = "macos.perl_host";
/// 歌詞條顯示的開關 id。
pub const TOGGLE_LYRIC_STRIP: &str = "music.lyric_strip";

/// 選單要為哪個平台組：只影響平台專屬的項目。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuPlatform {
    MacOs,
    /// 其他平台（Windows 系統匣在 R3）。
    Other,
}

impl From<Os> for MenuPlatform {
    fn from(os: Os) -> Self {
        match os {
            Os::MacOs => MenuPlatform::MacOs,
            Os::Windows | Os::Other => MenuPlatform::Other,
        }
    }
}

/// 套用選單事件後呼叫端要做的事。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// 歌詞來源開關變了：依新設定重建歌詞解析器（見 `switchable`）。
    RebuildLyricsProviders,
    /// perl 宿主開關變了：新狀態。
    SetPerlHost(bool),
    /// 歌詞條顯示變了：新狀態。
    SetLyricStripVisible(bool),
    /// 歌詞條一次顯示的行數變了：新行數（1..=3）。
    SetLyricStripLines(u8),
    /// 面板要移到這個螢幕；`None` = 自動（交給 `choose_screen`）。
    MoveToScreen(Option<String>),
    Quit,
}

/// 依設定與已連接的螢幕組出選單（順序與勾選語意見模組說明）。
pub fn build_menu(cfg: &Config, screens: &[ScreenInfo], platform: MenuPlatform) -> MenuModel {
    // 之後（R4）在這裡最前面加「切換模式」項目；目前只有 Notch 一種模式，沒有東西可切。
    let mut items = Vec::with_capacity(screens.len() + 8);

    items.push(MenuItem::ScreenChoice {
        screen_id: None,
        label: "自動".into(),
        checked: cfg.screen.is_none(),
    });
    for screen in screens {
        items.push(MenuItem::ScreenChoice {
            screen_id: Some(screen.id.clone()),
            // 螢幕沒有名字時退回 id，避免出現空白的選項。
            label: if screen.name.is_empty() {
                screen.id.clone()
            } else {
                screen.name.clone()
            },
            checked: cfg.screen.as_deref() == Some(screen.id.as_str()),
        });
    }

    items.push(MenuItem::Separator);
    items.push(toggle(
        TOGGLE_LRCLIB,
        "歌詞來源：LRCLIB",
        cfg.sources.lrclib,
    ));
    items.push(toggle(
        TOGGLE_NETEASE,
        "歌詞來源：網易雲音樂",
        cfg.sources.netease,
    ));
    items.push(toggle(
        TOGGLE_LYRIC_STRIP,
        "顯示歌詞條",
        cfg.lyric_strip_visible,
    ));
    // 歌詞條行數：三個互斥的勾選項（點已勾的那個沒有作用，見 `apply_event`）。
    for (n, id) in (1u8..).zip(TOGGLE_LYRIC_LINES) {
        items.push(toggle(
            id,
            &format!("歌詞條顯示 {n} 行"),
            cfg.lyric_strip_lines == n,
        ));
    }
    if platform == MenuPlatform::MacOs {
        items.push(toggle(
            TOGGLE_PERL_HOST,
            "使用 perl 宿主讀取現正播放（macOS）",
            cfg.macos.perl_host,
        ));
    }

    items.push(MenuItem::Separator);
    items.push(MenuItem::Quit);
    MenuModel { items }
}

fn toggle(id: &'static str, label: &str, checked: bool) -> MenuItem {
    MenuItem::Toggle {
        id,
        label: label.into(),
        checked,
    }
}

/// 把使用者的選單操作寫進設定，回傳要執行的副作用（規則見模組說明）。
/// 未知的開關 id 與沒有改變任何值的操作回傳空串列、不動設定。
pub fn apply_event(cfg: &mut Config, event: MenuEvent) -> Vec<Effect> {
    match event {
        MenuEvent::Toggled(TOGGLE_LRCLIB, v) => change(&mut cfg.sources.lrclib, v, |_| {
            Effect::RebuildLyricsProviders
        }),
        MenuEvent::Toggled(TOGGLE_NETEASE, v) => change(&mut cfg.sources.netease, v, |_| {
            Effect::RebuildLyricsProviders
        }),
        MenuEvent::Toggled(TOGGLE_PERL_HOST, v) => {
            change(&mut cfg.macos.perl_host, v, Effect::SetPerlHost)
        }
        MenuEvent::Toggled(TOGGLE_LYRIC_STRIP, v) => change(
            &mut cfg.lyric_strip_visible,
            v,
            Effect::SetLyricStripVisible,
        ),
        // 行數是互斥選項：只有「要選它」（true）才有作用；點已勾的那個回報 false，維持原樣。
        MenuEvent::Toggled(id, true) if TOGGLE_LYRIC_LINES.contains(&id) => {
            let lines = (1u8..)
                .zip(TOGGLE_LYRIC_LINES)
                .find_map(|(n, candidate)| (candidate == id).then_some(n))
                .unwrap_or(cfg.lyric_strip_lines);
            if cfg.lyric_strip_lines == lines {
                return Vec::new();
            }
            cfg.lyric_strip_lines = lines;
            vec![Effect::SetLyricStripLines(lines)]
        }
        MenuEvent::Toggled(..) => Vec::new(),
        MenuEvent::ScreenChosen(id) => {
            if cfg.screen == id {
                return Vec::new();
            }
            cfg.screen = id.clone();
            vec![Effect::MoveToScreen(id)]
        }
        MenuEvent::Quit => vec![Effect::Quit],
    }
}

/// 值有變才寫入並產生副作用；相同的值什麼都不做。
fn change(field: &mut bool, value: bool, effect: impl FnOnce(bool) -> Effect) -> Vec<Effect> {
    if *field == value {
        return Vec::new();
    }
    *field = value;
    vec![effect(value)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicat_core::shell_logic::geometry::Rect;

    fn screen(id: &str, name: &str) -> ScreenInfo {
        ScreenInfo {
            id: id.into(),
            name: name.into(),
            frame: Rect {
                x: 0.0,
                y: 0.0,
                w: 1512.0,
                h: 982.0,
            },
            hardware_notch: None,
            is_primary: false,
            is_builtin: false,
        }
    }

    fn cfg_with_screen(id: Option<&str>) -> Config {
        Config {
            screen: id.map(str::to_string),
            ..Config::default()
        }
    }

    fn toggle(id: &'static str, label: &str, checked: bool) -> MenuItem {
        MenuItem::Toggle {
            id,
            label: label.into(),
            checked,
        }
    }

    fn choice(id: Option<&str>, label: &str, checked: bool) -> MenuItem {
        MenuItem::ScreenChoice {
            screen_id: id.map(str::to_string),
            label: label.into(),
            checked,
        }
    }

    fn tail(perl: Option<bool>, lrclib: bool, netease: bool, strip: bool) -> Vec<MenuItem> {
        tail_with_lines(perl, lrclib, netease, strip, 1)
    }

    fn tail_with_lines(
        perl: Option<bool>,
        lrclib: bool,
        netease: bool,
        strip: bool,
        lines: u8,
    ) -> Vec<MenuItem> {
        let mut v = vec![
            MenuItem::Separator,
            toggle(TOGGLE_LRCLIB, "歌詞來源：LRCLIB", lrclib),
            toggle(TOGGLE_NETEASE, "歌詞來源：網易雲音樂", netease),
            toggle(TOGGLE_LYRIC_STRIP, "顯示歌詞條", strip),
            toggle(TOGGLE_LYRIC_LINES[0], "歌詞條顯示 1 行", lines == 1),
            toggle(TOGGLE_LYRIC_LINES[1], "歌詞條顯示 2 行", lines == 2),
            toggle(TOGGLE_LYRIC_LINES[2], "歌詞條顯示 3 行", lines == 3),
        ];
        if let Some(p) = perl {
            v.push(toggle(
                TOGGLE_PERL_HOST,
                "使用 perl 宿主讀取現正播放（macOS）",
                p,
            ));
        }
        v.push(MenuItem::Separator);
        v.push(MenuItem::Quit);
        v
    }

    #[test]
    fn ids_are_the_documented_strings() {
        assert_eq!(TOGGLE_LRCLIB, "lyrics.lrclib");
        assert_eq!(TOGGLE_NETEASE, "lyrics.netease");
        assert_eq!(TOGGLE_PERL_HOST, "macos.perl_host");
        assert_eq!(TOGGLE_LYRIC_STRIP, "music.lyric_strip");
        assert_eq!(
            TOGGLE_LYRIC_LINES,
            [
                "music.lyric_lines.1",
                "music.lyric_lines.2",
                "music.lyric_lines.3"
            ]
        );
    }

    #[test]
    fn menu_shape_with_no_screens_has_only_auto() {
        let m = build_menu(&Config::default(), &[], MenuPlatform::MacOs);
        let mut expected = vec![choice(None, "自動", true)];
        expected.extend(tail(Some(true), true, true, true));
        assert_eq!(m.items, expected);
    }

    #[test]
    fn menu_shape_with_one_screen() {
        let screens = [screen("1", "內建顯示器")];
        let m = build_menu(&Config::default(), &screens, MenuPlatform::MacOs);
        let mut expected = vec![
            choice(None, "自動", true),
            choice(Some("1"), "內建顯示器", false),
        ];
        expected.extend(tail(Some(true), true, true, true));
        assert_eq!(m.items, expected);
    }

    #[test]
    fn menu_shape_with_three_screens_and_the_second_selected() {
        let screens = [screen("1", "A"), screen("2", "B"), screen("3", "C")];
        let m = build_menu(&cfg_with_screen(Some("2")), &screens, MenuPlatform::MacOs);
        let mut expected = vec![
            choice(None, "自動", false),
            choice(Some("1"), "A", false),
            choice(Some("2"), "B", true),
            choice(Some("3"), "C", false),
        ];
        expected.extend(tail(Some(true), true, true, true));
        assert_eq!(m.items, expected);
    }

    #[test]
    fn exactly_one_screen_choice_is_checked_when_the_saved_screen_is_connected() {
        let screens = [screen("1", "A"), screen("2", "B")];
        for saved in [None, Some("1"), Some("2")] {
            let m = build_menu(&cfg_with_screen(saved), &screens, MenuPlatform::Other);
            let checked = m
                .items
                .iter()
                .filter(|i| matches!(i, MenuItem::ScreenChoice { checked: true, .. }))
                .count();
            assert_eq!(checked, 1, "{saved:?}");
        }
    }

    #[test]
    fn a_saved_screen_that_is_not_connected_leaves_nothing_checked_not_even_auto() {
        let screens = [screen("1", "A")];
        let m = build_menu(
            &cfg_with_screen(Some("gone")),
            &screens,
            MenuPlatform::MacOs,
        );
        let any_checked = m
            .items
            .iter()
            .any(|i| matches!(i, MenuItem::ScreenChoice { checked: true, .. }));
        assert!(!any_checked, "{:?}", m.items);
        // 設定本身沒被動到：螢幕回來時仍選著它。
        let back = [screen("1", "A"), screen("gone", "外接")];
        let m = build_menu(&cfg_with_screen(Some("gone")), &back, MenuPlatform::MacOs);
        assert!(m.items.contains(&choice(Some("gone"), "外接", true)));
    }

    #[test]
    fn a_screen_without_a_name_is_labelled_with_its_id() {
        let m = build_menu(&Config::default(), &[screen("42", "")], MenuPlatform::Other);
        assert!(m.items.contains(&choice(Some("42"), "42", false)));
    }

    #[test]
    fn toggles_reflect_the_config() {
        let mut cfg = Config::default();
        cfg.sources.lrclib = false;
        cfg.lyric_strip_visible = false;
        cfg.macos.perl_host = false;
        let m = build_menu(&cfg, &[], MenuPlatform::MacOs);
        let mut expected = vec![choice(None, "自動", true)];
        expected.extend(tail(Some(false), false, true, false));
        assert_eq!(m.items, expected);
    }

    #[test]
    fn the_line_count_options_show_exactly_the_configured_one() {
        for n in 1u8..=3 {
            let cfg = Config {
                lyric_strip_lines: n,
                ..Config::default()
            };
            let m = build_menu(&cfg, &[], MenuPlatform::MacOs);
            let mut expected = vec![choice(None, "自動", true)];
            expected.extend(tail_with_lines(Some(true), true, true, true, n));
            assert_eq!(m.items, expected, "lines = {n}");
        }
    }

    #[test]
    fn choosing_a_line_count_updates_the_config_and_reports_the_effect() {
        let mut cfg = Config::default();
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_LINES[2], true)),
            vec![Effect::SetLyricStripLines(3)]
        );
        assert_eq!(cfg.lyric_strip_lines, 3);
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_LINES[1], true)),
            vec![Effect::SetLyricStripLines(2)]
        );
        assert_eq!(cfg.lyric_strip_lines, 2);
    }

    #[test]
    fn choosing_the_current_line_count_again_changes_nothing() {
        let mut cfg = Config::default();
        // 已勾選的項目被點：選單回報「想要的新狀態」false。
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_LINES[0], false)),
            vec![]
        );
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_LINES[0], true)),
            vec![]
        );
        assert_eq!(cfg.lyric_strip_lines, 1);
    }

    #[test]
    fn the_perl_item_exists_only_on_macos() {
        let has_perl = |p| {
            build_menu(&Config::default(), &[], p)
                .items
                .iter()
                .any(|i| matches!(i, MenuItem::Toggle { id, .. } if *id == TOGGLE_PERL_HOST))
        };
        assert!(has_perl(MenuPlatform::MacOs));
        assert!(!has_perl(MenuPlatform::Other));
        let m = build_menu(&Config::default(), &[], MenuPlatform::Other);
        let mut expected = vec![choice(None, "自動", true)];
        expected.extend(tail(None, true, true, true));
        assert_eq!(m.items, expected);
    }

    #[test]
    fn menu_platform_follows_the_os() {
        assert_eq!(MenuPlatform::from(Os::MacOs), MenuPlatform::MacOs);
        assert_eq!(MenuPlatform::from(Os::Windows), MenuPlatform::Other);
        assert_eq!(MenuPlatform::from(Os::Other), MenuPlatform::Other);
    }

    #[test]
    fn lrclib_and_netease_toggles_rebuild_the_providers_only_when_changed() {
        let mut cfg = Config::default();
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LRCLIB, false)),
            vec![Effect::RebuildLyricsProviders]
        );
        assert!(!cfg.sources.lrclib && cfg.sources.netease);
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_NETEASE, false)),
            vec![Effect::RebuildLyricsProviders]
        );
        assert!(!cfg.sources.netease);
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LRCLIB, true)),
            vec![Effect::RebuildLyricsProviders]
        );
        assert!(cfg.sources.lrclib);
    }

    #[test]
    fn perl_host_toggle_reports_the_new_state() {
        let mut cfg = Config::default();
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_PERL_HOST, false)),
            vec![Effect::SetPerlHost(false)]
        );
        assert!(!cfg.macos.perl_host);
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_PERL_HOST, true)),
            vec![Effect::SetPerlHost(true)]
        );
        assert!(cfg.macos.perl_host);
    }

    #[test]
    fn lyric_strip_toggle_reports_the_new_state() {
        let mut cfg = Config::default();
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_STRIP, false)),
            vec![Effect::SetLyricStripVisible(false)]
        );
        assert!(!cfg.lyric_strip_visible);
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::Toggled(TOGGLE_LYRIC_STRIP, true)),
            vec![Effect::SetLyricStripVisible(true)]
        );
        assert!(cfg.lyric_strip_visible);
    }

    #[test]
    fn choosing_a_screen_sets_it_and_moves_the_panel() {
        let mut cfg = Config::default();
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::ScreenChosen(Some("2".into()))),
            vec![Effect::MoveToScreen(Some("2".into()))]
        );
        assert_eq!(cfg.screen.as_deref(), Some("2"));
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::ScreenChosen(None)),
            vec![Effect::MoveToScreen(None)]
        );
        assert_eq!(cfg.screen, None);
    }

    #[test]
    fn choosing_auto_clears_a_saved_screen_that_is_not_connected() {
        let mut cfg = cfg_with_screen(Some("gone"));
        assert_eq!(
            apply_event(&mut cfg, MenuEvent::ScreenChosen(None)),
            vec![Effect::MoveToScreen(None)]
        );
        assert_eq!(cfg.screen, None);
    }

    #[test]
    fn quit_returns_quit_and_leaves_the_config_alone() {
        let mut cfg = non_default_config();
        let before = cfg.clone();
        assert_eq!(apply_event(&mut cfg, MenuEvent::Quit), vec![Effect::Quit]);
        assert_eq!(cfg, before);
    }

    fn non_default_config() -> Config {
        let mut cfg = cfg_with_screen(Some("2"));
        cfg.sources.netease = false;
        cfg.macos.perl_host = false;
        cfg
    }

    #[test]
    fn repeating_the_same_value_is_a_no_op() {
        let mut cfg = non_default_config();
        let before = cfg.clone();
        for ev in [
            MenuEvent::Toggled(TOGGLE_LRCLIB, true),
            MenuEvent::Toggled(TOGGLE_NETEASE, false),
            MenuEvent::Toggled(TOGGLE_PERL_HOST, false),
            MenuEvent::Toggled(TOGGLE_LYRIC_STRIP, true),
            MenuEvent::ScreenChosen(Some("2".into())),
        ] {
            assert_eq!(apply_event(&mut cfg, ev.clone()), vec![], "{ev:?}");
            assert_eq!(cfg, before, "{ev:?} dirtied the config");
        }
        let mut auto = Config::default();
        assert_eq!(
            apply_event(&mut auto, MenuEvent::ScreenChosen(None)),
            vec![]
        );
        assert_eq!(auto, Config::default());
    }

    #[test]
    fn unknown_toggle_ids_do_nothing() {
        let mut cfg = non_default_config();
        let before = cfg.clone();
        for id in ["", "lyrics.unknown", "LYRICS.LRCLIB", "macos.perl_host "] {
            for v in [true, false] {
                assert_eq!(apply_event(&mut cfg, MenuEvent::Toggled(id, v)), vec![]);
            }
        }
        assert_eq!(cfg, before);
    }

    #[test]
    fn every_menu_item_round_trips_through_apply_event() {
        // 選單上每個開關被點一下：對應的設定欄位剛好翻轉一個、其他欄位不動、而且有副作用。
        let base = Config::default();
        let menu = build_menu(&base, &[screen("1", "A")], MenuPlatform::MacOs);
        let mut toggles = 0;
        for item in &menu.items {
            let Some(ev) = item.event_when_selected() else {
                continue;
            };
            let mut cfg = base.clone();
            let effects = apply_event(&mut cfg, ev.clone());
            match item {
                // 行數是互斥選項：目前勾選的那個（預設 1 行）再點一次沒有作用，其餘各改一個欄位。
                MenuItem::Toggle { id, .. } if *id == TOGGLE_LYRIC_LINES[0] => {
                    toggles += 1;
                    assert!(effects.is_empty(), "{ev:?}");
                    assert_eq!(cfg, base);
                }
                MenuItem::Toggle { .. } => {
                    toggles += 1;
                    assert_eq!(effects.len(), 1, "{ev:?}");
                    assert_ne!(cfg, base, "{ev:?} changed nothing");
                }
                MenuItem::ScreenChoice {
                    screen_id: None, ..
                } => assert!(effects.is_empty()),
                MenuItem::ScreenChoice { .. } => assert_eq!(effects.len(), 1),
                MenuItem::Quit => assert_eq!(effects, vec![Effect::Quit]),
                MenuItem::Separator => unreachable!(),
            }
        }
        assert_eq!(toggles, 7);
    }
}
