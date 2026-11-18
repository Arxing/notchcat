//! 注入式執行環境：時間（`Clock`）、背景工作（`Spawner`／`TaskHandle`）與同步通知（`Notifier`）。
//!
//! core 不依賴任何 async runtime；外殼把自己的計時器與執行器包成這些 trait 注入（ADR-0001）。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures_util::future::{AbortHandle, Abortable};

/// core 內所有跨邊界的 future 型別：`Send`，可交給任何執行緒的執行器。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 注入的時間來源。
pub trait Clock: Send + Sync {
    /// 目前牆上時間，Unix epoch 秒。
    fn now(&self) -> f64;
    /// 經過 `d` 後完成的 future；drop 即取消計時。
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}

/// 背景工作的把手：drop 即取消（對應 GPUI 的 `Task`）。要讓工作脫離把手繼續跑，呼叫 [`TaskHandle::detach`]。
#[must_use = "TaskHandle 被 drop 時工作就會被取消；要讓它繼續跑請呼叫 detach()"]
pub struct TaskHandle {
    cancel: Option<Box<dyn FnOnce() + Send>>,
}

impl TaskHandle {
    /// 以「取消時要做的事」建立把手。`Spawner` 實作可以把執行器自己的 task 物件 move 進來再 drop 掉。
    pub fn new(on_cancel: impl FnOnce() + Send + 'static) -> Self {
        Self {
            cancel: Some(Box::new(on_cancel)),
        }
    }

    /// 讓工作繼續跑到完成，不再能取消。
    pub fn detach(mut self) {
        self.cancel = None;
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("detached", &self.cancel.is_none())
            .finish()
    }
}

/// 把 `f` 包成可中止的 future 並回傳對應的把手：給沒有原生取消機制的執行器用。
/// 把手 drop 後，執行器下一次輪詢該 future 時它就會結束並被 drop。
pub fn abortable_task(f: BoxFuture<'static, ()>) -> (BoxFuture<'static, ()>, TaskHandle) {
    let (handle, registration) = AbortHandle::new_pair();
    let fut = Abortable::new(f, registration);
    (
        Box::pin(async move {
            let _ = fut.await;
        }),
        TaskHandle::new(move || handle.abort()),
    )
}

/// 注入的執行器：在背景跑一個 future。
pub trait Spawner: Send + Sync {
    fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle;
}

/// 忽略 mutex 中毒：本模組持鎖時不執行使用者程式碼，中毒只可能來自內部 panic，狀態仍一致。
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Callback<T> = Arc<dyn Fn(&T) + Send + Sync>;

struct Subscribers<T> {
    next_id: u64,
    list: Vec<(u64, Callback<T>)>,
}

/// 同步的一對多通知。回呼在 `notify` 的呼叫端執行緒上、且不持鎖執行，
/// 因此回呼內可以再 subscribe、取消訂閱或 notify，不會死結。
pub struct Notifier<T> {
    subs: Arc<Mutex<Subscribers<T>>>,
}

/// 訂閱把手：drop 即取消訂閱。
#[must_use = "Subscription 被 drop 時就會取消訂閱"]
pub struct Subscription {
    unsubscribe: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(f) = self.unsubscribe.take() {
            f();
        }
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Subscription")
    }
}

impl<T: Send + 'static> Notifier<T> {
    pub fn new() -> Self {
        Self {
            subs: Arc::new(Mutex::new(Subscribers {
                next_id: 0,
                list: Vec::new(),
            })),
        }
    }

    pub fn subscribe(&self, f: Box<dyn Fn(&T) + Send + Sync>) -> Subscription {
        let id = {
            let mut subs = lock(&self.subs);
            let id = subs.next_id;
            subs.next_id += 1;
            subs.list.push((id, Arc::from(f)));
            id
        };
        let weak: Weak<Mutex<Subscribers<T>>> = Arc::downgrade(&self.subs);
        Subscription {
            unsubscribe: Some(Box::new(move || {
                if let Some(subs) = weak.upgrade() {
                    // 先移出再於鎖外 drop：回呼的 drop 可能再碰這個 Notifier。
                    let removed = {
                        let mut subs = lock(&subs);
                        subs.list
                            .iter()
                            .position(|(i, _)| *i == id)
                            .map(|pos| subs.list.remove(pos))
                    };
                    drop(removed);
                }
            })),
        }
    }

    /// 以呼叫當下的訂閱者快照依訂閱順序呼叫。通知途中才取消的訂閱者仍可能收到這一次。
    pub fn notify(&self, v: &T) {
        let snapshot: Vec<Callback<T>> = lock(&self.subs)
            .list
            .iter()
            .map(|(_, f)| f.clone())
            .collect();
        for f in snapshot {
            f(v);
        }
    }
}

impl<T: Send + 'static> Default for Notifier<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn notifier_calls_every_subscriber_and_stops_after_subscription_drop() {
        let n: Notifier<u32> = Notifier::new();
        let total = Arc::new(AtomicUsize::new(0));
        let t = total.clone();
        let sub = n.subscribe(Box::new(move |v| {
            t.fetch_add(*v as usize, Ordering::SeqCst);
        }));
        n.notify(&3);
        assert_eq!(total.load(Ordering::SeqCst), 3);
        drop(sub);
        n.notify(&3);
        assert_eq!(total.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn notifier_callback_may_reenter_the_notifier_without_deadlock() {
        let n: Arc<Notifier<u32>> = Arc::new(Notifier::new());
        let inner_subs = Arc::new(Mutex::new(Vec::new()));
        let (n2, s2) = (n.clone(), inner_subs.clone());
        let _sub = n.subscribe(Box::new(move |v| {
            if *v == 1 {
                s2.lock().unwrap().push(n2.subscribe(Box::new(|_| {})));
                n2.notify(&2);
            }
        }));
        n.notify(&1);
        assert_eq!(inner_subs.lock().unwrap().len(), 1);
        inner_subs.lock().unwrap().clear();
    }

    #[test]
    fn subscription_outliving_notifier_drops_cleanly() {
        let n: Notifier<u8> = Notifier::new();
        let sub = n.subscribe(Box::new(|_| {}));
        drop(n);
        drop(sub);
    }

    #[test]
    fn task_handle_cancels_on_drop_but_not_after_detach() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let c = cancelled.clone();
        TaskHandle::new(move || c.store(true, Ordering::SeqCst)).detach();
        assert!(!cancelled.load(Ordering::SeqCst));
        let c = cancelled.clone();
        drop(TaskHandle::new(move || c.store(true, Ordering::SeqCst)));
        assert!(cancelled.load(Ordering::SeqCst));
    }
}
