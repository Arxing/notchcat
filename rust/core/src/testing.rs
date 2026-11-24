//! 測試替身（feature `testing`；本 crate 的單元測試一律可用）：
//! `FakeClock`（手動推進的時間）、`FakeHttp`（依 URL 排程的回應）、`LocalSpawner`（單執行緒、跑到停滯為止）。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::task::{waker, ArcWake};

use crate::http::{HttpClient, HttpError, HttpRequest, HttpResponse};
use crate::runtime::{abortable_task, lock, BoxFuture, Clock, Spawner, TaskHandle};

// ---------------------------------------------------------------- FakeClock

struct Sleeper {
    id: u64,
    deadline: Duration,
    waker: Option<Waker>,
}

struct ClockState {
    /// `now()` = `base` + `elapsed`；`set_now` 只改 `base`，sleep 只看 `elapsed`。
    base: f64,
    elapsed: Duration,
    next_id: u64,
    sleepers: Vec<Sleeper>,
}

/// 手動推進的時間。`advance` 推進時間並喚醒所有到期的 sleep；`set_now` 只改牆上時間讀數，不喚醒任何 sleep。
#[derive(Clone)]
pub struct FakeClock {
    state: Arc<Mutex<ClockState>>,
}

impl FakeClock {
    pub fn new(now: f64) -> Self {
        Self {
            state: Arc::new(Mutex::new(ClockState {
                base: now,
                elapsed: Duration::ZERO,
                next_id: 0,
                sleepers: Vec::new(),
            })),
        }
    }

    /// 把牆上時間設成 `now`（Unix epoch 秒），例如測快取 TTL；不影響進行中的 sleep。
    pub fn set_now(&self, now: f64) {
        let mut s = lock(&self.state);
        s.base = now - s.elapsed.as_secs_f64();
    }

    /// 推進時間 `d`，喚醒所有到期的 sleep（在鎖外喚醒）。被喚醒的工作要再跑一次執行器才會前進。
    pub fn advance(&self, d: Duration) {
        let wakers: Vec<Waker> = {
            let mut s = lock(&self.state);
            s.elapsed = s.elapsed.saturating_add(d);
            let elapsed = s.elapsed;
            let mut due = Vec::new();
            s.sleepers.retain_mut(|sl| {
                if sl.deadline <= elapsed {
                    due.extend(sl.waker.take());
                    false
                } else {
                    true
                }
            });
            due
        };
        for w in wakers {
            w.wake();
        }
    }

    /// 尚未到期、也尚未被 drop 的 sleep 數量。
    pub fn pending_sleeps(&self) -> usize {
        lock(&self.state).sleepers.len()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> f64 {
        let s = lock(&self.state);
        s.base + s.elapsed.as_secs_f64()
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let mut s = lock(&self.state);
        let deadline = s.elapsed.saturating_add(d);
        let id = s.next_id;
        s.next_id += 1;
        if deadline > s.elapsed {
            s.sleepers.push(Sleeper {
                id,
                deadline,
                waker: None,
            });
        }
        Box::pin(FakeSleep {
            state: self.state.clone(),
            id,
            deadline,
        })
    }
}

struct FakeSleep {
    state: Arc<Mutex<ClockState>>,
    id: u64,
    deadline: Duration,
}

impl Future for FakeSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = lock(&self.state);
        if s.elapsed >= self.deadline {
            return Poll::Ready(());
        }
        if let Some(sl) = s.sleepers.iter_mut().find(|sl| sl.id == self.id) {
            // 每次輪詢都換成最新的 waker（工作可能被搬到別的 context）。
            match &mut sl.waker {
                Some(w) if w.will_wake(cx.waker()) => {}
                slot => *slot = Some(cx.waker().clone()),
            }
        }
        Poll::Pending
    }
}

impl Drop for FakeSleep {
    fn drop(&mut self) {
        let removed = {
            let mut s = lock(&self.state);
            s.sleepers
                .iter()
                .position(|sl| sl.id == self.id)
                .map(|pos| s.sleepers.remove(pos))
        };
        drop(removed);
    }
}

// ---------------------------------------------------------------- FakeHttp

/// `FakeHttp` 對某個 URL 的固定行為。
#[derive(Debug, Clone)]
pub enum FakeReply {
    Respond(HttpResponse),
    Fail(HttpError),
    /// 永不回應（用來測逾時）。
    Hang,
}

#[derive(Default)]
struct HttpState {
    replies: HashMap<String, FakeReply>,
    requests: Vec<HttpRequest>,
}

/// 依完整 URL 排程回應；沒排程的 URL 回 `Transport` 錯誤。記錄每一次請求。
#[derive(Default)]
pub struct FakeHttp {
    state: Mutex<HttpState>,
}

impl FakeHttp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, url: impl Into<String>, reply: FakeReply) {
        lock(&self.state).replies.insert(url.into(), reply);
    }

    pub fn respond(&self, url: impl Into<String>, status: u16, body: impl Into<Vec<u8>>) {
        self.set(
            url,
            FakeReply::Respond(HttpResponse {
                status,
                body: body.into(),
            }),
        );
    }

    pub fn fail(&self, url: impl Into<String>, err: HttpError) {
        self.set(url, FakeReply::Fail(err));
    }

    pub fn hang(&self, url: impl Into<String>) {
        self.set(url, FakeReply::Hang);
    }

    /// 對 `url` 發出的請求次數。
    pub fn request_count(&self, url: &str) -> usize {
        lock(&self.state)
            .requests
            .iter()
            .filter(|r| r.url == url)
            .count()
    }

    /// 依序記錄的所有請求。
    pub fn requests(&self) -> Vec<HttpRequest> {
        lock(&self.state).requests.clone()
    }
}

impl HttpClient for FakeHttp {
    fn get(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, HttpError>> {
        let reply = {
            let mut s = lock(&self.state);
            let reply = s.replies.get(&req.url).cloned();
            s.requests.push(req.clone());
            reply
        };
        match reply {
            Some(FakeReply::Respond(r)) => Box::pin(std::future::ready(Ok(r))),
            Some(FakeReply::Fail(e)) => Box::pin(std::future::ready(Err(e))),
            Some(FakeReply::Hang) => Box::pin(std::future::pending()),
            None => Box::pin(std::future::ready(Err(HttpError::Transport(format!(
                "FakeHttp: no reply scheduled for {}",
                req.url
            ))))),
        }
    }
}

// ---------------------------------------------------------------- LocalSpawner

struct WakeFlag(AtomicBool);

impl ArcWake for WakeFlag {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.store(true, Ordering::SeqCst);
    }
}

struct SpawnerInner {
    incoming: Mutex<Vec<BoxFuture<'static, ()>>>,
    tasks: Mutex<FuturesUnordered<BoxFuture<'static, ()>>>,
    woken: Arc<WakeFlag>,
}

/// 決定性的單執行緒執行器：`spawn` 只排入，`run_until_stalled` 在呼叫端執行緒上把所有工作跑到沒有進展為止。
///
/// 語意等同 `futures::executor::LocalPool::run_until_stalled`，但 `LocalPool` 是 `!Send`／`!Sync`，
/// 無法實作 `Spawner: Send + Sync`，所以以 `FuturesUnordered` 自行驅動（見 ADR-0001）。
/// 把手 drop 後，工作在下一次 `run_until_stalled` 時被 drop。
#[derive(Clone)]
pub struct LocalSpawner {
    inner: Arc<SpawnerInner>,
}

impl LocalSpawner {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SpawnerInner {
                incoming: Mutex::new(Vec::new()),
                tasks: Mutex::new(FuturesUnordered::new()),
                woken: Arc::new(WakeFlag(AtomicBool::new(false))),
            }),
        }
    }

    /// 反覆輪詢所有被喚醒（或新排入）的工作，直到沒有任何工作能再前進。
    /// 工作內可以再 `spawn`；不可在工作內遞迴呼叫 `run_until_stalled`。
    pub fn run_until_stalled(&self) {
        let waker = waker(self.inner.woken.clone());
        let mut cx = Context::from_waker(&waker);
        let mut tasks = lock(&self.inner.tasks);
        loop {
            let incoming = std::mem::take(&mut *lock(&self.inner.incoming));
            // 新排入的工作進了 FuturesUnordered 的就緒佇列，下面這次 poll_next 就會輪詢到。
            tasks.extend(incoming);
            self.inner.woken.0.store(false, Ordering::SeqCst);
            match tasks.poll_next_unpin(&mut cx) {
                Poll::Ready(Some(())) => continue,
                Poll::Ready(None) | Poll::Pending => {
                    // FuturesUnordered 為了公平會輪詢一批後主動讓出（先喚醒外層 waker 再回 Pending），
                    // 所以 Pending 不代表停滯；只有沒被喚醒、也沒有新工作時才算停滯。
                    let woken = self.inner.woken.0.swap(false, Ordering::SeqCst);
                    let more = !lock(&self.inner.incoming).is_empty();
                    if !woken && !more {
                        return;
                    }
                }
            }
        }
    }

    /// 尚未結束的工作數（含尚未輪詢過的）。
    pub fn pending_tasks(&self) -> usize {
        lock(&self.inner.tasks).len() + lock(&self.inner.incoming).len()
    }
}

impl Default for LocalSpawner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spawner for LocalSpawner {
    fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle {
        let (fut, handle) = abortable_task(f);
        lock(&self.inner.incoming).push(fut);
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn fake_clock_now_follows_set_now_and_advance() {
        let c = FakeClock::new(100.0);
        c.advance(Duration::from_millis(1500));
        assert_eq!(c.now(), 101.5);
        c.set_now(500.0);
        assert_eq!(c.now(), 500.0);
        c.advance(Duration::from_secs(1));
        assert_eq!(c.now(), 501.0);
    }

    #[test]
    fn fake_sleep_wakes_only_when_its_deadline_is_reached() {
        let clock = FakeClock::new(0.0);
        let spawner = LocalSpawner::new();
        let done = Arc::new(AtomicUsize::new(0));
        let (c, d) = (clock.clone(), done.clone());
        let _t = spawner.spawn(Box::pin(async move {
            c.sleep(Duration::from_secs(2)).await;
            d.fetch_add(1, Ordering::SeqCst);
        }));
        spawner.run_until_stalled();
        clock.set_now(1e12);
        clock.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(done.load(Ordering::SeqCst), 0);
        clock.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(done.load(Ordering::SeqCst), 1);
        assert_eq!(spawner.pending_tasks(), 0);
    }

    #[test]
    fn zero_and_max_sleeps_neither_trap_nor_register_wrongly() {
        let clock = FakeClock::new(0.0);
        let spawner = LocalSpawner::new();
        let done = Arc::new(AtomicUsize::new(0));
        let (c, d) = (clock.clone(), done.clone());
        let _t = spawner.spawn(Box::pin(async move {
            c.sleep(Duration::ZERO).await;
            d.fetch_add(1, Ordering::SeqCst);
            c.sleep(Duration::MAX).await;
            d.fetch_add(1, Ordering::SeqCst);
        }));
        spawner.run_until_stalled();
        assert_eq!(done.load(Ordering::SeqCst), 1);
        clock.advance(Duration::MAX);
        spawner.run_until_stalled();
        assert_eq!(done.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn dropping_the_task_handle_drops_the_task() {
        let clock = FakeClock::new(0.0);
        let spawner = LocalSpawner::new();
        let c = clock.clone();
        let handle = spawner.spawn(Box::pin(async move {
            c.sleep(Duration::from_secs(1)).await;
        }));
        spawner.run_until_stalled();
        assert_eq!(clock.pending_sleeps(), 1);
        drop(handle);
        spawner.run_until_stalled();
        assert_eq!(clock.pending_sleeps(), 0);
        assert_eq!(spawner.pending_tasks(), 0);
    }

    #[test]
    fn tasks_spawned_from_tasks_run_in_the_same_turn() {
        let spawner = LocalSpawner::new();
        let done = Arc::new(AtomicUsize::new(0));
        let (s, d) = (spawner.clone(), done.clone());
        spawner
            .spawn(Box::pin(async move {
                let d2 = d.clone();
                s.spawn(Box::pin(async move {
                    d2.fetch_add(1, Ordering::SeqCst);
                }))
                .detach();
                d.fetch_add(1, Ordering::SeqCst);
            }))
            .detach();
        spawner.run_until_stalled();
        assert_eq!(done.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn fake_http_replies_by_url_and_counts_requests() {
        let http = FakeHttp::new();
        http.respond("https://a", 200, "ok");
        http.fail("https://b", HttpError::TooLarge);
        http.hang("https://c");
        let req = |url: &str| HttpRequest {
            url: url.to_string(),
            headers: vec![("User-Agent".into(), "MusiCat".into())],
            timeout: Duration::from_secs(5),
        };
        let spawner = LocalSpawner::new();
        let http = Arc::new(http);
        let out = Arc::new(Mutex::new(Vec::new()));
        let (h, o) = (http.clone(), out.clone());
        let _t = spawner.spawn(Box::pin(async move {
            let a = h.get(req("https://a")).await;
            let b = h.get(req("https://b")).await;
            let z = h.get(req("https://unknown")).await;
            o.lock().unwrap().extend([Some(a), Some(b), Some(z)]);
            let _ = h.get(req("https://c")).await;
            o.lock().unwrap().push(None);
        }));
        spawner.run_until_stalled();
        let out = out.lock().unwrap();
        assert_eq!(out.len(), 3, "hang 永不回應");
        assert_eq!(
            out[0],
            Some(Ok(HttpResponse {
                status: 200,
                body: b"ok".to_vec()
            }))
        );
        assert_eq!(out[1], Some(Err(HttpError::TooLarge)));
        assert!(matches!(out[2], Some(Err(HttpError::Transport(_)))));
        assert_eq!(http.request_count("https://a"), 1);
        assert_eq!(http.request_count("https://c"), 1);
        assert_eq!(http.requests().len(), 4);
        assert_eq!(http.requests()[0].headers[0].1, "MusiCat");
    }
}
