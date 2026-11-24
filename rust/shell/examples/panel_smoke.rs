//! GPUI 面板煙霧測試（Task 21）：用 `musicat-platform-macos::panel` 的繞路把一個 GPUI 視窗變成瀏海面板。
//!
//! 會在選定螢幕（`choose_screen`：有瀏海的優先）頂端顯示一條黑色 Preview 長條，左翼寫 `MusiCat`、
//! 右翼寫 `現正播放`（確認中文字可正常渲染），把螢幕、幾何與 `describe_panel` 印到 stdout，
//! 預設 8 秒後自動結束，不需要任何輸入。`MUSICAT_SMOKE_SECS=60` 可延長（手動驗證用，見 HANDOFF-rust.md）。
//!
//! 點擊穿透：每 16 ms 讀一次 `mouse_location()`，游標不在 Preview 長條上時讓面板忽略滑鼠，
//! 點擊落到下層視窗（與 spike 相同的做法）。
//!
//! 執行：`cd rust && cargo run -p musicat-shell --example panel_smoke`
//! 只在 macOS 有效；其他平台印一行訊息就結束。

#[cfg(target_os = "macos")]
fn main() {
    smoke::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("panel_smoke: macOS only");
}

#[cfg(target_os = "macos")]
mod smoke {
    use std::time::{Duration, Instant};

    use gpui::{
        div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use musicat_core::shell_logic::{
        choose_screen, container_frame, geometry_for, mode_size, NotchGeometry, Point, Rect,
        ShellMode, VirtualNotch, PREVIEW_CORNER_RADIUS,
    };
    use musicat_platform_macos::panel::{self, NativeWindow};
    use musicat_platform_macos::screens::list_screens;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const POLL: Duration = Duration::from_millis(16);

    struct Smoke {
        geometry: NotchGeometry,
        container: Rect,
        handle: Option<NativeWindow>,
        ignoring_mouse: Option<bool>,
        t0: Instant,
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

        /// 點擊穿透：游標在 Preview 長條上才接收滑鼠。
        fn poll(&mut self) {
            let p: Point = panel::mouse_location();
            // 游標貼在螢幕最上緣時 y == top；Rect::contains 含上緣，所以不必特別處理。
            let ignore = !self.preview_global().contains(p);
            if self.ignoring_mouse != Some(ignore) {
                let Some(handle) = self.handle else {
                    return;
                };
                panel::set_ignores_mouse_events(handle, ignore);
                self.ignoring_mouse = Some(ignore);
                println!(
                    "[+{:>5}ms] mouse=({:.1}, {:.1}) click-through={}",
                    self.t0.elapsed().as_millis(),
                    p.x,
                    p.y,
                    if ignore { "ON" } else { "OFF" }
                );
            }
        }
    }

    impl Render for Smoke {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let preview = mode_size(ShellMode::Preview, &self.geometry, 0.0);
            let wing = ((preview.w - self.geometry.notch.w) / 2.0) as f32;
            let h = preview.h as f32;
            let radius = px(PREVIEW_CORNER_RADIUS as f32);
            let label = |text: &'static str| {
                div()
                    .w(px(wing))
                    .h(px(h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(rgb(0xffffff))
                    .child(text)
            };
            let shape = div()
                .w(px(preview.w as f32))
                .h(px(h))
                .bg(rgb(0x000000))
                // 上方兩角直角，下方圓角（spec 第 5.3 節）。
                .rounded_bl(radius)
                .rounded_br(radius)
                .flex()
                .flex_row()
                .justify_between()
                .child(label("MusiCat"))
                .child(label("現正播放"));
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
            .unwrap_or(8);
        let app = Application::new();
        panel::patch_gpui_panel_class();

        app.run(move |cx: &mut App| {
            // GPUI 在完成啟動時把啟動模式設成 Regular，所以要在 run 的回呼裡、開視窗前改回來。
            panel::set_accessory_activation_policy();

            let screens = list_screens();
            for s in &screens {
                println!("{} screen: {s:?}", stamp(t0));
            }
            let Some(screen) = choose_screen(&screens, None) else {
                println!("{} no screens; quitting", stamp(t0));
                cx.quit();
                return;
            };
            let geometry = geometry_for(screen, VirtualNotch::default());
            let container = container_frame(&geometry);
            println!(
                "{} chosen screen id={} name={:?} hardware_notch={:?}",
                stamp(t0),
                screen.id,
                screen.name,
                screen.hardware_notch
            );
            println!("{} geometry: {geometry:?}", stamp(t0));
            println!(
                "{} container_frame (global, top-left): {container:?}",
                stamp(t0)
            );

            // GPUI 的初始位置只是大概（以顯示器為準的座標）；真正的外框由 configure_panel 設定。
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

            let opened = cx.open_window(options, move |window, cx| {
                let handle = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                    // SAFETY: `ns_view` 是 GPUI 剛建立、此刻正在回呼中的視窗的 NSView，所以現在是存活的。
                    // 把手只存在 `Smoke` 裡，而 `Smoke` 是這個視窗的根 view：視窗關閉時它跟著被釋放，
                    // 輪詢工作的 `this.update` 隨即失敗並結束，之後不會再使用把手。範例從不主動關視窗，
                    // 視窗一直開到 `cx.quit()` 結束行程。所有使用都在 GPUI 的主執行緒上。
                    Ok(RawWindowHandle::AppKit(h)) => {
                        Some(unsafe { NativeWindow::from_ns_view(h.ns_view.as_ptr()) })
                    }
                    other => {
                        println!(
                            "{} expected an AppKit window handle, got {other:?}; quitting",
                            stamp(t0)
                        );
                        cx.quit();
                        None
                    }
                };
                if let Some(handle) = handle {
                    panel::configure_panel(handle, container);
                    println!("{} panel: {}", stamp(t0), panel::describe_panel(handle));
                }
                println!(
                    "{} foreground_is_fullscreen={} (pending hardware check)",
                    stamp(t0),
                    panel::foreground_is_fullscreen()
                );

                cx.new(|cx: &mut Context<Smoke>| {
                    cx.spawn(async move |this, cx| loop {
                        cx.background_executor().timer(POLL).await;
                        if this.update(cx, |s, _| s.poll()).is_err() {
                            break;
                        }
                    })
                    .detach();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let _ = this.update(cx, |s, _| {
                            if let Some(handle) = s.handle {
                                println!(
                                    "{} panel@1s: {}",
                                    stamp(t0),
                                    panel::describe_panel(handle)
                                )
                            }
                        });
                    })
                    .detach();
                    Smoke {
                        geometry,
                        container,
                        handle,
                        ignoring_mouse: None,
                        t0,
                    }
                })
            });
            if let Err(e) = opened {
                println!("{} open_window failed: {e:?}", stamp(t0));
                cx.quit();
                return;
            }

            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                println!("{} {secs}s elapsed; quitting", stamp(t0));
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
