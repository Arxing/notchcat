//! core 的注入式執行環境（[`Clock`]、[`Spawner`]）以 GPUI 的背景執行器實作。
//!
//! - [`GpuiClock`]：`now()` 是牆上時間（Unix epoch 秒）；`sleep(d)` 是 `BackgroundExecutor::timer(d)`，
//!   在 macOS 上是 GCD 的 `dispatch_after`，**不會為每次睡眠開執行緒**。drop 回傳的 future 即取消計時器。
//! - [`GpuiSpawner`]：`spawn` 把 future 交給 `BackgroundExecutor::spawn`，回傳的 GPUI `Task` 放進
//!   `DetachOnDrop` 守衛，守衛再被搬進 [`TaskHandle`] 的取消閉包。
//!   - drop 把手：取消閉包被呼叫，先把 `Task` 從守衛取出再 drop = 取消工作。
//!   - [`TaskHandle::detach`]：取消閉包**不被呼叫**就被 drop，這時守衛的 `Drop` 對 `Task` 呼叫
//!     `Task::detach`，future 跑到完成。若直接把 `Task` 搬進閉包（`move || drop(task)`），`detach` 時閉包被 drop，
//!     `Task` 跟著被 drop，工作反而被取消——這是守衛存在的原因（見 `TaskHandle::new` 的文件）。
//!
//! # 執行緒
//!
//! 兩者都在 **GPUI 的背景執行緒**（GCD 全域佇列）上執行 future，不在主執行緒。所以交給它們的東西都必須是
//! `Send`——core 的 `BoxFuture` 本來就要求 `Send`，型別系統會擋住。不得在這些 future 裡碰 GPUI 的 `App`、
//! 視窗或 AppKit 物件；要回到主執行緒請用 channel 加 `cx.spawn` 的前景工作（見 `MusicShellContent`）。
//! 阻塞式的工作（例如 `WindowTitleSource` 每次約 9 ms 的視窗列舉）放在這裡是可以的：它佔用的是背景執行緒。
//!
//! 兩者都以 `cx.background_executor().clone()` 建立（`App` 回呼裡）。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::BackgroundExecutor;
use musicat_core::runtime::{BoxFuture, Clock, Spawner, TaskHandle};

/// 牆上時間加 GPUI 計時器的 [`Clock`]。見模組文件。
#[derive(Clone)]
pub struct GpuiClock {
    executor: BackgroundExecutor,
}

impl GpuiClock {
    pub fn new(executor: BackgroundExecutor) -> Self {
        Self { executor }
    }
}

/// `SystemTime` → Unix epoch 秒；系統時鐘早於 epoch 時為 0（不 panic）。
pub fn unix_seconds(t: SystemTime) -> f64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl Clock for GpuiClock {
    fn now(&self) -> f64 {
        unix_seconds(SystemTime::now())
    }

    /// `d` 為 0 時立即完成。drop 回傳的 future 即 drop GPUI 的 `Task`，計時器被取消。
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        Box::pin(self.executor.timer(d))
    }
}

/// 在 GPUI 背景執行器上跑 future 的 [`Spawner`]。見模組文件。
#[derive(Clone)]
pub struct GpuiSpawner {
    executor: BackgroundExecutor,
}

impl GpuiSpawner {
    pub fn new(executor: BackgroundExecutor) -> Self {
        Self { executor }
    }
}

impl Spawner for GpuiSpawner {
    fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle {
        let task = self.executor.spawn(f);
        // 不能寫成 `TaskHandle::new(move || drop(task))`：`TaskHandle::detach` 是把取消閉包**不呼叫就 drop 掉**，
        // 而閉包被 drop 時它擁有的 Task 也被 drop——GPUI 會因此取消工作，detach 形同取消。
        // 所以 Task 放在「drop 時呼叫 `Task::detach`」的守衛裡；只有取消閉包被呼叫時才先取出 Task 明確 drop。
        let guard = DetachOnDrop(Some(task));
        TaskHandle::new(move || {
            let mut guard = guard;
            drop(guard.0.take());
        })
    }
}

/// 被 drop 時讓 GPUI 的 `Task` 繼續跑（`Task::detach`）；取消路徑會先把 `Task` 取走再 drop。
struct DetachOnDrop(Option<gpui::Task<()>>);

impl Drop for DetachOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    use async_task::Runnable;
    use gpui::{PlatformDispatcher, TaskLabel};

    /// 測試用的 GPUI 調度器：每個 runnable 一條執行緒（`dispatch_after` 先睡再跑）。
    /// 與正式的 GCD 調度器一樣在「背景執行緒」上跑，足以驗證取消與分離的語意。
    struct ThreadDispatcher;

    impl PlatformDispatcher for ThreadDispatcher {
        fn is_main_thread(&self) -> bool {
            false
        }
        fn dispatch(&self, runnable: Runnable, _label: Option<TaskLabel>) {
            std::thread::spawn(move || {
                runnable.run();
            });
        }
        fn dispatch_on_main_thread(&self, runnable: Runnable) {
            self.dispatch(runnable, None);
        }
        fn dispatch_after(&self, duration: Duration, runnable: Runnable) {
            std::thread::spawn(move || {
                std::thread::sleep(duration);
                runnable.run();
            });
        }
    }

    fn executor() -> BackgroundExecutor {
        BackgroundExecutor::new(Arc::new(ThreadDispatcher))
    }

    #[test]
    fn now_is_unix_seconds() {
        let clock = GpuiClock::new(executor());
        let expected = unix_seconds(SystemTime::now());
        assert!((clock.now() - expected).abs() < 1.0);
        assert!(clock.now() > 1.7e9, "after 2023");
        assert_eq!(unix_seconds(UNIX_EPOCH - Duration::from_secs(5)), 0.0);
    }

    #[test]
    fn sleep_completes_after_the_duration() {
        let clock = GpuiClock::new(executor());
        let start = Instant::now();
        futures_executor_block_on(clock.sleep(Duration::from_millis(60)));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(55), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        // 0 立即完成。
        futures_executor_block_on(clock.sleep(Duration::ZERO));
    }

    /// 一個在 `gate` 收到訊號後才遞增 `ran` 的工作。
    fn gated_job(
        ran: Arc<AtomicUsize>,
    ) -> (BoxFuture<'static, ()>, futures_channel::oneshot::Sender<()>) {
        let (tx, rx) = futures_channel::oneshot::channel::<()>();
        (
            Box::pin(async move {
                if rx.await.is_ok() {
                    ran.fetch_add(1, Ordering::SeqCst);
                }
            }),
            tx,
        )
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn spawned_work_runs_on_a_background_thread() {
        let spawner = GpuiSpawner::new(executor());
        let (tx, rx) = mpsc::channel();
        let main = std::thread::current().id();
        spawner
            .spawn(Box::pin(async move {
                let _ = tx.send(std::thread::current().id());
            }))
            .detach();
        let ran_on = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_ne!(ran_on, main);
    }

    #[test]
    fn dropping_the_handle_cancels_the_work() {
        let spawner = GpuiSpawner::new(executor());
        let ran = Arc::new(AtomicUsize::new(0));
        let (job, gate) = gated_job(ran.clone());
        let handle = spawner.spawn(job);
        drop(handle);
        // 工作已被取消：它的接收端隨之 drop，送出失敗。
        assert!(wait_until(|| gate.is_canceled()));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_detached_handle_lets_the_work_finish() {
        let spawner = GpuiSpawner::new(executor());
        let ran = Arc::new(AtomicUsize::new(0));
        let (job, gate) = gated_job(ran.clone());
        spawner.spawn(job).detach();
        std::thread::sleep(Duration::from_millis(30));
        assert!(!gate.is_canceled(), "detach must not cancel");
        gate.send(()).unwrap();
        assert!(wait_until(|| ran.load(Ordering::SeqCst) == 1));
    }

    /// 計數的 waker：被喚醒幾次（= 計時器的完成回呼是否真的跑了）。
    struct CountingWaker(AtomicUsize);

    impl std::task::Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 輪詢一次 `sleep`（把 waker 登記給計時器），回傳 future 與 waker 的喚醒計數。
    fn polled_sleep(
        clock: &GpuiClock,
        d: Duration,
    ) -> (BoxFuture<'static, ()>, Arc<CountingWaker>) {
        use std::task::{Context, Poll, Waker};
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut fut = clock.sleep(d);
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
        (fut, counter)
    }

    #[test]
    fn dropping_a_sleep_never_wakes_its_waker() {
        let clock = GpuiClock::new(executor());
        // 對照組：不 drop 的計時器到期時確實喚醒 waker（否則下面的斷言恆成立、沒有意義）。
        let (live, live_counter) = polled_sleep(&clock, Duration::from_millis(20));
        assert!(wait_until(|| live_counter.0.load(Ordering::SeqCst) >= 1));
        drop(live);

        // 被 drop 的計時器：drop 當下 GPUI 的 Task 會（同步）通知一次等待者，這不是計時器到期；
        // 之後過了遠超過到期時間的 150 ms，喚醒次數不再增加，也就是到期的完成回呼沒有跑。
        let (fut, counter) = polled_sleep(&clock, Duration::from_millis(20));
        drop(fut);
        let after_drop = counter.0.load(Ordering::SeqCst);
        assert!(
            after_drop <= 1,
            "at most the cancellation notice: {after_drop}"
        );
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(counter.0.load(Ordering::SeqCst), after_drop);
    }

    /// 最小的 block_on（不引入執行器 crate）：以執行緒 park/unpark 當 waker。
    fn futures_executor_block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, Wake, Waker};
        struct ThreadWaker(std::thread::Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park();
        }
    }
}
