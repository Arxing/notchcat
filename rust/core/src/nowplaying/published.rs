//! 「目前值 + 訂閱者」的發布格（來源與監視器共用）：目前的 `Option<TrackSnapshot>`、訂閱者清單與
//! 序列化的送出佇列。`HelperSupervisor`、`WindowTitleSource` 與 [`super::NowPlayingMonitor`] 都用它，
//! 取代各自手寫的同一套邏輯。
//!
//! 並行與重播保證（Task 8 修正後的設計）：
//! - 發布（[`PublishedSnapshot::queue_publish`]／[`PublishedSnapshot::queue_set`]）只在內部鎖裡更新目前值、
//!   遞增序號並把事件排入佇列，**不**呼叫任何訂閱者；呼叫端放掉自己的鎖之後再呼叫
//!   [`PublishedSnapshot::flush`]，由「同一時間只有一條執行緒」的送出者在鎖外依序送給訂閱者。
//!   呼叫端可以在持有自己的狀態鎖時排入（鎖順序固定為呼叫端的鎖 → 本格的鎖；本格從不回頭呼叫呼叫端），
//!   所以事件的順序與呼叫端狀態變化的順序一致，訂閱者不會在新值之後收到舊值。
//! - 訂閱時的「重播目前值」也是佇列裡的一個事件：訂閱者登記時記下當下的序號，只會收到序號更大的廣播，
//!   加上一筆排在佇列尾端、內容為登記當下目前值的重播。因此第一次呼叫一定是重播，之後嚴格是更新的狀態：
//!   不重複，也不在新值之後收到舊值。
//! - 回呼內可以再讀 `current()`、訂閱、取消訂閱，甚至讓來源再發布（只會排進佇列，由正在送的執行緒接著送），
//!   不會死結。
//! - 訂閱者 panic 時，這一輪進行中的那個值對排在它之後的訂閱者就遺失了（之後的更新照常送達）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::{SharedSnapshotCallback, SnapshotCallback};
use crate::runtime::{lock, Subscription};
use crate::types::TrackSnapshot;

struct Sub {
    id: u64,
    /// 登記當下已發出的最後一個廣播序號；只接收序號更大的廣播。
    since_seq: u64,
    cancelled: AtomicBool,
    cb: SharedSnapshotCallback,
}

enum Event {
    /// 狀態變動，送給 `since_seq < seq` 的所有訂閱者。
    Broadcast {
        seq: u64,
        value: Option<TrackSnapshot>,
    },
    /// 只送給剛訂閱的那一位：登記當下的目前值。
    Replay {
        target: Arc<Sub>,
        value: Option<TrackSnapshot>,
    },
}

struct State {
    current: Option<TrackSnapshot>,
    /// 已排定、尚未送出的事件（依產生順序）。
    pending: VecDeque<Event>,
    /// 最後一個廣播的序號。
    seq: u64,
    next_sub_id: u64,
    subscribers: Vec<Arc<Sub>>,
    /// 是否已有一條執行緒正在送事件。
    draining: bool,
}

struct Inner {
    state: Mutex<State>,
}

/// 送事件的流程若因訂閱者 panic 而中途離開，要把 `draining` 放掉，否則之後再也沒有人會送。
struct DrainGuard<'a> {
    inner: &'a Inner,
    armed: bool,
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            lock(&self.inner.state).draining = false;
        }
    }
}

impl Inner {
    /// 若沒有別的執行緒正在送，就由呼叫端接手把佇列送完；回呼一律在鎖外執行。
    fn drain(&self) {
        {
            let mut st = lock(&self.state);
            if st.draining || st.pending.is_empty() {
                return;
            }
            st.draining = true;
        }
        let mut guard = DrainGuard {
            inner: self,
            armed: true,
        };
        loop {
            let (targets, value) = {
                let mut st = lock(&self.state);
                match st.pending.pop_front() {
                    Some(Event::Broadcast { seq, value }) => {
                        let targets: Vec<Arc<Sub>> = st
                            .subscribers
                            .iter()
                            .filter(|s| s.since_seq < seq)
                            .cloned()
                            .collect();
                        (targets, value)
                    }
                    Some(Event::Replay { target, value }) => (vec![target], value),
                    None => {
                        st.draining = false;
                        guard.armed = false;
                        return;
                    }
                }
            };
            for target in targets {
                if !target.cancelled.load(Ordering::Acquire) {
                    (target.cb)(&value);
                }
            }
        }
    }

    fn unsubscribe(&self, id: u64) {
        let removed = {
            let mut st = lock(&self.state);
            st.subscribers
                .iter()
                .position(|s| s.id == id)
                .map(|pos| st.subscribers.remove(pos))
        };
        if let Some(sub) = removed {
            sub.cancelled.store(true, Ordering::Release);
            // 回呼的 drop 可能再碰這個格，所以在鎖外釋放。
            drop(sub);
        }
    }
}

/// 發布格的把手（複製只是多一個把手，指向同一個格）。
#[derive(Clone)]
pub struct PublishedSnapshot {
    inner: Arc<Inner>,
}

impl PublishedSnapshot {
    /// 目前值從 `None` 開始。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    current: None,
                    pending: VecDeque::new(),
                    seq: 0,
                    next_sub_id: 0,
                    subscribers: Vec::new(),
                    draining: false,
                }),
            }),
        }
    }

    pub fn current(&self) -> Option<TrackSnapshot> {
        lock(&self.inner.state).current.clone()
    }

    /// 把 `value` 設為目前值並排入廣播（即使與目前值相同；Swift `CurrentValueSubject.send` 的語意）。
    /// 只排入、不通知任何人；呼叫端放掉自己的鎖之後要呼叫 [`PublishedSnapshot::flush`]。
    pub fn queue_publish(&self, value: Option<TrackSnapshot>) {
        let mut st = lock(&self.inner.state);
        st.current = value.clone();
        st.seq += 1;
        let seq = st.seq;
        st.pending.push_back(Event::Broadcast { seq, value });
    }

    /// 與 [`PublishedSnapshot::queue_publish`] 相同，但值等於目前值時什麼都不做；回傳是否真的排入了廣播。
    pub fn queue_set(&self, value: Option<TrackSnapshot>) -> bool {
        let mut st = lock(&self.inner.state);
        if st.current == value {
            return false;
        }
        st.current = value.clone();
        st.seq += 1;
        let seq = st.seq;
        st.pending.push_back(Event::Broadcast { seq, value });
        true
    }

    /// 若沒有別的執行緒正在送，就由呼叫端接手把佇列送完；回呼一律在鎖外執行，所以呼叫時不得持有
    /// 會被訂閱者回呼碰到的鎖。有別的執行緒在送時立刻返回，由它接著送。
    pub fn flush(&self) {
        self.inner.drain();
    }

    /// 第一次呼叫一定是登記當下目前值的重播，之後只在發布時呼叫，且嚴格依發布順序（不重複、不倒退）。
    /// 沒有別的執行緒正在送通知時，重播在 `subscribe` 返回前同步完成；否則（含在回呼內訂閱）
    /// 由正在送的執行緒在這個回呼之後接著送。drop 回傳的把手即取消；把手可以比格活得久。
    pub fn subscribe(&self, f: SnapshotCallback) -> Subscription {
        let id = {
            let mut st = lock(&self.inner.state);
            let id = st.next_sub_id;
            st.next_sub_id += 1;
            let sub = Arc::new(Sub {
                id,
                since_seq: st.seq,
                cancelled: AtomicBool::new(false),
                cb: Arc::from(f),
            });
            st.subscribers.push(sub.clone());
            let value = st.current.clone();
            st.pending.push_back(Event::Replay { target: sub, value });
            id
        };
        self.inner.drain();
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.unsubscribe(id);
            }
        })
    }
}

impl Default for PublishedSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn track(title: &str) -> Option<TrackSnapshot> {
        Some(TrackSnapshot {
            title: title.to_owned(),
            artist: None,
            album: None,
            duration: None,
            is_playing: true,
            position: None,
            artwork: None,
            source_id: "test".to_owned(),
        })
    }

    fn title(v: &Option<TrackSnapshot>) -> Option<String> {
        v.as_ref().map(|s| s.title.clone())
    }

    type Seen = Arc<Mutex<Vec<Option<String>>>>;

    fn recorder() -> (Seen, SnapshotCallback) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (seen, Box::new(move |v| lock(&s).push(title(v))))
    }

    fn titles(seen: &Seen) -> Vec<Option<String>> {
        lock(seen).clone()
    }

    fn some(t: &str) -> Option<String> {
        Some(t.to_owned())
    }

    /// 排入並立刻送出（測試用的便利函式；真實呼叫端在放鎖後才 flush）。
    fn set(p: &PublishedSnapshot, value: Option<TrackSnapshot>) -> bool {
        let changed = p.queue_set(value);
        p.flush();
        changed
    }

    fn publish(p: &PublishedSnapshot, value: Option<TrackSnapshot>) {
        p.queue_publish(value);
        p.flush();
    }

    #[test]
    fn starts_with_none_and_subscribe_replays_it() {
        let p = PublishedSnapshot::new();
        assert_eq!(p.current(), None);
        let (seen, cb) = recorder();
        let _sub = p.subscribe(cb);
        assert_eq!(titles(&seen), vec![None]);
    }

    #[test]
    fn subscribe_replays_the_current_value_immediately() {
        let p = PublishedSnapshot::new();
        set(&p, track("A"));
        let (seen, cb) = recorder();
        let _sub = p.subscribe(cb);
        assert_eq!(titles(&seen), vec![some("A")]);
    }

    #[test]
    fn set_publishes_only_when_the_value_changes() {
        let p = PublishedSnapshot::new();
        let (seen, cb) = recorder();
        let _sub = p.subscribe(cb);
        assert!(set(&p, track("A")));
        assert!(!set(&p, track("A")));
        assert!(!set(&p, track("A")));
        assert!(set(&p, None));
        assert!(!set(&p, None));
        assert_eq!(titles(&seen), vec![None, some("A"), None]);
        assert_eq!(p.current(), None);
    }

    #[test]
    fn publish_always_notifies_even_for_an_equal_value() {
        let p = PublishedSnapshot::new();
        let (seen, cb) = recorder();
        let _sub = p.subscribe(cb);
        publish(&p, None);
        publish(&p, track("A"));
        publish(&p, track("A"));
        assert_eq!(titles(&seen), vec![None, None, some("A"), some("A")]);
        assert_eq!(title(&p.current()), some("A"));
    }

    #[test]
    fn queued_values_are_not_delivered_until_flush() {
        let p = PublishedSnapshot::new();
        let (seen, cb) = recorder();
        let _sub = p.subscribe(cb);
        p.queue_set(track("A"));
        assert_eq!(title(&p.current()), some("A"), "current is updated at once");
        assert_eq!(titles(&seen), vec![None], "but nobody is called yet");
        p.flush();
        assert_eq!(titles(&seen), vec![None, some("A")]);
    }

    #[test]
    fn dropped_subscription_stops_receiving() {
        let p = PublishedSnapshot::new();
        let (seen, cb) = recorder();
        let sub = p.subscribe(cb);
        drop(sub);
        set(&p, track("A"));
        assert_eq!(titles(&seen), vec![None]);
    }

    #[test]
    fn subscription_may_outlive_the_cell() {
        let p = PublishedSnapshot::new();
        let sub = p.subscribe(Box::new(|_| {}));
        drop(p);
        drop(sub);
    }

    #[test]
    fn clones_share_one_cell() {
        let p = PublishedSnapshot::new();
        let q = p.clone();
        let (seen, cb) = recorder();
        let _sub = q.subscribe(cb);
        set(&p, track("A"));
        assert_eq!(title(&q.current()), some("A"));
        assert_eq!(titles(&seen), vec![None, some("A")]);
    }

    #[test]
    fn subscriber_that_publishes_sees_values_in_order_without_deadlock() {
        let p = PublishedSnapshot::new();
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let (s, p2) = (seen.clone(), p.clone());
        let _sub = p.subscribe(Box::new(move |v| {
            lock(&s).push(title(v));
            if title(v).as_deref() == Some("A") {
                p2.queue_set(track("B"));
                p2.flush();
            }
        }));
        set(&p, track("A"));
        assert_eq!(titles(&seen), vec![None, some("A"), some("B")]);
        assert_eq!(title(&p.current()), some("B"));
    }

    #[test]
    fn subscriber_may_read_current_and_subscribe_during_notification() {
        let p = PublishedSnapshot::new();
        let (p2, inner) = (p.clone(), Arc::new(Mutex::new(Vec::new())));
        let inner2 = inner.clone();
        let _sub = p.subscribe(Box::new(move |v| {
            if v.is_some() {
                let _ = p2.current();
                lock(&inner2).push(p2.subscribe(Box::new(|_| {})));
            }
        }));
        set(&p, track("A"));
        assert_eq!(lock(&inner).len(), 1);
        lock(&inner).clear();
    }

    #[test]
    fn subscriber_may_unsubscribe_itself_during_notification() {
        let p = PublishedSnapshot::new();
        let slot: Arc<Mutex<Option<Subscription>>> = Arc::new(Mutex::new(None));
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let (s, slot2) = (seen.clone(), slot.clone());
        let sub = p.subscribe(Box::new(move |v| {
            lock(&s).push(title(v));
            if v.is_some() {
                drop(lock(&slot2).take());
            }
        }));
        *lock(&slot) = Some(sub);
        set(&p, track("A"));
        set(&p, track("B"));
        assert_eq!(titles(&seen), vec![None, some("A")]);
    }

    #[test]
    fn concurrent_publishers_converge_without_deadlock() {
        let p = PublishedSnapshot::new();
        let last = Arc::new(Mutex::new(None));
        let l = last.clone();
        let _sub = p.subscribe(Box::new(move |v| *lock(&l) = v.clone()));
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for n in 0..200 {
                        let t = format!("{i}-{n}");
                        publish(&p, if n % 7 == 0 { None } else { track(&t) });
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // 最後一次通知給訂閱者的值必須等於最終狀態（沒有舊值蓋掉新值）。
        assert_eq!(*lock(&last), p.current());
    }

    /// 回歸：訂閱時的重播必須是串流的一部分。
    /// 送通知的執行緒卡在訂閱者 S0 的回呼裡（已送出 A）時，主執行緒再發布 B、C 並新增訂閱者 S1：
    /// S1 的第一次呼叫必須是登記當下的目前值（C）；不得先收到舊值 A（過時），
    /// 也不得在 C 之後又收到 B 或 C（重複）。全程以 channel 手動排定交錯，沒有 sleep。
    #[test]
    fn subscriber_added_while_another_thread_is_draining_gets_current_then_only_newer() {
        let p = PublishedSnapshot::new();
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let entered_tx = Mutex::new(entered_tx);
        let release_rx = Mutex::new(release_rx);
        let s0_seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let s0 = s0_seen.clone();
        let _s0_sub = p.subscribe(Box::new(move |v| {
            lock(&s0).push(title(v));
            if title(v).as_deref() == Some("A") {
                lock(&entered_tx).send(()).unwrap();
                lock(&release_rx).recv().unwrap();
            }
        }));

        let p2 = p.clone();
        let drainer = std::thread::spawn(move || set(&p2, track("A")));
        // 此刻 drainer 已把 A 交給 S0 並卡在回呼裡（持有「正在送」的身分，不持任何鎖）。
        entered_rx.recv().unwrap();

        set(&p, track("B")); // 只排進佇列，立刻返回
        set(&p, track("C"));
        let (s1_seen, cb) = recorder();
        let _s1_sub = p.subscribe(cb);
        // 有別的執行緒在送：重播排在佇列裡，S1 目前還沒收到任何東西，也沒有被舊值 A 搶先。
        assert!(titles(&s1_seen).is_empty());

        release_tx.send(()).unwrap();
        drainer.join().unwrap();

        assert_eq!(
            titles(&s1_seen),
            vec![some("C")],
            "S1: first call is the replay of current-at-registration, then nothing stale or duplicated"
        );
        assert_eq!(
            titles(&s0_seen),
            vec![None, some("A"), some("B"), some("C")]
        );

        // 之後的更新照常只送一次、依序送達兩位訂閱者。
        set(&p, track("D"));
        assert_eq!(titles(&s1_seen), vec![some("C"), some("D")]);
    }

    #[test]
    fn panicking_subscriber_does_not_wedge_later_notifications() {
        let p = PublishedSnapshot::new();
        let _bad = p.subscribe(Box::new(|v| {
            if v.as_ref().is_some_and(|s| s.title == "boom") {
                panic!("subscriber panic (expected by test)");
            }
        }));
        let (seen, cb) = recorder();
        let _good = p.subscribe(cb);

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| set(&p, track("boom"))));
        assert!(result.is_err());
        // 進行中的 "boom" 對排在 panic 者之後的訂閱者遺失（已記載），但之後的更新照常送達。
        set(&p, track("next"));

        assert_eq!(
            titles(&seen),
            vec![None, some("next")],
            "later update still delivered after a subscriber panicked"
        );
        assert_eq!(title(&p.current()), some("next"));
    }
}
