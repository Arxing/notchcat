//! 真實時鐘（`Clock` 的牆上時間與執行緒計時器實作）。
//!
//! 這是 R1 示範用的暫時實作：`sleep` 每次起一條 `std::thread`。Task 25 會以 GPUI 的計時器取代它
//! （外殼已有執行器與計時器，不需要為每個計時另開執行緒）。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_channel::oneshot;
use musicat_core::runtime::{BoxFuture, Clock};

/// 牆上時間加每次 `sleep` 一條睡眠執行緒的 [`Clock`]。
#[derive(Debug, Clone, Copy, Default)]
pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> f64 {
        // 系統時鐘早於 epoch 時退化為 0，不 panic。
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }

    /// 睡眠執行緒到點後透過 oneshot 喚醒 future。drop 這個 future 不會取消執行緒：
    /// 它會睡完、送出（接收端已不在，送出失敗被忽略）然後結束；除了這條睡眠中的執行緒外不洩漏任何東西。
    /// 無法起執行緒時（資源耗盡）退化為立刻完成，因為 `Clock::sleep` 無法回報錯誤，
    /// 而逾時賽跑寧可早到期也不要永遠卡住。
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let (tx, rx) = oneshot::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("musicat-sleep".into())
            .spawn(move || {
                std::thread::sleep(d);
                let _ = tx.send(());
            });
        if spawned.is_err() {
            return Box::pin(std::future::ready(()));
        }
        // 送出端被 drop（執行緒 panic）也視為到期。
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn now_is_within_a_second_of_system_time() {
        let expected = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert!((RealClock.now() - expected).abs() < 1.0);
    }

    #[test]
    fn sleep_resolves_after_at_least_the_requested_duration() {
        let start = Instant::now();
        futures_executor::block_on(RealClock.sleep(Duration::from_millis(50)));
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(45), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    }

    #[test]
    fn dropping_an_unawaited_sleep_does_not_panic() {
        let fut = RealClock.sleep(Duration::from_millis(20));
        drop(fut);
        // 睡眠執行緒醒來時接收端已不在；等它醒來送出（失敗被忽略）而不 panic。
        std::thread::sleep(Duration::from_millis(80));
        // 一個被 poll 過一次再 drop 的也一樣。
        let mut fut = RealClock.sleep(Duration::from_millis(20));
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        assert!(std::future::Future::poll(fut.as_mut(), &mut cx).is_pending());
        drop(fut);
        std::thread::sleep(Duration::from_millis(80));
    }
}
