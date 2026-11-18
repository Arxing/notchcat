//! 外殼示範（Task 23）：真正的 `ShellWindow`（固定容器、hover 狀態機、0.28 秒動畫、點擊穿透）配上佔位內容。
//!
//! 游標移到選定螢幕（有瀏海的優先，沒有就用 200 × 32 的虛擬瀏海）頂端中央的黑色長條上 → 展開；
//! 離開 150 ms 後收合。每次狀態轉移印一行（`expand`、`collapse`、`visible=..`），附 epoch 時間戳
//! （可與 `spikes/gpui-notch/tools/hover.swift` 的 STEP 時間對照）與啟動後經過的毫秒。
//!
//! 環境變數：
//! - `MUSICAT_DEMO_SECS`：幾秒後自動結束（預設 60）。
//! - `MUSICAT_DEMO_EXTRA`：Preview 額外高度（點，預設 0；外殼夾到 0…28）。
//! - `MUSICAT_DEMO_BLINK=1`：第 3 秒 `set_visible(false)`、第 4 秒 `set_visible(true)`（驗證隱藏／再顯示）。
//! - `MUSICAT_SHELL_TRACE=1`：外殼每畫一格印一行 `shell-frame`（量動畫長度用）。
//!
//! 執行：`cd rust && cargo run -p musicat-shell --example shell_demo`
//! 只在 macOS 有效；其他平台印一行訊息就結束。

#[cfg(target_os = "macos")]
fn main() {
    demo::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("shell_demo: macOS only");
}

#[cfg(target_os = "macos")]
mod demo {
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use gpui::{div, prelude::*, px, rgb, AnyElement, App, Application, Window};
    use musicat_core::shell_logic::{
        choose_screen, container_frame, geometry_for, mode_size, NotchGeometry, ShellMode,
        VirtualNotch,
    };
    use musicat_platform_macos::panel;
    use musicat_platform_macos::screens::list_screens;
    use musicat_shell::{ShellContent, ShellEvent, ShellWindow};

    fn stamp(t0: Instant) -> String {
        let d = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "[{}.{:03} +{:>6}ms]",
            d.as_secs(),
            d.subsec_millis(),
            t0.elapsed().as_millis()
        )
    }

    /// 佔位內容：Preview 是左右兩個色塊「翼」加中央的深色標籤膠囊（在實體瀏海後面時看不到）；Expanded 是標題列、三個色塊與幾行文字。
    struct Placeholder {
        geometry: NotchGeometry,
        extra: f64,
        t0: Instant,
    }

    impl ShellContent for Placeholder {
        fn preview(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
            let size = mode_size(ShellMode::Preview, &self.geometry, self.extra);
            let wing = ((size.w - self.geometry.notch.w) / 2.0) as f32;
            let notch_h = self.geometry.notch.h as f32;
            let square = |color: u32| div().size(px(14.)).rounded_sm().bg(rgb(color));
            let wing_box = |color: u32| {
                div()
                    .w(px(wing))
                    .h(px(notch_h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(square(color))
            };
            div()
                .size_full()
                .flex()
                .flex_row()
                .justify_between()
                .child(wing_box(0xff375f))
                .child(
                    div()
                        .flex_1()
                        .h(px(notch_h))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            // 內容不鋪滿槽（GPUI 只能矩形裁切，鋪滿會蓋到下方圓角外）：中央是一顆深色小膠囊。
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded_full()
                                .bg(rgb(0x2c2c2e))
                                .text_size(px(11.))
                                .text_color(rgb(0xd1d1d6))
                                .child("MusiCat"),
                        ),
                )
                .child(wing_box(0x30d158))
                .into_any_element()
        }

        fn expanded(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
            let block = |color: u32| div().flex_1().h(px(48.)).rounded_md().bg(rgb(color));
            div()
                .size_full()
                .px(px(24.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .text_color(rgb(0xffffff))
                .child(
                    div()
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .child("MusiCat"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(block(0xff375f))
                        .child(block(0xffd60a))
                        .child(block(0x0a84ff)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(0xd1d1d6))
                        .child("夜空中最亮的星 — 佔位歌詞")
                        .child("繁體／简体／日本語かな")
                        .child("Placeholder content (Task 24 brings the real views)"),
                )
                .into_any_element()
        }

        fn preview_extra_height(&self) -> f64 {
            self.extra
        }

        fn on_expand(&self) {
            // 真正的 app 在這裡呼叫 ModuleManager::reset_selection_to_active()。
        }

        fn on_shell_event(&self, event: ShellEvent) {
            match event {
                ShellEvent::Expanded => println!("{} expand", stamp(self.t0)),
                ShellEvent::Collapsed => println!("{} collapse", stamp(self.t0)),
                ShellEvent::Visible(v) => println!("{} visible={v}", stamp(self.t0)),
            }
        }
    }

    fn env_f64(name: &str) -> Option<f64> {
        std::env::var(name).ok().and_then(|s| s.parse().ok())
    }

    pub fn run() {
        let t0 = Instant::now();
        let secs = env_f64("MUSICAT_DEMO_SECS").unwrap_or(60.0).max(0.0);
        let extra = env_f64("MUSICAT_DEMO_EXTRA").unwrap_or(0.0);
        let blink = std::env::var("MUSICAT_DEMO_BLINK").as_deref() == Ok("1");
        println!("hover the top-centre of the built-in screen to expand");

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
            println!(
                "{} screen id={} name={:?} hardware_notch={:?} geometry={geometry:?} container={:?} pid={}",
                stamp(t0),
                screen.id,
                screen.name,
                screen.hardware_notch,
                container_frame(&geometry),
                std::process::id()
            );

            let content = Arc::new(Placeholder {
                geometry,
                extra,
                t0,
            });
            let shell = match ShellWindow::open(cx, geometry, content) {
                Ok(shell) => shell,
                Err(e) => {
                    println!("{} open failed: {e:?}; quitting", stamp(t0));
                    cx.quit();
                    return;
                }
            };

            cx.spawn(async move |cx| {
                let timer = |s: f64| cx.background_executor().timer(Duration::from_secs_f64(s));
                let mut elapsed = 0.0;
                if blink && secs > 4.0 {
                    timer(3.0).await;
                    shell.set_visible(false);
                    timer(1.0).await;
                    shell.set_visible(true);
                    elapsed = 4.0;
                }
                timer(secs - elapsed).await;
                println!("{} {secs}s elapsed; quitting", stamp(t0));
                drop(shell);
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
