//! 逾時與併發合併（ADR-0001）。

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::future::{select, Either, FutureExt, Shared, WeakShared};

use crate::runtime::{lock, BoxFuture, Clock};

/// `with_timeout` 的逾時結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedOut;

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timed out")
    }
}

impl std::error::Error for TimedOut {}

/// 讓 `fut` 與 `clock.sleep(limit)` 賽跑；輸家直接被 drop（即取消），不需要 `fut` 配合。
///
/// 計時從第一次輪詢開始。兩者同一輪都就緒時 `fut` 優先（快的來源不會被逾時切掉）。
/// `limit` 可以是 `Duration::MAX`：那只是一個永遠不到期的 sleep。
pub async fn with_timeout<T>(
    clock: &dyn Clock,
    limit: Duration,
    fut: impl Future<Output = T>,
) -> Result<T, TimedOut> {
    let fut = std::pin::pin!(fut);
    match select(fut, clock.sleep(limit)).await {
        Either::Left((value, _sleep)) => Ok(value),
        Either::Right(((), _fut)) => Err(TimedOut),
    }
}

struct Entry<V> {
    id: u64,
    run: WeakShared<BoxFuture<'static, V>>,
    waiters: usize,
}

struct Flights<K, V> {
    next_id: u64,
    map: HashMap<K, Entry<V>>,
}

type FlightsRef<K, V> = Arc<Mutex<Flights<K, V>>>;

/// 同 key 的併發呼叫共用一次執行（對應 Swift `LyricsRepository` 的 flights）。
///
/// - 每個呼叫者拿到一個等待 future；所有等待者共用一個 `Shared` 內部 future，表中只存 `WeakShared`。
/// - 內部 future 跑完時自己把表項移除，之後同 key 的呼叫會重新執行 `make`。
/// - 最後一個等待者被 drop 時表項移除、最後一個強參考消失，內部 future 隨之被 drop（取消），
///   它內部尚未執行的副作用（例如寫快取）永遠不會發生。
pub struct Coalescer<K, V: Clone> {
    flights: FlightsRef<K, V>,
}

impl<K, V> Coalescer<K, V>
where
    K: Hash + Eq + Clone + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            flights: Arc::new(Mutex::new(Flights {
                next_id: 0,
                map: HashMap::new(),
            })),
        }
    }

    /// 加入 `key` 正在進行的執行，沒有的話以 `make()` 開一個新的。登記在呼叫當下完成（不必等到輪詢）。
    /// `make` 在鎖外呼叫；多執行緒同時搶同一個新 key 時，輸家的 `make()` 結果會在未輪詢前被丟棄。
    pub fn run(
        &self,
        key: K,
        make: impl FnOnce() -> BoxFuture<'static, V>,
    ) -> impl Future<Output = V> + Send {
        if let Some(waiter) = self.join(&key) {
            return waiter;
        }

        let id = {
            let mut flights = lock(&self.flights);
            let id = flights.next_id;
            flights.next_id += 1;
            id
        };
        let inner = make();
        let weak_flights = Arc::downgrade(&self.flights);
        let done_key = key.clone();
        let wrapped: BoxFuture<'static, V> = Box::pin(async move {
            let value = inner.await;
            remove_if_current(&weak_flights, &done_key, id);
            value
        });
        let shared = wrapped.shared();

        let mut flights = lock(&self.flights);
        if let Some(waiter) = Self::join_locked(&self.flights, &mut flights, &key) {
            // 另一個執行緒在鎖外期間先登記了；加入它，丟掉自己還沒跑過的 future。
            drop(flights);
            drop(shared);
            return waiter;
        }
        let weak = shared
            .downgrade()
            .expect("剛建立且未輪詢的 Shared 不可能已完成");
        flights.map.insert(
            key.clone(),
            Entry {
                id,
                run: weak,
                waiters: 1,
            },
        );
        drop(flights);
        Waiter {
            shared,
            _guard: WaiterGuard {
                flights: Arc::downgrade(&self.flights),
                key: Some(key),
                id,
            },
        }
    }

    /// 目前等待 `key` 的呼叫者數量（測試用）。
    pub fn waiters(&self, key: &K) -> usize {
        lock(&self.flights)
            .map
            .get(key)
            .filter(|e| e.run.upgrade().is_some())
            .map_or(0, |e| e.waiters)
    }

    fn join(&self, key: &K) -> Option<Waiter<K, V>> {
        let mut flights = lock(&self.flights);
        Self::join_locked(&self.flights, &mut flights, key)
    }

    fn join_locked(
        this: &FlightsRef<K, V>,
        flights: &mut Flights<K, V>,
        key: &K,
    ) -> Option<Waiter<K, V>> {
        let entry = flights.map.get_mut(key)?;
        // 強參考全沒了的 WeakShared 升級失敗：視同沒有進行中的執行。
        // （正常情況下最後一個等待者 drop 時表項已移除，這裡是防線。）
        let shared = entry.run.upgrade()?;
        entry.waiters += 1;
        Some(Waiter {
            shared,
            _guard: WaiterGuard {
                flights: Arc::downgrade(this),
                key: Some(key.clone()),
                id: entry.id,
            },
        })
    }
}

impl<K, V> Default for Coalescer<K, V>
where
    K: Hash + Eq + Clone + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// 只在表項仍是 `id` 那一次執行時移除；移出的值在鎖外 drop。
fn remove_if_current<K: Hash + Eq, V>(flights: &Weak<Mutex<Flights<K, V>>>, key: &K, id: u64) {
    let Some(flights) = flights.upgrade() else {
        return;
    };
    let removed = {
        let mut flights = lock(&flights);
        match flights.map.get(key) {
            Some(e) if e.id == id => flights.map.remove(key),
            _ => None,
        }
    };
    drop(removed);
}

struct Waiter<K: Hash + Eq, V: Clone> {
    shared: Shared<BoxFuture<'static, V>>,
    _guard: WaiterGuard<K, V>,
}

// 不做結構性 pin：`Shared` 本身是 Unpin，guard 從不被 pin 住使用。
impl<K: Hash + Eq, V: Clone> Unpin for Waiter<K, V> {}

impl<K: Hash + Eq, V: Clone> Future for Waiter<K, V> {
    type Output = V;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<V> {
        self.shared.poll_unpin(cx)
    }
}

/// 等待者離開時把計數減一；歸零就移除表項。
///
/// drop 順序：`Waiter` 的欄位依宣告順序 drop，所以 `shared` 先 drop（若它是最後一個強參考，
/// 內部 future 就在那時、於鎖外被 drop），這個 guard 才接著減計數。中間的空檔裡表項還在，
/// 但它的 `WeakShared` 已升級失敗，`join_locked` 視同沒有進行中的執行；
/// 新開的執行會以新的 id 蓋掉表項，這個 guard 看到 id 不符就不動它。
struct WaiterGuard<K: Hash + Eq, V> {
    flights: Weak<Mutex<Flights<K, V>>>,
    key: Option<K>,
    id: u64,
}

impl<K: Hash + Eq, V> Drop for WaiterGuard<K, V> {
    fn drop(&mut self) {
        let (Some(flights), Some(key)) = (self.flights.upgrade(), self.key.take()) else {
            return;
        };
        let removed = {
            let mut flights = lock(&flights);
            match flights.map.get_mut(&key) {
                Some(e) if e.id == self.id => {
                    e.waiters -= 1;
                    if e.waiters == 0 {
                        flights.map.remove(&key)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        drop(removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{lock, BoxFuture, Clock, Spawner};
    use crate::testing::{FakeClock, LocalSpawner};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    type Slot<T> = Arc<Mutex<Option<T>>>;

    fn slot<T>() -> Slot<T> {
        Arc::new(Mutex::new(None))
    }

    fn take<T>(s: &Slot<T>) -> Option<T> {
        s.lock().unwrap().take()
    }

    /// drop 時立起旗標：用來證明內部 future 確實被 drop（取消）。
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn with_timeout_does_not_wait_for_a_future_that_ignores_cancellation() {
        let clock = Arc::new(FakeClock::new(1_700_000_000.0));
        let spawner = LocalSpawner::new();
        let out: Slot<Result<(), TimedOut>> = slot();

        // 被包的 future 永不完成，也不檢查任何取消訊號；drop 時立旗。
        let dropped = Arc::new(AtomicBool::new(false));
        let (c, o, d) = (clock.clone(), out.clone(), dropped.clone());
        let _task = spawner.spawn(Box::pin(async move {
            let inner = async move {
                let _guard = DropFlag(d);
                std::future::pending::<()>().await
            };
            let r = with_timeout(&*c, Duration::from_secs(5), inner).await;
            *o.lock().unwrap() = Some(r);
        }));

        spawner.run_until_stalled();
        assert!(take(&out).is_none());
        assert_eq!(clock.pending_sleeps(), 1);

        clock.advance(Duration::from_millis(4_999));
        spawner.run_until_stalled();
        assert!(take(&out).is_none());
        assert!(!dropped.load(Ordering::SeqCst));

        clock.advance(Duration::from_millis(1));
        spawner.run_until_stalled();
        assert_eq!(take(&out), Some(Err(TimedOut)));
        assert_eq!(clock.pending_sleeps(), 0);
        assert!(
            dropped.load(Ordering::SeqCst),
            "輸家（被包的 future）必須被 drop"
        );
    }

    #[test]
    fn with_timeout_returns_a_value_that_is_ready_at_the_deadline() {
        let clock = Arc::new(FakeClock::new(0.0));
        let spawner = LocalSpawner::new();
        let out: Slot<Result<u8, TimedOut>> = slot();

        let (c, o) = (clock.clone(), out.clone());
        let _task = spawner.spawn(Box::pin(async move {
            let inner = c.sleep(Duration::from_secs(5));
            let r = with_timeout(&*c, Duration::from_secs(5), async move {
                inner.await;
                9
            })
            .await;
            *o.lock().unwrap() = Some(r);
        }));

        spawner.run_until_stalled();
        clock.advance(Duration::from_secs(5));
        spawner.run_until_stalled();
        assert_eq!(take(&out), Some(Ok(9)));
    }

    #[test]
    fn coalescer_shares_one_run_between_concurrent_callers() {
        let clock = Arc::new(FakeClock::new(0.0));
        let spawner = LocalSpawner::new();
        let coalescer: Coalescer<String, u32> = Coalescer::new();
        let makes = Arc::new(AtomicUsize::new(0));
        let key = "k".to_string();

        let make = || {
            let (c, m) = (clock.clone(), makes.clone());
            move || -> BoxFuture<'static, u32> {
                m.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    c.sleep(Duration::from_secs(1)).await;
                    42
                })
            }
        };

        let (a, b) = (slot(), slot());
        let fa = coalescer.run(key.clone(), make());
        let fb = coalescer.run(key.clone(), make());
        assert_eq!(coalescer.waiters(&key), 2);
        assert_eq!(makes.load(Ordering::SeqCst), 1);

        let (oa, ob) = (a.clone(), b.clone());
        let _ta = spawner.spawn(Box::pin(
            async move { *oa.lock().unwrap() = Some(fa.await) },
        ));
        let _tb = spawner.spawn(Box::pin(
            async move { *ob.lock().unwrap() = Some(fb.await) },
        ));
        spawner.run_until_stalled();
        assert!(take(&a).is_none());

        clock.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(take(&a), Some(42));
        assert_eq!(take(&b), Some(42));
        assert_eq!(makes.load(Ordering::SeqCst), 1);
        assert_eq!(coalescer.waiters(&key), 0);

        // 完成後同 key 再呼叫會重新跑 make。
        let c = slot();
        let fc = coalescer.run(key.clone(), make());
        assert_eq!(makes.load(Ordering::SeqCst), 2);
        let oc = c.clone();
        let _tc = spawner.spawn(Box::pin(
            async move { *oc.lock().unwrap() = Some(fc.await) },
        ));
        spawner.run_until_stalled();
        clock.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(take(&c), Some(42));
    }

    #[test]
    fn coalescer_forgets_a_finished_run_before_every_waiter_has_observed_it() {
        use futures_util::FutureExt;
        let coalescer: Coalescer<u8, u8> = Coalescer::new();
        let makes = Arc::new(AtomicUsize::new(0));
        let make = || {
            let m = makes.clone();
            move || -> BoxFuture<'static, u8> {
                let n = m.fetch_add(1, Ordering::SeqCst) as u8;
                Box::pin(std::future::ready(n))
            }
        };
        let a = coalescer.run(1, make());
        let b = coalescer.run(1, make());
        assert_eq!(a.now_or_never(), Some(0));
        // `b` 尚未輪詢、仍持有 Shared 強參考；完成時表項必須已移除，否則 `c` 會加入舊的那次執行。
        let c = coalescer.run(1, make());
        assert_eq!(makes.load(Ordering::SeqCst), 2);
        assert_eq!(c.now_or_never(), Some(1));
        assert_eq!(b.now_or_never(), Some(0));
    }

    #[test]
    fn coalescer_cancels_the_run_when_every_waiter_is_dropped_and_never_reports_a_result() {
        let clock = Arc::new(FakeClock::new(0.0));
        let spawner = LocalSpawner::new();
        let coalescer: Coalescer<String, u32> = Coalescer::new();
        let makes = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(AtomicBool::new(false));
        let key = "k".to_string();

        let make = || {
            let (c, m, d, r) = (
                clock.clone(),
                makes.clone(),
                dropped.clone(),
                reported.clone(),
            );
            move || -> BoxFuture<'static, u32> {
                m.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    let _guard = DropFlag(d);
                    c.sleep(Duration::from_secs(1)).await;
                    // 對應 Swift：快取寫入在 future 內部，沒跑完就不會寫。
                    r.store(true, Ordering::SeqCst);
                    7
                })
            }
        };

        let fa = coalescer.run(key.clone(), make());
        let fb = coalescer.run(key.clone(), make());
        let ta = spawner.spawn(Box::pin(async move {
            fa.await;
        }));
        let tb = spawner.spawn(Box::pin(async move {
            fb.await;
        }));
        spawner.run_until_stalled();
        assert_eq!(coalescer.waiters(&key), 2);

        // 只剩一個呼叫者被 drop：內部 future 繼續跑。
        drop(ta);
        spawner.run_until_stalled();
        assert!(!dropped.load(Ordering::SeqCst));
        assert_eq!(coalescer.waiters(&key), 1);
        assert_eq!(clock.pending_sleeps(), 1);

        // 最後一個等待者也 drop：內部 future 被 drop，且永遠不回報結果。
        drop(tb);
        spawner.run_until_stalled();
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(coalescer.waiters(&key), 0);
        assert!(lock(&coalescer.flights).map.is_empty(), "表項必須移除");
        assert_eq!(clock.pending_sleeps(), 0);
        clock.advance(Duration::from_secs(5));
        spawner.run_until_stalled();
        assert!(!reported.load(Ordering::SeqCst));

        // 下一次呼叫重新執行。
        let _fc = coalescer.run(key.clone(), make());
        assert_eq!(makes.load(Ordering::SeqCst), 2);
        assert_eq!(coalescer.waiters(&key), 1);
    }

    #[test]
    fn coalescer_survivor_still_receives_the_value_after_another_waiter_is_dropped() {
        let clock = Arc::new(FakeClock::new(0.0));
        let spawner = LocalSpawner::new();
        let coalescer: Coalescer<u8, u32> = Coalescer::new();
        let makes = Arc::new(AtomicUsize::new(0));
        let make = || {
            let (c, m) = (clock.clone(), makes.clone());
            move || -> BoxFuture<'static, u32> {
                m.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    c.sleep(Duration::from_secs(1)).await;
                    7
                })
            }
        };

        let (fa, fb) = (coalescer.run(1, make()), coalescer.run(1, make()));
        let survivor = slot();
        let o = survivor.clone();
        let ta = spawner.spawn(Box::pin(async move {
            fa.await;
        }));
        let _tb = spawner.spawn(Box::pin(async move { *o.lock().unwrap() = Some(fb.await) }));
        spawner.run_until_stalled();
        drop(ta);
        spawner.run_until_stalled();
        assert_eq!(coalescer.waiters(&1), 1);

        clock.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(take(&survivor), Some(7));
        assert_eq!(makes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn coalescer_does_not_join_a_run_whose_waiter_futures_were_dropped_unpolled() {
        let coalescer: Coalescer<u8, u8> = Coalescer::new();
        let makes = Arc::new(AtomicUsize::new(0));
        let make = || {
            let m = makes.clone();
            move || -> BoxFuture<'static, u8> {
                m.fetch_add(1, Ordering::SeqCst);
                Box::pin(std::future::ready(1))
            }
        };
        drop(coalescer.run(1, make()));
        assert_eq!(coalescer.waiters(&1), 0);
        drop(coalescer.run(1, make()));
        assert_eq!(makes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn coalescer_drops_the_cancelled_run_outside_its_lock() {
        // 內部 future 的 drop 再呼叫同一個 Coalescer：若在持鎖時 drop 會死結（測試卡住）。
        struct Reenter(Arc<Coalescer<u8, u8>>);
        impl Drop for Reenter {
            fn drop(&mut self) {
                let _ = self.0.waiters(&1);
                drop(self.0.run(1, || Box::pin(std::future::ready(0))));
            }
        }
        let coalescer = Arc::new(Coalescer::new());
        let spawner = LocalSpawner::new();
        let co = coalescer.clone();
        let fut = coalescer.run(1, move || {
            Box::pin(async move {
                let _r = Reenter(co);
                std::future::pending::<u8>().await
            })
        });
        let task = spawner.spawn(Box::pin(async move {
            fut.await;
        }));
        spawner.run_until_stalled();
        drop(task);
        spawner.run_until_stalled();
        assert_eq!(coalescer.waiters(&1), 0);
    }

    #[test]
    fn coalescer_keys_are_independent() {
        let coalescer: Coalescer<u8, u8> = Coalescer::new();
        let _a = coalescer.run(1, || Box::pin(std::future::pending()));
        let _b = coalescer.run(2, || Box::pin(std::future::pending()));
        assert_eq!(coalescer.waiters(&1), 1);
        assert_eq!(coalescer.waiters(&2), 1);
    }
}
