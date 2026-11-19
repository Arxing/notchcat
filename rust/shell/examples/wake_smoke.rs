//! 喚醒通知的煙霧測試（Task 25b）：`musicat_platform_macos::power::observe_wake` 在真的 AppKit run loop 裡
//! 收得到 `NSWorkspaceDidWakeNotification`，回呼在主執行緒上，drop 登記之後不再收到。
//!
//! **不會讓機器睡眠**：通知是程式自己用 `post_wake_for_test()` 發的（一次從主執行緒、一次從背景執行緒）。
//!
//! ```sh
//! cd rust && cargo run -p musicat-shell --example wake_smoke
//! ```
//!
//! 通過時印 `wake_smoke: PASS` 並以 0 結束；失敗印 `wake_smoke: FAIL …` 並以 1 結束。只在 macOS 有效。

#[cfg(target_os = "macos")]
fn main() {
    smoke::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("wake_smoke: macOS only");
}

#[cfg(target_os = "macos")]
mod smoke {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use gpui::{App, Application};
    use musicat_platform_macos::{panel, power, tray};

    static FIRED: AtomicUsize = AtomicUsize::new(0);
    static OFF_MAIN: AtomicBool = AtomicBool::new(false);

    fn fail(msg: &str) -> ! {
        println!("wake_smoke: FAIL {msg}");
        std::process::exit(1);
    }

    pub fn run() {
        Application::new().run(|cx: &mut App| {
            panel::set_accessory_activation_policy();

            let observer = power::observe_wake(Box::new(|| {
                if !tray::is_main_thread() {
                    OFF_MAIN.store(true, Ordering::SeqCst);
                }
                FIRED.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap_or_else(|| fail("observe_wake returned None on the main thread"));

            // 背景執行緒呼叫 observe_wake 必須拒絕（會印一行預期中的警告）。
            println!("wake_smoke: an off-main-thread observe_wake must be refused (expected warning follows)");
            let off_main_none = std::thread::spawn(|| power::observe_wake(Box::new(|| {})).is_none())
                .join()
                .unwrap_or(false);
            if !off_main_none {
                fail("observe_wake off the main thread did not return None");
            }

            power::post_wake_for_test();
            // 不 join：從別的執行緒發出時，NSNotificationCenter 會等主佇列上的回呼跑完才返回，
            // 主執行緒在這裡 join 就會互相等待。
            std::thread::spawn(power::post_wake_for_test);

            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let fired = FIRED.load(Ordering::SeqCst);
                println!("wake_smoke: callbacks after two posts = {fired}");
                if fired != 2 {
                    fail(&format!("expected 2 callbacks, got {fired}"));
                }
                if OFF_MAIN.load(Ordering::SeqCst) {
                    fail("a callback ran off the main thread");
                }

                drop(observer);
                power::post_wake_for_test();
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let after = FIRED.load(Ordering::SeqCst);
                println!("wake_smoke: callbacks after drop + post = {after}");
                if after != 2 {
                    fail("callback fired after the observer was dropped");
                }
                println!("wake_smoke: PASS");
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
