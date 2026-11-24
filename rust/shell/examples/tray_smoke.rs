//! 選單列圖示煙霧測試（Task 22）：同一個行程裡，accessory 啟動模式 + `patch_gpui_panel_class` +
//! GPUI 瀏海面板 + `musicat-platform-macos::tray::MenuBar` 是否能共存。
//!
//! 時間表（不需要任何輸入；`MUSICAT_SMOKE_SECS` 可延長總長，手動驗證用，見 HANDOFF-rust.md）：
//!
//! - 0 s：建立面板（黑色 Preview 長條）與選單列圖示（文字 `♪`），印出 `describe_panel` 與 `MenuBar::describe`。
//! - 1 s：再印一次兩者（確認層級 25、`canBecomeKey=false`、圖示可見）。
//! - 2 s：以程式觸發「LRCLIB」開關（`NSMenu performActionForItemAtIndex:`，走 AppKit 的 target/action）。
//!   **回呼裡直接 `set_model`**（同一個 `MenuBar`、在 AppKit 派送這個選單動作的當下）換成「沒有螢幕項目」的
//!   位移模型，證明在回呼裡換選單不會崩潰；事件也經通道交給 GPUI 前景工作，更新狀態後再 `set_model` 一次。
//! - 2.6 s：過期選單：對保留中的舊選單（位移模型那一份）觸發它的「LRCLIB」位置；這個位置在目前的模型裡是別的項目，
//!   事件必須仍是 `Toggled("lyrics.lrclib", …)`（每份選單以自己的模型快照解析）。
//! - 3 s：以程式觸發「自動」螢幕選項。
//! - 4 s：`MenuBar::open_menu()`（排到 run loop 下一輪才對圖示按鈕 `performClick:`，與使用者點擊同一路徑），
//!   0.5 s 後印出 `menuOpen`（NSMenuDelegate），1.5 s 後由 GPUI 計時器 `close_menu()`；印出這 1.5 s 內面板
//!   輪詢（16 ms）跑了幾次，證明選單開著時 GPUI 前景工作照常執行。
//! - 總長 − 1 s（預設 7 s）：以程式觸發「結束」，收到 `MenuEvent::Quit` 後移除圖示並 `cx.quit()`。
//!
//! 執行：`cd rust && cargo run -p musicat-shell --example tray_smoke`
//! （`MUSICAT_TRAY_ICON=/path/icon.png` 改用 PNG 圖示，會設成 template、高 18 點。）
//! 只在 macOS 有效；其他平台印一行訊息就結束。

#[cfg(target_os = "macos")]
fn main() {
    smoke::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("tray_smoke: macOS only");
}

#[cfg(target_os = "macos")]
mod smoke {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use gpui::{
        div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use musicat_core::menu::{MenuEvent, MenuItem, MenuModel};
    use musicat_core::shell_logic::{
        choose_screen, container_frame, geometry_for, mode_size, NotchGeometry, Point, Rect,
        ScreenInfo, ShellMode, VirtualNotch, PREVIEW_CORNER_RADIUS,
    };
    use musicat_platform_macos::panel::{self, NativeWindow};
    use musicat_platform_macos::screens::list_screens;
    use musicat_platform_macos::tray::{is_main_thread, MenuBar};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const POLL: Duration = Duration::from_millis(16);

    thread_local! {
        /// 讓（必須是 `Send + Sync` 的）選單回呼在主執行緒上拿到同一個 `MenuBar`，以便在回呼裡呼叫 `set_model`。
        static IN_CALLBACK_BAR: RefCell<Option<Rc<RefCell<Option<MenuBar>>>>> = const { RefCell::new(None) };
        static IN_CALLBACK_DONE: Cell<bool> = const { Cell::new(false) };
    }

    const MODE_NOTCH: &str = "mode.notch";
    const LYRICS_LRCLIB: &str = "lyrics.lrclib";
    const LYRICS_NETEASE: &str = "lyrics.netease";
    const PERL_HOST: &str = "macos.perl_host";
    const LYRIC_STRIP: &str = "lyrics.strip_visible";

    /// 範例自己的「設定」（Task 25 由 app 的 Config 取代）。
    #[derive(Clone, Debug)]
    struct Settings {
        lrclib: bool,
        netease: bool,
        perl_host: bool,
        strip: bool,
        screen: Option<String>,
    }

    fn build_model(s: &Settings, screens: &[ScreenInfo]) -> MenuModel {
        let toggle = |id, label: &str, checked| MenuItem::Toggle {
            id,
            label: label.into(),
            checked,
        };
        let mut items = vec![toggle(MODE_NOTCH, "瀏海式", true), MenuItem::Separator];
        items.push(MenuItem::ScreenChoice {
            screen_id: None,
            label: "自動".into(),
            checked: s.screen.is_none(),
        });
        for screen in screens {
            items.push(MenuItem::ScreenChoice {
                screen_id: Some(screen.id.clone()),
                label: screen.name.clone(),
                checked: s.screen.as_deref() == Some(screen.id.as_str()),
            });
        }
        items.extend([
            MenuItem::Separator,
            toggle(LYRICS_LRCLIB, "LRCLIB", s.lrclib),
            toggle(LYRICS_NETEASE, "網易雲", s.netease),
            toggle(PERL_HOST, "macOS perl 宿主", s.perl_host),
            toggle(LYRIC_STRIP, "顯示歌詞條", s.strip),
            MenuItem::Separator,
            MenuItem::Quit,
        ]);
        MenuModel { items }
    }

    fn index_of(model: &MenuModel, pred: impl Fn(&MenuItem) -> bool) -> usize {
        model.items.iter().position(pred).expect("item in model")
    }

    struct Smoke {
        geometry: NotchGeometry,
        container: Rect,
        handle: Option<NativeWindow>,
        ignoring_mouse: Option<bool>,
        t0: Instant,
        polls: Rc<Cell<u64>>,
        bar: Rc<RefCell<Option<MenuBar>>>,
        events: mpsc::Receiver<MenuEvent>,
        settings: Settings,
        screens: Vec<ScreenInfo>,
    }

    impl Smoke {
        fn preview_global(&self) -> Rect {
            let s = mode_size(ShellMode::Preview, &self.geometry, 0.0);
            Rect {
                x: self.container.x + (self.container.w - s.w) / 2.0,
                y: self.container.y,
                w: s.w,
                h: s.h,
            }
        }

        fn poll(&mut self, cx: &mut Context<Self>) {
            self.polls.set(self.polls.get() + 1);
            // 點擊穿透（與 panel_smoke 相同）：游標在 Preview 長條上才接收滑鼠。
            let p: Point = panel::mouse_location();
            let ignore = !self.preview_global().contains(p);
            if self.ignoring_mouse != Some(ignore) {
                if let Some(handle) = self.handle {
                    panel::set_ignores_mouse_events(handle, ignore);
                    self.ignoring_mouse = Some(ignore);
                    println!(
                        "{} mouse=({:.1}, {:.1}) click-through={}",
                        stamp(self.t0),
                        p.x,
                        p.y,
                        if ignore { "ON" } else { "OFF" }
                    );
                }
            }
            // 選單事件：回呼只把事件送進通道，這裡（GPUI 前景工作）才處理。
            while let Ok(event) = self.events.try_recv() {
                println!("{} MenuEvent received: {event:?}", stamp(self.t0));
                match event {
                    MenuEvent::Toggled(LYRICS_LRCLIB, on) => self.settings.lrclib = on,
                    MenuEvent::Toggled(LYRICS_NETEASE, on) => self.settings.netease = on,
                    MenuEvent::Toggled(PERL_HOST, on) => self.settings.perl_host = on,
                    MenuEvent::Toggled(LYRIC_STRIP, on) => self.settings.strip = on,
                    MenuEvent::Toggled(other, on) => {
                        println!("{} (ignored toggle {other}={on})", stamp(self.t0))
                    }
                    MenuEvent::ScreenChosen(id) => self.settings.screen = id,
                    MenuEvent::Quit => {
                        // drop MenuBar → 從選單列移除圖示。
                        let removed = self.bar.borrow_mut().take().is_some();
                        println!(
                            "{} quitting (status item removed={removed}); panel: {}",
                            stamp(self.t0),
                            self.handle.map(panel::describe_panel).unwrap_or_default()
                        );
                        cx.quit();
                        return;
                    }
                }
                if let Some(bar) = self.bar.borrow().as_ref() {
                    bar.set_model(build_model(&self.settings, &self.screens));
                    println!(
                        "{} tray after set_model: {}",
                        stamp(self.t0),
                        bar.describe()
                    );
                }
            }
        }
    }

    impl Render for Smoke {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let preview = mode_size(ShellMode::Preview, &self.geometry, 0.0);
            let radius = px(PREVIEW_CORNER_RADIUS as f32);
            let shape = div()
                .w(px(preview.w as f32))
                .h(px(preview.h as f32))
                .bg(rgb(0x000000))
                .rounded_bl(radius)
                .rounded_br(radius)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(rgb(0xffffff))
                .child("MusiCat ♪ 選單列測試");
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .child(shape)
        }
    }

    fn stamp(t0: Instant) -> String {
        format!("[+{:>5}ms]", t0.elapsed().as_millis())
    }

    pub fn run() {
        let t0 = Instant::now();
        let secs: u64 = std::env::var("MUSICAT_SMOKE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8)
            .max(7);
        let app = Application::new();
        panel::patch_gpui_panel_class();

        app.run(move |cx: &mut App| {
            panel::set_accessory_activation_policy();

            let screens = list_screens();
            let Some(screen) = choose_screen(&screens, None) else {
                println!("{} no screens; quitting", stamp(t0));
                cx.quit();
                return;
            };
            let geometry = geometry_for(screen, VirtualNotch::default());
            let container = container_frame(&geometry);

            // 選單列圖示：必須在 NSApplication 啟動之後（這個回呼裡）、主執行緒上建立。
            let settings = Settings {
                lrclib: true,
                netease: true,
                perl_host: true,
                strip: true,
                screen: None,
            };
            let model = build_model(&settings, &screens);
            let lrclib_index = index_of(
                &model,
                |i| matches!(i, MenuItem::Toggle { id, .. } if *id == LYRICS_LRCLIB),
            );
            let auto_index = index_of(&model, |i| {
                matches!(
                    i,
                    MenuItem::ScreenChoice {
                        screen_id: None,
                        ..
                    }
                )
            });
            let quit_index = index_of(&model, |i| matches!(i, MenuItem::Quit));
            // 位移模型：沒有任何螢幕項目，所以「LRCLIB」以後的項目都往前移。
            let shifted = build_model(
                &Settings {
                    lrclib: false,
                    ..settings.clone()
                },
                &[],
            );
            let shifted_lrclib_index = index_of(
                &shifted,
                |i| matches!(i, MenuItem::Toggle { id, .. } if *id == LYRICS_LRCLIB),
            );
            let current_at_shifted_index = format!("{:?}", model.items[shifted_lrclib_index]);
            // 圖示：預設用文字 ♪；`MUSICAT_TRAY_ICON=<png 路徑>` 可測 PNG（template）路徑。
            let icon = std::env::var_os("MUSICAT_TRAY_ICON").and_then(|p| std::fs::read(p).ok());
            let Some(bar) = MenuBar::new(icon.as_deref()) else {
                println!("{} MenuBar::new failed; quitting", stamp(t0));
                cx.quit();
                return;
            };
            let (tx, rx) = mpsc::channel::<MenuEvent>();
            bar.on_event(Box::new(move |event| {
                println!("[callback on main thread={}] {event:?}", is_main_thread());
                if matches!(event, MenuEvent::Toggled(LYRICS_LRCLIB, _)) && !IN_CALLBACK_DONE.get() {
                    IN_CALLBACK_DONE.set(true);
                    IN_CALLBACK_BAR.with_borrow(|cell| {
                        let Some(cell) = cell else { return };
                        // 呼叫 perform_item 的那一端只持有不可變借用，這裡也只借不可變的。
                        if let Some(b) = cell.borrow().as_ref() {
                            b.set_model(shifted.clone());
                            println!(
                                "[callback] set_model(shifted) inside the callback; tray: {}",
                                b.describe()
                            );
                        }
                    });
                }
                let _ = tx.send(event);
            }));
            bar.set_model(model);
            println!("{} tray: {}", stamp(t0), bar.describe());
            let bar = Rc::new(RefCell::new(Some(bar)));
            IN_CALLBACK_BAR.set(Some(bar.clone()));
            let polls = Rc::new(Cell::new(0u64));

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: gpui::point(px(0.), px(0.)),
                    size: size(px(container.w as f32), px(container.h as f32)),
                })),
                titlebar: None,
                focus: false,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            };

            let smoke_bar = bar.clone();
            let smoke_polls = polls.clone();
            let opened = cx.open_window(options, move |window, cx| {
                let handle = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                    // SAFETY: 與 panel_smoke 相同：`ns_view` 是此刻正在回呼中的 GPUI 視窗的 NSView；
                    // 把手只存在根 view `Smoke` 裡，視窗關閉時跟著釋放；範例從不主動關視窗；全在主執行緒。
                    Ok(RawWindowHandle::AppKit(h)) => {
                        Some(unsafe { NativeWindow::from_ns_view(h.ns_view.as_ptr()) })
                    }
                    other => {
                        println!("{} unexpected window handle {other:?}", stamp(t0));
                        None
                    }
                };
                if let Some(handle) = handle {
                    panel::configure_panel(handle, container);
                    println!("{} panel: {}", stamp(t0), panel::describe_panel(handle));
                }
                cx.new(|cx: &mut Context<Smoke>| {
                    cx.spawn(async move |this, cx| loop {
                        cx.background_executor().timer(POLL).await;
                        if this.update(cx, |s, cx| s.poll(cx)).is_err() {
                            break;
                        }
                    })
                    .detach();
                    Smoke {
                        geometry,
                        container,
                        handle,
                        ignoring_mouse: None,
                        t0,
                        polls: smoke_polls,
                        bar: smoke_bar,
                        events: rx,
                        settings,
                        screens: screens.clone(),
                    }
                })
            });
            let window = match opened {
                Ok(w) => w,
                Err(e) => {
                    println!("{} open_window failed: {e:?}", stamp(t0));
                    cx.quit();
                    return;
                }
            };

            // 時間表。MenuBar 的方法直接在前景工作裡呼叫（不在任何 GPUI update 之內）：
            // open_menu 會跑巢狀的選單追蹤迴圈，期間其他前景工作仍要能借用 App。
            cx.spawn(async move |cx| {
                let executor = cx.background_executor().clone();
                let timer = |ms| executor.timer(Duration::from_millis(ms));
                timer(1000).await;
                let _ = window.update(cx, |s, _, _| {
                    if let Some(h) = s.handle {
                        println!("{} panel@1s: {}", stamp(t0), panel::describe_panel(h));
                    }
                });
                if let Some(b) = bar.borrow().as_ref() {
                    println!("{} tray@1s: {}", stamp(t0), b.describe());
                }

                timer(1000).await;
                println!("{} perform_item({lrclib_index}) [LRCLIB]", stamp(t0));
                let ok = bar.borrow().as_ref().map(|b| b.perform_item(lrclib_index));
                println!("{} perform_item returned {ok:?} (survived set_model inside the callback)", stamp(t0));

                timer(600).await;
                println!(
                    "{} perform_retired_item({shifted_lrclib_index}) [stale menu: LRCLIB there; current model has {current_at_shifted_index} at that index]",
                    stamp(t0)
                );
                let ok = bar
                    .borrow()
                    .as_ref()
                    .map(|b| b.perform_retired_item(shifted_lrclib_index));
                println!("{} perform_retired_item returned {ok:?}", stamp(t0));

                timer(400).await;
                println!("{} perform_item({auto_index}) [自動]", stamp(t0));
                let ok = bar.borrow().as_ref().map(|b| b.perform_item(auto_index));
                println!("{} perform_item returned {ok:?}", stamp(t0));

                timer(1000).await;
                // 打開選單（open_menu 排到 run loop 下一輪才點，不阻塞這裡），1.5 s 後由 GPUI 計時器關閉。
                // 若選單開著時 GPUI 前景工作停住，關閉就不會發生、輪詢次數也不會增加。
                let before = polls.get();
                println!("{} open_menu()", stamp(t0));
                if let Some(b) = bar.borrow().as_ref() {
                    b.open_menu();
                }
                timer(500).await;
                if let Some(b) = bar.borrow().as_ref() {
                    println!("{} menu open? tray: {}", stamp(t0), b.describe());
                }
                timer(1000).await;
                println!(
                    "{} close_menu() from a GPUI timer; panel polls in the last 1.5 s: {}",
                    stamp(t0),
                    polls.get() - before
                );
                if let Some(b) = bar.borrow().as_ref() {
                    b.close_menu();
                }
                timer(200).await;
                if let Some(b) = bar.borrow().as_ref() {
                    println!(
                        "{} after close_menu: menuOpen={}",
                        stamp(t0),
                        b.is_menu_open()
                    );
                }

                timer((secs * 1000).saturating_sub(5200 + 1000)).await;
                let _ = window.update(cx, |s, _, _| {
                    if let Some(h) = s.handle {
                        println!(
                            "{} panel before quit: {}",
                            stamp(t0),
                            panel::describe_panel(h)
                        );
                    }
                });
                println!("{} perform_item({quit_index}) [結束]", stamp(t0));
                let ok = bar.borrow().as_ref().map(|b| b.perform_item(quit_index));
                println!("{} perform_item returned {ok:?}", stamp(t0));

                // 保險：結束事件沒有送達時，2 秒後仍結束並標成失敗。
                timer(2000).await;
                println!("{} FAIL: Quit event was not handled; quitting", stamp(t0));
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
        println!("{} Application::run returned", stamp(t0));
    }
}
