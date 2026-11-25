//! 換螢幕的回歸檢查：面板開啟或移動後，GPUI 自己認定的視窗大小與縮放比例要跟原生視窗一致。
//!
//! 背景：GPUI 0.2.2 靠原生回呼（`setFrameSize:`、`viewDidChangeBackingProperties`）更新視窗大小與縮放比例，
//! 回呼裡用 `try_borrow_mut` 借用 `App`；若是在 `cx.update` 之中改外框，回呼同步發生、借不到、靜默丟掉，
//! GPUI 就一直用舊的大小與縮放比例排版（接投影機、換到非 Retina 螢幕時整個跑版）。
//!
//! 步驟：開面板 → 移到較寬的瀏海（單螢幕也能抓到大小沒更新）→ 隱藏中移動再顯示 → 依序移到每個螢幕 →
//! 在每個螢幕上直接開一個面板。縮放比例只有在接著不同倍率的螢幕（例如 Retina 筆電加一般螢幕）時才測得出來。
//! 失敗時以狀態碼 1 結束。需要視窗伺服器的工作階段，面板會在各螢幕頂端出現幾秒。
//!
//! 執行：`cd rust && cargo run -p musicat-shell --example move_check`
//! 只在 macOS 有效；其他平台印一行訊息就結束。

#[cfg(target_os = "macos")]
fn main() {
    check::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("move_check: macOS only");
}

#[cfg(target_os = "macos")]
mod check {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use gpui::{div, prelude::*, AnyElement, App, Application, AsyncApp, Window};
    use musicat_core::shell_logic::{
        choose_screen, container_size, geometry_for, NotchGeometry, Size, VirtualNotch,
    };
    use musicat_platform_macos::panel::{self, NativeWindow};
    use musicat_platform_macos::screens::list_screens;
    use musicat_shell::{ShellContent, ShellWindow};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// 最後一次繪製時 GPUI 認定的視窗大小（點）與縮放比例，以及原生視窗實際的縮放比例。
    #[derive(Clone, Copy, Debug)]
    struct Seen {
        w: f64,
        h: f64,
        gpui_scale: f32,
        native_scale: Option<f64>,
    }

    #[derive(Default)]
    struct Probe {
        seen: Mutex<Option<Seen>>,
    }

    fn native_scale(window: &Window) -> Option<f64> {
        let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        // SAFETY: 正在繪製的視窗，NSView 存活；把手只在這個主執行緒呼叫裡使用。
        let native = unsafe { NativeWindow::from_ns_view(h.ns_view.as_ptr()) };
        panel::panel_backing_scale(native)
    }

    impl ShellContent for Probe {
        fn preview(&self, window: &mut Window, _cx: &mut App) -> AnyElement {
            let size = window.viewport_size();
            *self.seen.lock().unwrap() = Some(Seen {
                w: f64::from(f32::from(size.width)),
                h: f64::from(f32::from(size.height)),
                gpui_scale: window.scale_factor(),
                native_scale: native_scale(window),
            });
            div().into_any_element()
        }

        fn expanded(&self, _window: &mut Window, _cx: &mut App) -> AnyElement {
            div().into_any_element()
        }

        fn preview_extra_height(&self) -> f64 {
            0.0
        }

        fn on_expand(&self) {}
    }

    /// 取走最後一次繪製的紀錄來比對（取走：每一步都必須重新畫過，舊紀錄不算數）。
    fn report(step: &str, probe: &Probe, geometry: &NotchGeometry) -> bool {
        let want = container_size(geometry);
        let seen = probe.seen.lock().unwrap().take();
        let ok = seen.is_some_and(|s| {
            (s.w - want.w).abs() < 1.0
                && (s.h - want.h).abs() < 1.0
                && s.native_scale
                    .is_some_and(|n| (n - f64::from(s.gpui_scale)).abs() < 0.01)
        });
        println!(
            "{} {step}: window should be {} x {}; {seen:?}",
            if ok { "ok  " } else { "FAIL" },
            want.w,
            want.h
        );
        ok
    }

    async fn wait(cx: &AsyncApp) {
        cx.background_executor()
            .timer(Duration::from_millis(1200))
            .await;
    }

    fn open(cx: &mut App, geometry: NotchGeometry, probe: &Arc<Probe>) -> ShellWindow {
        match ShellWindow::open(cx, geometry, probe.clone()) {
            Ok(shell) => shell,
            Err(e) => {
                println!("move_check: could not open the panel: {e}");
                std::process::exit(1);
            }
        }
    }

    pub fn run() {
        let app = Application::new();
        panel::patch_gpui_panel_class();
        app.run(|cx| {
            panel::set_accessory_activation_policy();
            let screens = list_screens();
            let Some(home) = choose_screen(&screens, None) else {
                println!("move_check: no screen");
                std::process::exit(1);
            };
            let first = geometry_for(home, VirtualNotch::default());
            let wider = NotchGeometry {
                notch: Size {
                    w: first.notch.w + 120.0,
                    h: first.notch.h,
                },
                ..first
            };
            let probe = Arc::new(Probe::default());
            let shell = open(cx, first, &probe);
            cx.spawn(async move |cx| {
                let mut ok = true;
                wait(cx).await;
                ok &= report("after open", &probe, &first);

                // 與 app 相同：在 `cx.update` 之中移動面板。
                let _ = cx.update(|_| shell.reposition(wider));
                wait(cx).await;
                ok &= report("after moving to a wider notch", &probe, &wider);

                let _ = cx.update(|_| shell.set_visible(false));
                wait(cx).await;
                let _ = cx.update(|_| shell.reposition(first));
                wait(cx).await;
                probe.seen.lock().unwrap().take();
                let _ = cx.update(|_| shell.set_visible(true));
                wait(cx).await;
                ok &= report("after moving while hidden", &probe, &first);

                for screen in &screens {
                    let geometry = geometry_for(screen, VirtualNotch::default());
                    // 先移到較寬的幾何，確保每一步都真的有變化、需要重畫。
                    let _ = cx.update(|_| shell.reposition(wider));
                    wait(cx).await;
                    probe.seen.lock().unwrap().take();
                    let _ = cx.update(|_| shell.reposition(geometry));
                    wait(cx).await;
                    ok &= report(&format!("moved to {}", screen.name), &probe, &geometry);
                }

                let _ = cx.update(|_| drop(shell));
                wait(cx).await;
                for screen in &screens {
                    let geometry = geometry_for(screen, VirtualNotch::default());
                    probe.seen.lock().unwrap().take();
                    let Ok(shell) = cx.update(|cx| open(cx, geometry, &probe)) else {
                        std::process::exit(1);
                    };
                    wait(cx).await;
                    ok &= report(&format!("opened on {}", screen.name), &probe, &geometry);
                    let _ = cx.update(|_| drop(shell));
                    wait(cx).await;
                }

                println!("{} screen(s) connected", screens.len());
                std::process::exit(if ok { 0 } else { 1 });
            })
            .detach();
        });
    }
}
