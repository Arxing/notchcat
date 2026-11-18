//! 選單列（macOS）／系統匣（Windows，R3）選單的平台中立資料模型。
//!
//! app 依設定組出 [`MenuModel`]（spec 第 3 節：切換模式、選擇顯示螢幕〔含「自動」〕、各歌詞來源開關、
//! macOS perl 宿主開關、歌詞條顯示、結束），交給平台層的渲染器（macOS 為
//! `musicat_platform_macos::tray::MenuBar`）畫成原生選單；使用者點選項目時，渲染器回報 [`MenuEvent`]。
//! 這裡不碰任何作業系統 API。
//!
//! # 為什麼開關的 id 是 `&'static str`
//!
//! - 選單上的開關是**程式裡固定的一組**（spec 第 3 節），id 由 app 以具名常數定義（例如
//!   `"lyrics.lrclib"`），不會來自使用者輸入或設定檔，所以 `'static` 的限制正好擋掉「執行時拼出 id」。
//! - 事件因此不必配置記憶體，[`MenuEvent`] 的開關事件可以直接用 `==` 比對常數。
//! - 不用 core 裡的 enum：開關的集合屬於 app 的設定（Task 25），放進 core 會讓 core 認識 app 的設定欄位，
//!   而 Windows 系統匣（R3）與之後新增的開關都得改 core。不用 `String`：沒有任何 id 需要在執行時產生。
//! - 代價：拼錯 id 不會在編譯期被抓到。由 app 把 id 集中成常數、並以「選單事件 → 設定欄位」的對照測試涵蓋。

/// 整個選單，依 `items` 的順序由上而下排列。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MenuModel {
    pub items: Vec<MenuItem>,
}

/// 選單上的一個項目。
#[derive(Clone, PartialEq, Debug)]
pub enum MenuItem {
    /// 有勾選狀態的開關（也用來表示「切換模式」裡的單一模式選項）。
    /// `checked` 是目前狀態；點選後回報的是**想要的新狀態**（`!checked`）。
    Toggle {
        id: &'static str,
        label: String,
        checked: bool,
    },
    /// 「顯示在哪個螢幕」的一個選項；同一個選單裡的所有 `ScreenChoice` 形成一組單選。
    /// `screen_id == None` 代表「自動」（macOS 有瀏海的優先、否則主螢幕）。
    ScreenChoice {
        screen_id: Option<String>,
        label: String,
        checked: bool,
    },
    /// 分隔線（不可點選）。
    Separator,
    /// 結束程式。標題由平台渲染器決定。
    Quit,
}

/// 使用者點選選單項目後，平台層回報給 app 的事件。
#[derive(Clone, PartialEq, Debug)]
pub enum MenuEvent {
    /// 開關被點選：`(id, 想要的新狀態)`。
    Toggled(&'static str, bool),
    /// 選擇了顯示螢幕；`None` 為「自動」。
    ScreenChosen(Option<String>),
    /// 結束。
    Quit,
}

impl MenuItem {
    /// 使用者點選這個項目時應回報的事件；分隔線不可點選，回 `None`。
    /// 所有平台的渲染器都經由這個函式產生事件，行為因此一致。
    pub fn event_when_selected(&self) -> Option<MenuEvent> {
        match self {
            MenuItem::Toggle { id, checked, .. } => Some(MenuEvent::Toggled(id, !checked)),
            MenuItem::ScreenChoice { screen_id, .. } => {
                Some(MenuEvent::ScreenChosen(screen_id.clone()))
            }
            MenuItem::Separator => None,
            MenuItem::Quit => Some(MenuEvent::Quit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle(id: &'static str, checked: bool) -> MenuItem {
        MenuItem::Toggle {
            id,
            label: id.to_string(),
            checked,
        }
    }

    #[test]
    fn selecting_an_unchecked_toggle_requests_on() {
        assert_eq!(
            toggle("lyrics.lrclib", false).event_when_selected(),
            Some(MenuEvent::Toggled("lyrics.lrclib", true))
        );
    }

    #[test]
    fn selecting_a_checked_toggle_requests_off() {
        assert_eq!(
            toggle("macos.perl_host", true).event_when_selected(),
            Some(MenuEvent::Toggled("macos.perl_host", false))
        );
    }

    #[test]
    fn selecting_auto_screen_reports_none() {
        let auto = MenuItem::ScreenChoice {
            screen_id: None,
            label: "自動".into(),
            checked: true,
        };
        assert_eq!(
            auto.event_when_selected(),
            Some(MenuEvent::ScreenChosen(None))
        );
    }

    #[test]
    fn selecting_a_screen_reports_its_id_even_when_already_checked() {
        let screen = MenuItem::ScreenChoice {
            screen_id: Some("2".into()),
            label: "LG".into(),
            checked: true,
        };
        assert_eq!(
            screen.event_when_selected(),
            Some(MenuEvent::ScreenChosen(Some("2".into())))
        );
    }

    #[test]
    fn separator_is_not_selectable() {
        assert_eq!(MenuItem::Separator.event_when_selected(), None);
    }

    #[test]
    fn quit_reports_quit() {
        assert_eq!(MenuItem::Quit.event_when_selected(), Some(MenuEvent::Quit));
    }

    #[test]
    fn model_is_plain_data() {
        let model = MenuModel {
            items: vec![toggle("a", true), MenuItem::Separator, MenuItem::Quit],
        };
        assert_eq!(model.clone(), model);
        assert_eq!(MenuModel::default().items, vec![]);
    }
}
