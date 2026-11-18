//! 把多個現正播放來源合成單一 `current` 快照（移植自 Swift `NowPlayingMonitor.swift`）。
//!
//! 來源依優先序列出。最新值非 `None` 的第一個來源勝出，即使它是暫停狀態；
//! 尚未發布過的來源視為 `None`。勝出者的封面由 [`ArtworkCache`] 補齊。
//!
//! 並行：來源可能在任意執行緒上發布。狀態（各來源最新值、目前結果、封面快取、訂閱者清單）由單一 mutex 保護，
//! 在鎖內算出結果並排入待送佇列，**鎖外**才執行訂閱者回呼。佇列由「同一時間只有一條執行緒」依序送出，
//! 所以訂閱者看到的順序與狀態變化的順序一致，且回呼內可以再讀 `current()`、訂閱，
//! 甚至讓來源再發布（該次更新只會排進佇列，由正在送的執行緒接著送），都不會死結。
//!
//! 訂閱時的「重播目前值」也是這條序列化串流裡的一個事件：訂閱者登記時記下當下的序號，
//! 只會收到序號更大的廣播，加上一筆排在佇列尾端、內容為登記當下 `current` 的重播。
//! 因此第一次呼叫一定是重播，之後嚴格是更新的狀態：不會重複，也不會在新值之後收到舊值。
//!
//! 訂閱者 panic 時，這一輪的進行中那個值對排在它之後的訂閱者就遺失了（之後的更新照常送達）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::{NowPlayingSource, SharedSnapshotCallback, SnapshotCallback};
use crate::artwork_cache::ArtworkCache;
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
    /// 只送給剛訂閱的那一位：登記當下的 `current`。
    Replay {
        target: Arc<Sub>,
        value: Option<TrackSnapshot>,
    },
}

struct State {
    /// 各來源的最新值，索引與 `new` 收到的 `sources` 相同。
    latest: Vec<Option<TrackSnapshot>>,
    current: Option<TrackSnapshot>,
    artwork: ArtworkCache,
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
    fn update(&self, index: usize, value: Option<TrackSnapshot>) {
        {
            let mut st = lock(&self.state);
            st.latest[index] = value;
            let winner = st.latest.iter().find_map(|s| s.clone());
            let winner = match winner {
                Some(mut snapshot) => {
                    snapshot.artwork = st.artwork.resolve(snapshot.artwork.take());
                    Some(snapshot)
                }
                None => None,
            };
            if winner != st.current {
                st.current = winner.clone();
                st.seq += 1;
                let seq = st.seq;
                st.pending
                    .push_back(Event::Broadcast { seq, value: winner });
            }
        }
        self.drain();
    }

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
            // 回呼的 drop 可能再碰這個監視器，所以在鎖外釋放。
            drop(sub);
        }
    }
}

pub struct NowPlayingMonitor {
    inner: Arc<Inner>,
    /// 對各來源的訂閱；監視器 drop 時一併取消。包在 mutex 裡只是為了讓監視器是 `Sync`。
    _source_subscriptions: Mutex<Vec<Subscription>>,
}

impl NowPlayingMonitor {
    /// `sources` 依優先序排列。每個來源都會被訂閱；來源在訂閱當下同步重播的初始值也會被套用。
    pub fn new(sources: Vec<Arc<dyn NowPlayingSource>>, artwork: ArtworkCache) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                latest: vec![None; sources.len()],
                current: None,
                artwork,
                pending: VecDeque::new(),
                seq: 0,
                next_sub_id: 0,
                subscribers: Vec::new(),
                draining: false,
            }),
        });
        // 回呼只持有弱參照：來源比監視器活得久時不會把它撐住，也不會形成循環。
        let subscriptions = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let weak: Weak<Inner> = Arc::downgrade(&inner);
                source.subscribe(Box::new(move |snapshot| {
                    if let Some(inner) = weak.upgrade() {
                        inner.update(index, snapshot.clone());
                    }
                }))
            })
            .collect();
        Self {
            inner,
            _source_subscriptions: Mutex::new(subscriptions),
        }
    }

    pub fn current(&self) -> Option<TrackSnapshot> {
        lock(&self.inner.state).current.clone()
    }

    /// 訂閱 `current` 的變動（同 Combine 的 `@Published`）：第一次呼叫一定是登記當下 `current` 的重播，
    /// 之後只在結果真的改變時呼叫，且嚴格依狀態變化的順序（不重複、不倒退）。
    /// 沒有別的執行緒正在送通知時，重播在 `subscribe` 返回前同步完成；否則（含在回呼內訂閱）
    /// 由正在送的執行緒在這個回呼之後接著送。drop 回傳的把手即取消。
    pub fn subscribe(&self, f: SnapshotCallback) -> Subscription {
        let (sub, id) = {
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
            st.pending.push_back(Event::Replay {
                target: sub.clone(),
                value,
            });
            (sub, id)
        };
        drop(sub);
        self.inner.drain();
        let weak = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.unsubscribe(id);
            }
        })
    }

    /// `current?.is_playing ?? false`，不含連續重複（Swift `isPlayingPublisher`）。
    /// 訂閱當下立即以目前值呼叫一次。
    pub fn subscribe_is_playing(&self, f: Box<dyn Fn(&bool) + Send + Sync>) -> Subscription {
        let last: Mutex<Option<bool>> = Mutex::new(None);
        self.subscribe(Box::new(move |snapshot| {
            let playing = snapshot.as_ref().is_some_and(|s| s.is_playing);
            {
                let mut last = lock(&last);
                if *last == Some(playing) {
                    return;
                }
                *last = Some(playing);
            }
            f(&playing);
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{lock, Notifier};
    use crate::types::ArtworkRef;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// 對應 Swift 的 `CurrentValueSubject` 假來源：訂閱當下立即重播目前值。
    struct FakeSource {
        id: String,
        value: Mutex<Option<TrackSnapshot>>,
        notifier: Notifier<Option<TrackSnapshot>>,
    }

    impl FakeSource {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                value: Mutex::new(None),
                notifier: Notifier::new(),
            })
        }

        fn emit(&self, v: Option<TrackSnapshot>) {
            *lock(&self.value) = v.clone();
            self.notifier.notify(&v);
        }
    }

    impl NowPlayingSource for FakeSource {
        fn id(&self) -> &str {
            &self.id
        }
        fn start(&self) {}
        fn stop(&self) {}
        fn subscribe(&self, f: SnapshotCallback) -> Subscription {
            let f: SharedSnapshotCallback = Arc::from(f);
            let f2 = f.clone();
            let sub = self.notifier.subscribe(Box::new(move |v| f2(v)));
            let current = lock(&self.value).clone();
            f(&current);
            sub
        }
    }

    /// 從未發布過、連初始 `None` 都沒有的來源。
    struct SilentSource {
        notifier: Notifier<Option<TrackSnapshot>>,
    }

    impl SilentSource {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                notifier: Notifier::new(),
            })
        }
    }

    impl NowPlayingSource for SilentSource {
        fn id(&self) -> &str {
            "silent"
        }
        fn start(&self) {}
        fn stop(&self) {}
        fn subscribe(&self, f: SnapshotCallback) -> Subscription {
            self.notifier.subscribe(f)
        }
    }

    fn snapshot(
        title: &str,
        source: &str,
        is_playing: bool,
        artwork: Option<ArtworkRef>,
    ) -> Option<TrackSnapshot> {
        Some(TrackSnapshot {
            title: title.to_string(),
            artist: None,
            album: None,
            duration: None,
            is_playing,
            position: None,
            artwork,
            source_id: source.to_string(),
        })
    }

    fn playing(title: &str, source: &str) -> Option<TrackSnapshot> {
        snapshot(title, source, true, None)
    }

    fn art(id: &str, data: Option<&[u8]>) -> Option<ArtworkRef> {
        Some(ArtworkRef {
            id: id.to_string(),
            data: data.map(Arc::from),
        })
    }

    fn monitor(sources: Vec<Arc<dyn NowPlayingSource>>) -> NowPlayingMonitor {
        NowPlayingMonitor::new(sources, ArtworkCache::default())
    }

    fn title_of(m: &NowPlayingMonitor) -> Option<String> {
        m.current().map(|s| s.title)
    }

    fn setup() -> (Arc<FakeSource>, Arc<FakeSource>) {
        (FakeSource::new("primary"), FakeSource::new("fallback"))
    }

    #[test]
    fn first_non_nil_source_wins() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        fallback.emit(playing("Fallback", "fallback"));
        primary.emit(playing("Primary", "primary"));

        assert_eq!(title_of(&m).as_deref(), Some("Primary"));
        assert_eq!(m.current().unwrap().source_id, "primary");
    }

    #[test]
    fn falls_through_to_later_source_when_first_is_nil() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        primary.emit(playing("Primary", "primary"));
        fallback.emit(playing("Fallback", "fallback"));
        assert_eq!(title_of(&m).as_deref(), Some("Primary"));

        primary.emit(None);

        assert_eq!(title_of(&m).as_deref(), Some("Fallback"));
    }

    #[test]
    fn first_source_regaining_data_takes_back_over() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        fallback.emit(playing("Fallback", "fallback"));
        assert_eq!(title_of(&m).as_deref(), Some("Fallback"));

        primary.emit(playing("Primary", "primary"));

        assert_eq!(title_of(&m).as_deref(), Some("Primary"));
    }

    #[test]
    fn never_emitted_source_counts_as_nil() {
        let (_, fallback) = setup();
        let m = monitor(vec![SilentSource::new(), fallback.clone()]);
        assert_eq!(m.current(), None);

        fallback.emit(playing("Fallback", "fallback"));

        assert_eq!(title_of(&m).as_deref(), Some("Fallback"));
    }

    #[test]
    fn starts_with_initial_values_of_sources() {
        let (primary, fallback) = setup();
        primary.emit(playing("Already", "primary"));

        let m = monitor(vec![primary.clone(), fallback.clone()]);

        assert_eq!(title_of(&m).as_deref(), Some("Already"));
    }

    #[test]
    fn no_sources_yields_nil() {
        assert_eq!(monitor(vec![]).current(), None);
    }

    #[test]
    fn paused_snapshot_still_wins_over_later_source() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        fallback.emit(snapshot("Fallback", "fallback", true, None));
        primary.emit(snapshot("Paused", "primary", false, None));

        assert_eq!(title_of(&m).as_deref(), Some("Paused"));
        assert!(!m.current().unwrap().is_playing);
    }

    #[test]
    fn is_playing_publisher_deduplicates() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        let values = Arc::new(Mutex::new(Vec::new()));
        let v = values.clone();
        let _sub = m.subscribe_is_playing(Box::new(move |b| lock(&v).push(*b)));

        primary.emit(snapshot("One", "primary", true, None));
        primary.emit(snapshot("Two", "primary", true, None));
        primary.emit(snapshot("Two", "primary", false, None));
        primary.emit(snapshot("Three", "primary", false, None));
        primary.emit(None);
        primary.emit(snapshot("Four", "primary", true, None));

        assert_eq!(*lock(&values), vec![false, true, false, true]);
    }

    #[test]
    fn artwork_bytes_are_filled_in_from_earlier_snapshot_with_same_identifier() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);

        primary.emit(snapshot(
            "Song",
            "primary",
            true,
            art("art", Some(&[1, 2, 3])),
        ));
        assert_eq!(m.current().unwrap().artwork, art("art", Some(&[1, 2, 3])));

        primary.emit(snapshot("Song", "primary", false, art("art", None)));
        assert_eq!(m.current().unwrap().artwork, art("art", Some(&[1, 2, 3])));
    }

    #[test]
    fn artwork_with_unknown_identifier_stays_without_data() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);

        primary.emit(snapshot("Song", "primary", true, art("never-seen", None)));

        assert_eq!(m.current().unwrap().artwork, art("never-seen", None));
    }

    #[test]
    fn uses_injected_artwork_cache() {
        let primary = FakeSource::new("primary");
        let mut cache = ArtworkCache::default();
        let _ = cache.resolve(art("art", Some(&[9])));
        let m = NowPlayingMonitor::new(vec![primary.clone()], cache);

        primary.emit(snapshot("Song", "primary", true, art("art", None)));

        let data = m.current().unwrap().artwork.unwrap().data;
        assert_eq!(data.as_deref(), Some(&[9u8][..]));
    }

    #[test]
    fn current_is_not_republished_for_an_equal_snapshot() {
        let (primary, fallback) = setup();
        let m = monitor(vec![primary.clone(), fallback.clone()]);
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let _sub = m.subscribe(Box::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        // 訂閱當下會重播目前值一次；其後才計算「變動」次數。
        changes.store(0, Ordering::SeqCst);

        primary.emit(playing("Song", "primary"));
        primary.emit(playing("Song", "primary"));
        fallback.emit(playing("Other", "fallback"));

        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }

    // 以下是 Swift 測試沒有、但 Rust 版的並行與重入保證所需的額外案例。

    #[test]
    fn subscribe_replays_the_current_value_immediately() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);
        primary.emit(playing("Song", "primary"));

        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let _sub = m.subscribe(Box::new(move |v| lock(&s).push(v.clone())));

        assert_eq!(*lock(&seen), vec![playing("Song", "primary")]);
    }

    #[test]
    fn dropped_subscription_stops_receiving() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sub = m.subscribe(Box::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        drop(sub);
        let after_drop = count.load(Ordering::SeqCst);

        primary.emit(playing("Song", "primary"));

        assert_eq!(count.load(Ordering::SeqCst), after_drop);
    }

    #[test]
    fn dropping_the_monitor_unsubscribes_from_sources() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);
        drop(m);
        // 不得 panic，也不得再呼叫已被釋放的監視器。
        primary.emit(playing("Song", "primary"));
    }

    #[test]
    fn subscriber_that_makes_a_source_emit_sees_values_in_order_without_deadlock() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (s, p) = (seen.clone(), primary.clone());
        let _sub = m.subscribe(Box::new(move |v| {
            let title = v.as_ref().map(|x| x.title.clone());
            lock(&s).push(title.clone());
            if title.as_deref() == Some("A") {
                p.emit(playing("B", "primary"));
            }
        }));

        primary.emit(playing("A", "primary"));

        assert_eq!(
            *lock(&seen),
            vec![None, Some("A".to_string()), Some("B".to_string())]
        );
        assert_eq!(title_of(&m).as_deref(), Some("B"));
    }

    #[test]
    fn subscriber_may_read_current_and_subscribe_during_notification() {
        let primary = FakeSource::new("primary");
        let m = Arc::new(monitor(vec![primary.clone()]));
        let (m2, inner) = (m.clone(), Arc::new(Mutex::new(Vec::new())));
        let inner2 = inner.clone();
        let _sub = m.subscribe(Box::new(move |v| {
            if v.is_some() {
                let _ = m2.current();
                lock(&inner2).push(m2.subscribe(Box::new(|_| {})));
            }
        }));

        primary.emit(playing("A", "primary"));

        assert_eq!(lock(&inner).len(), 1);
        lock(&inner).clear();
    }

    #[test]
    fn concurrent_sources_converge_without_deadlock() {
        let sources: Vec<Arc<FakeSource>> =
            (0..4).map(|i| FakeSource::new(&format!("s{i}"))).collect();
        let m = Arc::new(monitor(
            sources
                .iter()
                .map(|s| s.clone() as Arc<dyn NowPlayingSource>)
                .collect(),
        ));
        let last = Arc::new(Mutex::new(None));
        let l = last.clone();
        let _sub = m.subscribe(Box::new(move |v| *lock(&l) = v.clone()));

        let handles: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let s = s.clone();
                std::thread::spawn(move || {
                    for n in 0..200 {
                        let title = format!("{i}-{n}");
                        s.emit(if n % 7 == 0 {
                            None
                        } else {
                            playing(&title, &format!("s{i}"))
                        });
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        // 最後一次通知給訂閱者的值必須等於監視器最終狀態（沒有舊值蓋掉新值）。
        assert_eq!(*lock(&last), m.current());
        let expected = sources.iter().find_map(|s| lock(&s.value).clone());
        assert_eq!(m.current().map(|s| s.title), expected.map(|s| s.title));
    }

    /// 回歸：訂閱時的重播必須是串流的一部分。
    /// 送通知的執行緒卡在訂閱者 S0 的回呼裡（已送出 A）時，主執行緒再發布 B、C 並新增訂閱者 S1：
    /// S1 的第一次呼叫必須是登記當下的 `current`（C）；不得先收到舊值 A（過時），
    /// 也不得在 C 之後又收到 B 或 C（重複）。全程以 channel 手動排定交錯，沒有 sleep。
    #[test]
    fn subscriber_added_while_another_thread_is_draining_gets_current_then_only_newer() {
        use std::sync::mpsc;
        let primary = FakeSource::new("primary");
        let m = Arc::new(monitor(vec![primary.clone()]));

        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let entered_tx = Mutex::new(entered_tx);
        let release_rx = Mutex::new(release_rx);
        let s0_seen = Arc::new(Mutex::new(Vec::new()));
        let s0 = s0_seen.clone();
        let _s0_sub = m.subscribe(Box::new(move |v| {
            let title = v.as_ref().map(|x| x.title.clone());
            lock(&s0).push(title.clone());
            if title.as_deref() == Some("A") {
                lock(&entered_tx).send(()).unwrap();
                lock(&release_rx).recv().unwrap();
            }
        }));

        let p = primary.clone();
        let drainer = std::thread::spawn(move || p.emit(playing("A", "primary")));
        // 此刻 drainer 已把 A 交給 S0 並卡在回呼裡（持有「正在送」的身分，不持任何鎖）。
        entered_rx.recv().unwrap();

        primary.emit(playing("B", "primary")); // 只排進佇列，立刻返回
        primary.emit(playing("C", "primary"));
        let s1_seen = Arc::new(Mutex::new(Vec::new()));
        let s1 = s1_seen.clone();
        let _s1_sub = m.subscribe(Box::new(move |v| {
            lock(&s1).push(v.as_ref().map(|x| x.title.clone()))
        }));
        // 有別的執行緒在送：重播排在佇列裡，S1 目前還沒收到任何東西，也沒有被舊值 A 搶先。
        assert!(lock(&s1_seen).is_empty());

        release_tx.send(()).unwrap();
        drainer.join().unwrap();

        let titles = |seen: &Arc<Mutex<Vec<Option<String>>>>| lock(seen).clone();
        assert_eq!(
            titles(&s1_seen),
            vec![Some("C".to_string())],
            "S1: first call is the replay of current-at-registration, then nothing stale or duplicated"
        );
        assert_eq!(
            titles(&s0_seen),
            vec![
                None,
                Some("A".to_string()),
                Some("B".to_string()),
                Some("C".to_string())
            ]
        );

        // 之後的更新照常只送一次、依序送達兩位訂閱者。
        primary.emit(playing("D", "primary"));
        assert_eq!(
            titles(&s1_seen),
            vec![Some("C".to_string()), Some("D".to_string())]
        );
    }

    #[test]
    fn panicking_subscriber_does_not_wedge_later_notifications() {
        let primary = FakeSource::new("primary");
        let m = monitor(vec![primary.clone()]);
        let _bad = m.subscribe(Box::new(|v| {
            if v.as_ref().is_some_and(|s| s.title == "boom") {
                panic!("subscriber panic (expected by test)");
            }
        }));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let _good = m.subscribe(Box::new(move |v| {
            lock(&s).push(v.as_ref().map(|x| x.title.clone()))
        }));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            primary.emit(playing("boom", "primary"))
        }));
        assert!(result.is_err());
        // 進行中的 "boom" 對排在 panic 者之後的訂閱者遺失（已記載），但之後的更新照常送達。
        primary.emit(playing("next", "primary"));

        assert_eq!(
            *lock(&seen),
            vec![None, Some("next".to_string())],
            "later update still delivered after a subscriber panicked"
        );
        assert_eq!(title_of(&m).as_deref(), Some("next"));
    }
}
