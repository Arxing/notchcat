//! 選單列圖示（`NSStatusItem` + `NSMenu`）：把 core 的 [`MenuModel`] 畫成原生選單，點選時回報 [`MenuEvent`]。
//!
//! 決策與 spike 紀錄見 `rust/docs/adr-0002-menu-bar-item.md`：GPUI 0.2.2 沒有可用的狀態列 API，
//! 所以這裡直接以 `objc2` 呼叫**公開**的 AppKit API（不是視窗繞路，也不碰任何私有 API）。
//! 這是 `panel.rs` 之外第二個寫 Objective-C 的檔案；`shell` 與 `app` 只呼叫本模組的公開介面。
//!
//! 結構：
//!
//! - 純函式 [`render_items`]／[`event_for_tag`]：`MenuModel` → 平台中立的 [`RenderedItem`] 列表，
//!   以及 NSMenuItem 的 `tag` → [`MenuEvent`]。所有平台都編譯並有單元測試，不需要 AppKit。
//! - [`MenuBar`]（只在 macOS）：持有 `NSStatusItem`，`set_model` 依 `render_items` 重建 `NSMenu`。
//!
//! 執行緒：`MenuBar` 的所有方法都必須在主執行緒呼叫（GPUI 的 `Application::run` 回呼與前景工作就在主執行緒），
//! 而且 `MenuBar` 本身不是 `Send`。`on_event` 註冊的回呼**一律在主執行緒上同步呼叫**（AppKit 在主執行緒派送
//! 選單動作）。回呼裡呼叫同一個 `MenuBar` 的 `set_model`／`on_event` 是安全的（煙霧範例實測過這條路徑）：
//! 被換下的選單由 [`MenuGenerations`] 保留到派送結束之後。不得在 GPUI 前景工作裡開巢狀 run loop，見 `MenuBar` 的文件。
//!
//! 生命週期：drop `MenuBar` 時從選單列移除圖示。每份 NSMenu 有自己的 Objective-C target，帶著建出它的模型快照，
//! 所以點舊選單永遠照舊模型解析。target 只持有共享狀態（回呼、開關狀態）與快照，共享狀態不回頭參考 target、選單或
//! `MenuBar`，所以沒有參考循環；NSMenuItem 對 target、NSMenu 對 delegate 是弱參考（AppKit 慣例），
//! target 與選單由 `MenuBar` 以世代保管。

use musicat_core::menu::{MenuEvent, MenuItem, MenuModel};

/// 「結束」項目的標題（core 的 `MenuItem::Quit` 不帶標題）。快捷鍵為 ⌘Q。
pub const QUIT_TITLE: &str = "結束 MusiCat";

/// `ScreenChoice { screen_id: None }` 的標題留空時使用的預設標題。
pub const AUTO_SCREEN_TITLE: &str = "自動";

/// 沒有圖示（或 PNG 解碼失敗）時，選單列上顯示的文字。正式的 template 圖示是之後的打磨工作。
pub const FALLBACK_TITLE: &str = "♪";

/// 渲染後項目的種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderedKind {
    /// 獨立的開關（勾選標記）。
    Toggle,
    /// 單選群組（顯示螢幕）的一個選項；同一群組最多一個勾選。
    Radio,
    Separator,
    Quit,
}

/// 平台中立的選單項目：AppKit（或之後 Windows 的系統匣）只要照著畫。
#[derive(Clone, PartialEq, Debug)]
pub struct RenderedItem {
    pub title: String,
    pub kind: RenderedKind,
    pub checked: bool,
    /// 交給 `NSMenuItem.tag` 的值：`items` 的索引 + 1。0 保留給不可點選的分隔線（NSMenuItem 的預設 tag 也是 0）。
    pub tag: isize,
}

/// `MenuModel` → 依序的 [`RenderedItem`]。純函式。
///
/// - 開關：標題取 `label`，空字串時退回 `id`。
/// - 螢幕選項：標題取 `label`，空字串時 `None` 用 [`AUTO_SCREEN_TITLE`]、`Some(id)` 用 id；
///   所有螢幕選項視為同一個單選群組，若模型裡勾了不只一個，只保留第一個勾選（避免畫出兩個勾）。
/// - 結束：標題為 [`QUIT_TITLE`]。
pub fn render_items(model: &MenuModel) -> Vec<RenderedItem> {
    let mut radio_checked_seen = false;
    model
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let tag = index as isize + 1;
            match item {
                MenuItem::Toggle { id, label, checked } => RenderedItem {
                    title: non_empty_or(label, id),
                    kind: RenderedKind::Toggle,
                    checked: *checked,
                    tag,
                },
                MenuItem::ScreenChoice {
                    screen_id,
                    label,
                    checked,
                } => {
                    let fallback = screen_id.as_deref().unwrap_or(AUTO_SCREEN_TITLE);
                    let shown = *checked && !radio_checked_seen;
                    radio_checked_seen |= shown;
                    RenderedItem {
                        title: non_empty_or(label, fallback),
                        kind: RenderedKind::Radio,
                        checked: shown,
                        tag,
                    }
                }
                MenuItem::Separator => RenderedItem {
                    title: String::new(),
                    kind: RenderedKind::Separator,
                    checked: false,
                    tag: 0,
                },
                MenuItem::Quit => RenderedItem {
                    title: QUIT_TITLE.to_string(),
                    kind: RenderedKind::Quit,
                    checked: false,
                    tag,
                },
            }
        })
        .collect()
}

/// NSMenuItem 的 `tag`（由 [`render_items`] 產生）→ 該項目點選時的事件。純函式。
/// tag 不在範圍內（含分隔線的 0）時回 `None`。
pub fn event_for_tag(model: &MenuModel, tag: isize) -> Option<MenuEvent> {
    let index = usize::try_from(tag).ok()?.checked_sub(1)?;
    model.items.get(index)?.event_when_selected()
}

fn non_empty_or(label: &str, fallback: &str) -> String {
    if label.is_empty() {
        fallback.to_string()
    } else {
        label.to_string()
    }
}

/// 每個建出來的原生選單（「一代」）的保管規則。純邏輯，泛型參數在 macOS 上是（NSMenu、它的 target）。
///
/// 為什麼需要（code review 的兩個問題）：
///
/// 1. **過期的選單**：`set_model` 換上新選單時，使用者可能正開著舊選單；舊選單的項目必須照**它自己的模型快照**
///    解析（見 [`event_for_tag`]），而且它的 target 在選單關閉前不能被釋放（NSMenuItem 對 target 是弱參考）。
/// 2. **在回呼裡呼叫 `set_model`**：回呼是在舊選單的動作派送裡被呼叫的；AppKit 在回呼返回後仍會使用該選單與項目，
///    所以被換下的選單不能在這時釋放。
///
/// 規則：被換下的世代先放進 `retired`。[`MenuGenerations::replace`] 只有在「沒有選單開著、也不在任何選單動作派送中」
/// 時，才把**更早**退休的世代交回給呼叫端釋放；剛被換下的那一代一律留到下一次 `replace`（或 [`MenuGenerations::take_all`]）。
#[derive(Debug)]
pub struct MenuGenerations<T> {
    current: Option<T>,
    retired: Vec<T>,
}

impl<T> Default for MenuGenerations<T> {
    fn default() -> Self {
        Self {
            current: None,
            retired: Vec::new(),
        }
    }
}

impl<T> MenuGenerations<T> {
    /// 目前掛在選單列上的世代。
    pub fn current(&self) -> Option<&T> {
        self.current.as_ref()
    }

    /// 最近一次被換下、仍被保留的世代。
    pub fn last_retired(&self) -> Option<&T> {
        self.retired.last()
    }

    /// 仍被保留的退休世代數。
    pub fn retired_len(&self) -> usize {
        self.retired.len()
    }

    /// 換上 `next`。`busy` = 有選單開著或正在派送選單動作。回傳可以釋放的世代（呼叫端在放掉所有借用後再釋放）。
    pub fn replace(&mut self, next: T, busy: bool) -> Vec<T> {
        let released = if busy {
            Vec::new()
        } else {
            std::mem::take(&mut self.retired)
        };
        if let Some(old) = self.current.replace(next) {
            self.retired.push(old);
        }
        released
    }

    /// 取出全部世代（drop `MenuBar` 時用）。
    pub fn take_all(&mut self) -> Vec<T> {
        let mut all = std::mem::take(&mut self.retired);
        all.extend(self.current.take());
        all
    }
}

#[cfg(target_os = "macos")]
pub use appkit::MenuBar;

/// 目前是否在主執行緒（`MainThreadMarker::new().is_some()`）。給 `shell`／`app` 檢查回呼的執行緒用，
/// 讓它們不必自己碰 objc2。
#[cfg(target_os = "macos")]
pub fn is_main_thread() -> bool {
    objc2::MainThreadMarker::new().is_some()
}

#[cfg(target_os = "macos")]
mod appkit {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use musicat_core::menu::{MenuEvent, MenuModel};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{
        define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuDelegate, NSMenuItem,
        NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSData, NSSize, NSString};

    use super::{event_for_tag, render_items, MenuGenerations, RenderedKind, FALLBACK_TITLE};

    type Callback = Rc<dyn Fn(MenuEvent) + Send + Sync>;

    /// 所有世代的 target 與 `MenuBar` 共用的狀態。不持有 target、選單或 `MenuBar`，所以沒有參考循環。
    #[derive(Default)]
    struct Shared {
        callback: RefCell<Option<Callback>>,
        /// `menuWillOpen:` 到 `menuDidClose:` 之間為 true（NSMenuDelegate）。
        menu_open: Cell<bool>,
        /// `menuWillOpen:` 的累計次數（診斷用）。
        opens: Cell<u64>,
        /// 正在執行中的選單動作數（`menuItemClicked:` 的巢狀深度）。
        dispatching: Cell<u32>,
        /// `MenuBar` 被 drop 後為 true：已排程的 `openMenuNow:` 不再動作。
        dropped: Cell<bool>,
    }

    /// 一個 target 的 ivars：共用狀態，加上**建出這份選單的模型快照**。
    struct TargetIvars {
        shared: Rc<Shared>,
        snapshot: MenuModel,
    }

    define_class!(
        // SAFETY:
        // - 父類別 NSObject 沒有子類別化的要求。
        // - `MenuTarget` 沒有實作 `Drop`。
        // - 只在主執行緒使用（MainThreadOnly），所以 ivars 裡的 `Rc`／`Cell` 不會跨執行緒。
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MusiCatMenuTarget"]
        #[ivars = TargetIvars]
        struct MenuTarget;

        impl MenuTarget {
            /// 所有 MusiCat 選單項目的動作。AppKit 在主執行緒呼叫。
            /// tag 一律以**這個 target 的快照**解析：使用者點的是哪一份選單，就照哪一份模型回報。
            #[unsafe(method(menuItemClicked:))]
            fn menu_item_clicked(&self, sender: &NSMenuItem) {
                let ivars = self.ivars();
                let shared = &ivars.shared;
                let Some(event) = event_for_tag(&ivars.snapshot, sender.tag()) else {
                    return;
                };
                // 先複製出回呼再呼叫：回呼裡可以呼叫 on_event／set_model。
                let callback = shared.callback.borrow().clone();
                if let Some(callback) = callback {
                    shared.dispatching.set(shared.dispatching.get() + 1);
                    callback(event);
                    shared.dispatching.set(shared.dispatching.get() - 1);
                }
            }

            /// [`MenuBar::open_menu`] 排程的動作：點一下圖示按鈕（與使用者點擊同一條路徑），
            /// 在這裡進入 AppKit 的選單追蹤迴圈，直到選單關閉才返回。`MenuBar` 已被 drop 時什麼都不做。
            #[unsafe(method(openMenuNow:))]
            fn open_menu_now(&self, item: &NSStatusItem) {
                if self.ivars().shared.dropped.get() {
                    return;
                }
                if let Some(button) = item.button(self.mtm()) {
                    // SAFETY: sender 傳 nil 是 performClick: 允許的。
                    unsafe { button.performClick(None) };
                }
            }
        }

        unsafe impl NSObjectProtocol for MenuTarget {}

        unsafe impl NSMenuDelegate for MenuTarget {
            #[unsafe(method(menuWillOpen:))]
            fn menu_will_open(&self, _menu: &NSMenu) {
                let shared = &self.ivars().shared;
                shared.menu_open.set(true);
                shared.opens.set(shared.opens.get() + 1);
            }

            #[unsafe(method(menuDidClose:))]
            fn menu_did_close(&self, _menu: &NSMenu) {
                self.ivars().shared.menu_open.set(false);
            }
        }
    );

    impl MenuTarget {
        fn new(mtm: MainThreadMarker, shared: Rc<Shared>, snapshot: MenuModel) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { shared, snapshot });
            // SAFETY: NSObject 的 init，ivars 已設定。
            unsafe { msg_send![super(this), init] }
        }
    }

    /// 一代選單：NSMenu 與它專屬的 target（target 帶著建出這份選單的模型快照）。
    struct Generation {
        menu: Retained<NSMenu>,
        target: Retained<MenuTarget>,
    }

    /// 選單列上的 MusiCat 圖示與它的選單。drop 時從選單列移除。只能在主執行緒使用（不是 `Send`）。
    ///
    /// **巢狀 run loop 的禁忌**：GPUI 0.2.2 的前景工作跑在序列的主 dispatch queue 區塊裡，所以絕不能在 GPUI 的前景工作
    /// 或 `update` 裡同步打開選單、跑 modal（`NSAlert runModal` 等）或任何巢狀 run loop——期間所有 GPUI 前景工作都會停住
    /// （spike 實測：337 ms 內面板輪詢 0 次，見 ADR-0002）。本型別的 [`MenuBar::set_model`]、[`MenuBar::perform_item`]、
    /// [`MenuBar::close_menu`]、[`MenuBar::on_event`]、[`MenuBar::describe`] 都不會開巢狀 run loop，可以在 GPUI 前景工作裡呼叫；
    /// [`MenuBar::open_menu`] 會把開選單排到 run loop 的下一輪，本身也立即返回。
    ///
    /// 回呼裡呼叫 `set_model` 是安全的：被換下的選單（正在派送動作的那一份）會保留到下一次 `set_model` 或 drop
    /// 之後才釋放（見 [`MenuGenerations`]），drop 時則交給 autorelease pool，在目前的事件派送結束後才釋放。
    pub struct MenuBar {
        item: Retained<NSStatusItem>,
        shared: Rc<Shared>,
        generations: RefCell<MenuGenerations<Generation>>,
        mtm: MainThreadMarker,
    }

    impl MenuBar {
        /// 在選單列建立圖示。`icon` 是 PNG 位元組（會設成 template 圖示，由系統依淺色／深色選單列著色，
        /// 高 18 點）；`None` 或解碼失敗時改顯示文字 [`FALLBACK_TITLE`]。
        ///
        /// 必須在主執行緒、`NSApplication` 已啟動之後呼叫（即 GPUI `Application::run` 的回呼裡）；
        /// 不在主執行緒時印警告並回 `None`。一開始選單是空的，請接著呼叫 [`MenuBar::set_model`]。
        pub fn new(icon: Option<&[u8]>) -> Option<MenuBar> {
            let Some(mtm) = MainThreadMarker::new() else {
                eprintln!("musicat: MenuBar::new() must run on the main thread; no menu bar item");
                return None;
            };
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            let Some(button) = item.button(mtm) else {
                eprintln!("musicat: NSStatusItem has no button; no menu bar item");
                NSStatusBar::systemStatusBar().removeStatusItem(&item);
                return None;
            };
            match icon.and_then(template_image) {
                Some(image) => button.setImage(Some(&image)),
                None => button.setTitle(&NSString::from_str(FALLBACK_TITLE)),
            }
            let bar = MenuBar {
                item,
                shared: Rc::new(Shared::default()),
                generations: RefCell::new(MenuGenerations::default()),
                mtm,
            };
            bar.set_model(MenuModel::default());
            Some(bar)
        }

        /// 以新的模型建一份新選單換上去（勾選以 `NSControlStateValueOn/Off` 呈現、分隔線、結束為 ⌘Q）。
        /// 點選開關**不會**自動改變勾選：app 收到事件、更新設定後再呼叫一次本函式。
        ///
        /// 已經開著的舊選單維持原樣，點它的項目照**舊選單自己的模型**回報事件（不會對到新模型的別的項目）。
        /// 可以在 `on_event` 的回呼裡呼叫。
        pub fn set_model(&self, model: MenuModel) {
            let mtm = self.mtm;
            let rendered = render_items(&model);
            let target = MenuTarget::new(mtm, self.shared.clone(), model);
            let menu = NSMenu::new(mtm);
            menu.setAutoenablesItems(false);
            // NSMenu 對 delegate 是弱參考；target 與選單放在同一個世代裡一起保管。
            menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
            for r in rendered {
                if r.kind == RenderedKind::Separator {
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                    continue;
                }
                let key = if r.kind == RenderedKind::Quit {
                    "q"
                } else {
                    ""
                };
                // SAFETY: `menuItemClicked:` 是 `MenuTarget` 定義的方法，簽章為 `(&self, &NSMenuItem)`，
                // 與 NSMenuItem 動作的 `- (void)action:(id)sender` 相符。NSMenuItem 對 target 是弱參考；
                // target 與這份選單同屬一個 `Generation`，由 `generations` 保管到選單不再可能被使用為止。
                let item = unsafe {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&r.title),
                        Some(sel!(menuItemClicked:)),
                        &NSString::from_str(key),
                    );
                    item.setTarget(Some(&target));
                    item
                };
                item.setTag(r.tag);
                item.setEnabled(true);
                item.setState(if r.checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                menu.addItem(&item);
            }
            self.item.setMenu(Some(&menu));
            let busy = self.shared.menu_open.get() || self.shared.dispatching.get() > 0;
            let released = self
                .generations
                .borrow_mut()
                .replace(Generation { menu, target }, busy);
            // 借用已放掉；更早的世代此刻沒有開著、也不在派送中，直接釋放。
            drop(released);
        }

        /// 設定（取代）點選事件的回呼。回呼在主執行緒上同步呼叫，應盡快返回
        /// （例如把事件送進通道，由 GPUI 前景工作處理）；不得在回呼裡借用 GPUI 的 `App`。
        /// `MenuEvent::Quit` 不會自動結束程式：由 app 處理。
        pub fn on_event(&self, callback: Box<dyn Fn(MenuEvent) + Send + Sync>) {
            *self.shared.callback.borrow_mut() = Some(Rc::from(callback));
        }

        fn current_menu(&self) -> Option<Retained<NSMenu>> {
            self.generations.borrow().current().map(|g| g.menu.clone())
        }

        /// 以程式觸發目前選單的第 `index` 個項目（含分隔線的位置），走 AppKit 正常的 target/action 派送
        /// （`-[NSMenu performActionForItemAtIndex:]`）。給煙霧測試與診斷用。索引超出範圍時回 `false`。
        pub fn perform_item(&self, index: usize) -> bool {
            perform(self.current_menu(), index)
        }

        /// 同 [`MenuBar::perform_item`]，但對象是**最近一次被換下、仍被保留**的舊選單（模擬「使用者開著舊選單時
        /// 模型被換掉，然後點了舊選單」）。給煙霧測試與診斷用。沒有保留中的舊選單或索引超出範圍時回 `false`。
        pub fn perform_retired_item(&self, index: usize) -> bool {
            let menu = self
                .generations
                .borrow()
                .last_retired()
                .map(|g| g.menu.clone());
            perform(menu, index)
        }

        /// 以程式點一下選單列圖示、打開選單（與使用者點擊同一條路徑）。給煙霧測試與診斷用。
        ///
        /// **不會阻塞**：以 `performSelector:withObject:afterDelay:0` 排到主執行緒 run loop 的下一輪才點，
        /// 而不是在呼叫端當下點（理由見型別文件的「巢狀 run loop 的禁忌」）。觸發前 `MenuBar` 已被 drop 時什麼都不做。
        pub fn open_menu(&self) {
            let Some(target) = self
                .generations
                .borrow()
                .current()
                .map(|g| g.target.clone())
            else {
                return;
            };
            // SAFETY: `openMenuNow:` 是 `MenuTarget` 定義的方法，參數為 NSStatusItem；
            // performSelector:withObject:afterDelay: 會 retain target 與參數直到觸發。
            unsafe {
                let _: () = msg_send![
                    &*target,
                    performSelector: sel!(openMenuNow:),
                    withObject: &*self.item,
                    afterDelay: 0.0f64
                ];
            }
        }

        /// 選單目前是否開著（NSMenuDelegate 的 `menuWillOpen:`／`menuDidClose:`）。
        pub fn is_menu_open(&self) -> bool {
            self.shared.menu_open.get()
        }

        /// 關閉正在顯示的選單（沒開時無作用）。
        pub fn close_menu(&self) {
            if let Some(menu) = self.item.menu(self.mtm) {
                menu.cancelTrackingWithoutAnimation();
            }
        }

        /// 給診斷用的一行狀態：是否可見、按鈕標題、圖示所在視窗的 AppKit 外框、保留中的舊選單數、選單項目與勾選。
        pub fn describe(&self) -> String {
            let mtm = self.mtm;
            let (title, frame) = match self.item.button(mtm) {
                Some(button) => {
                    let title = match button.image() {
                        Some(image) => {
                            let s = image.size();
                            format!(
                                "{}<image {}x{} template={}>",
                                button.title(),
                                s.width,
                                s.height,
                                image.isTemplate()
                            )
                        }
                        None => button.title().to_string(),
                    };
                    let frame = button.window().map(|w| {
                        let f = w.frame();
                        format!(
                            "({}, {}, {}x{}) level={}",
                            f.origin.x,
                            f.origin.y,
                            f.size.width,
                            f.size.height,
                            w.level()
                        )
                    });
                    (title, frame.unwrap_or_else(|| "none".into()))
                }
                None => ("<no button>".into(), "none".into()),
            };
            let items = self
                .item
                .menu(mtm)
                .map(|menu| {
                    (0..menu.numberOfItems())
                        .filter_map(|i| menu.itemAtIndex(i))
                        .map(|it| {
                            if it.isSeparatorItem() {
                                "---".to_string()
                            } else {
                                let mark = if it.state() == NSControlStateValueOn {
                                    "[x]"
                                } else {
                                    "[ ]"
                                };
                                format!("{mark} {}#{}", it.title(), it.tag())
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default();
            format!(
                "visible={} menuOpen={} opens={} retired={} title={title:?} buttonWindow={frame} menu=[{items}]",
                self.item.isVisible(),
                self.is_menu_open(),
                self.shared.opens.get(),
                self.generations.borrow().retired_len(),
            )
        }
    }

    fn perform(menu: Option<Retained<NSMenu>>, index: usize) -> bool {
        // 不持有任何 RefCell 借用：動作的回呼可能呼叫 set_model。
        let Some(menu) = menu else {
            return false;
        };
        let Ok(i) = isize::try_from(index) else {
            return false;
        };
        if i >= menu.numberOfItems() {
            return false;
        }
        menu.performActionForItemAtIndex(i);
        true
    }

    impl Drop for MenuBar {
        fn drop(&mut self) {
            self.shared.dropped.set(true);
            self.item.setMenu(None);
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
            // drop 可能發生在回呼裡（正在派送某份選單的動作），所以不立即釋放選單與 target：
            // 交給目前的 autorelease pool，在這次事件派送結束、pool 排空時才釋放。
            for g in self.generations.get_mut().take_all() {
                let _ = Retained::autorelease_ptr(g.menu);
                let _ = Retained::autorelease_ptr(g.target);
            }
        }
    }

    /// PNG 位元組 → 18 點高的 template NSImage；解碼失敗回 `None`。
    fn template_image(png: &[u8]) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(png);
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        let s = image.size();
        if !(s.width > 0.0 && s.height > 0.0) {
            return None;
        }
        image.setSize(NSSize::new(18.0 * s.width / s.height, 18.0));
        image.setTemplate(true);
        Some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MenuModel {
        MenuModel {
            items: vec![
                MenuItem::Toggle {
                    id: "mode.notch",
                    label: "瀏海式".into(),
                    checked: true,
                },
                MenuItem::Separator,
                MenuItem::ScreenChoice {
                    screen_id: None,
                    label: "自動".into(),
                    checked: false,
                },
                MenuItem::ScreenChoice {
                    screen_id: Some("1".into()),
                    label: "內建顯示器".into(),
                    checked: true,
                },
                MenuItem::Separator,
                MenuItem::Toggle {
                    id: "lyrics.lrclib",
                    label: "LRCLIB".into(),
                    checked: false,
                },
                MenuItem::Separator,
                MenuItem::Quit,
            ],
        }
    }

    #[test]
    fn renders_titles_kinds_and_checked_state_in_order() {
        let r = render_items(&sample());
        let summary: Vec<_> = r
            .iter()
            .map(|i| (i.title.as_str(), i.kind, i.checked))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("瀏海式", RenderedKind::Toggle, true),
                ("", RenderedKind::Separator, false),
                ("自動", RenderedKind::Radio, false),
                ("內建顯示器", RenderedKind::Radio, true),
                ("", RenderedKind::Separator, false),
                ("LRCLIB", RenderedKind::Toggle, false),
                ("", RenderedKind::Separator, false),
                (QUIT_TITLE, RenderedKind::Quit, false),
            ]
        );
    }

    #[test]
    fn every_clickable_tag_round_trips_to_its_items_event() {
        let model = sample();
        let rendered = render_items(&model);
        assert_eq!(rendered.len(), model.items.len());
        for (index, rendered) in rendered.iter().enumerate() {
            let expected = model.items[index].event_when_selected();
            if rendered.kind == RenderedKind::Separator {
                assert_eq!(rendered.tag, 0);
                assert_eq!(expected, None);
            } else {
                assert_eq!(
                    event_for_tag(&model, rendered.tag),
                    expected,
                    "item {index}"
                );
                assert!(expected.is_some());
            }
        }
    }

    #[test]
    fn tags_map_to_the_expected_events() {
        let model = sample();
        assert_eq!(
            event_for_tag(&model, 1),
            Some(MenuEvent::Toggled("mode.notch", false))
        );
        assert_eq!(
            event_for_tag(&model, 3),
            Some(MenuEvent::ScreenChosen(None))
        );
        assert_eq!(
            event_for_tag(&model, 4),
            Some(MenuEvent::ScreenChosen(Some("1".into())))
        );
        assert_eq!(
            event_for_tag(&model, 6),
            Some(MenuEvent::Toggled("lyrics.lrclib", true))
        );
        assert_eq!(event_for_tag(&model, 8), Some(MenuEvent::Quit));
    }

    #[test]
    fn unknown_tags_yield_no_event() {
        let model = sample();
        for tag in [0, -1, 2, 9, isize::MAX, isize::MIN] {
            assert_eq!(event_for_tag(&model, tag), None, "tag {tag}");
        }
        assert_eq!(event_for_tag(&MenuModel::default(), 1), None);
    }

    #[test]
    fn empty_labels_fall_back_to_auto_screen_id_or_toggle_id() {
        let model = MenuModel {
            items: vec![
                MenuItem::ScreenChoice {
                    screen_id: None,
                    label: String::new(),
                    checked: false,
                },
                MenuItem::ScreenChoice {
                    screen_id: Some("42".into()),
                    label: String::new(),
                    checked: false,
                },
                MenuItem::Toggle {
                    id: "lyrics.netease",
                    label: String::new(),
                    checked: false,
                },
            ],
        };
        let titles: Vec<_> = render_items(&model).into_iter().map(|r| r.title).collect();
        assert_eq!(titles, vec![AUTO_SCREEN_TITLE, "42", "lyrics.netease"]);
    }

    #[test]
    fn only_the_first_checked_screen_choice_is_shown_checked() {
        let choice = |id: &str, checked| MenuItem::ScreenChoice {
            screen_id: Some(id.into()),
            label: id.into(),
            checked,
        };
        let model = MenuModel {
            items: vec![
                choice("1", false),
                choice("2", true),
                MenuItem::Toggle {
                    id: "x",
                    label: "x".into(),
                    checked: true,
                },
                choice("3", true),
            ],
        };
        let checked: Vec<_> = render_items(&model).iter().map(|r| r.checked).collect();
        // 開關不受單選群組影響。
        assert_eq!(checked, vec![false, true, true, false]);
    }

    /// Review 問題 1 的回歸測試：模型換掉之後（例如開著選單時接上一台螢幕，後面的項目全部往後移一格），
    /// 舊選單的 tag 必須照**舊快照**解析成舊項目的事件，不能對到新模型同位置的別的項目。
    #[test]
    fn a_click_on_an_old_snapshot_resolves_against_that_snapshot() {
        let old = sample();
        let mut new = old.clone();
        new.items.insert(
            3,
            MenuItem::ScreenChoice {
                screen_id: Some("9".into()),
                label: "新螢幕".into(),
                checked: false,
            },
        );
        // 舊選單上的「LRCLIB」是 tag 6。
        let lrclib_tag = 6;
        assert_eq!(
            event_for_tag(&old, lrclib_tag),
            Some(MenuEvent::Toggled("lyrics.lrclib", true))
        );
        // 新模型同一個 tag 已經是別的項目（分隔線往後移到了這裡）——若拿新模型解析就會出錯。
        assert_ne!(
            event_for_tag(&new, lrclib_tag),
            event_for_tag(&old, lrclib_tag)
        );

        // 每一代保管自己的快照：換上新模型後，舊世代仍以舊快照解析。
        let mut gens = MenuGenerations::default();
        assert!(gens.replace(old.clone(), false).is_empty());
        assert!(gens.replace(new.clone(), false).is_empty());
        let retired = gens.last_retired().expect("old generation kept");
        assert_eq!(
            event_for_tag(retired, lrclib_tag),
            Some(MenuEvent::Toggled("lyrics.lrclib", true))
        );
        assert_eq!(gens.current(), Some(&new));
    }

    #[test]
    fn generations_keep_the_just_replaced_menu_until_the_next_replace() {
        let mut gens = MenuGenerations::default();
        assert!(gens.replace(1, false).is_empty());
        assert_eq!(gens.retired_len(), 0);
        // 換下的那一代（可能正在派送動作）保留到下一次 replace。
        assert!(gens.replace(2, false).is_empty());
        assert_eq!(gens.last_retired(), Some(&1));
        // 不忙時，更早的世代才被交回釋放；剛換下的 2 仍保留。
        assert_eq!(gens.replace(3, false), vec![1]);
        assert_eq!(gens.last_retired(), Some(&2));
        assert_eq!(gens.current(), Some(&3));
    }

    #[test]
    fn generations_release_nothing_while_busy() {
        let mut gens = MenuGenerations::default();
        gens.replace(1, false);
        gens.replace(2, false);
        // 選單開著或正在派送（例如在回呼裡連續 set_model 兩次）：全部保留。
        assert!(gens.replace(3, true).is_empty());
        assert!(gens.replace(4, true).is_empty());
        assert_eq!(gens.retired_len(), 3);
        // 閒下來後的第一次 replace 一次交回所有更早的世代。
        assert_eq!(gens.replace(5, false), vec![1, 2, 3]);
        assert_eq!(gens.last_retired(), Some(&4));
        assert_eq!(gens.take_all(), vec![4, 5]);
        assert_eq!(gens.current(), None);
        assert_eq!(gens.retired_len(), 0);
    }

    #[test]
    fn calls_off_the_main_thread_return_none() {
        #[cfg(target_os = "macos")]
        {
            if objc2::MainThreadMarker::new().is_some() {
                eprintln!("test runs on the main thread; skipping");
                return;
            }
            assert!(MenuBar::new(None).is_none());
        }
    }
}
