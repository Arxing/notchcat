//! 把多個現正播放來源合成單一 `current` 快照（移植自 Swift `NowPlayingMonitor.swift`）。
//!
//! 來源依優先序列出。最新值非 `None` 的第一個來源勝出，即使它是暫停狀態；
//! 尚未發布過的來源視為 `None`。勝出者的封面由 [`ArtworkCache`] 補齊。
//!
//! 並行：來源可能在任意執行緒上發布。狀態（各來源最新值、目前結果、封面快取）由單一 mutex 保護，
//! 在鎖內算出結果並排入待送佇列，**鎖外**才通知訂閱者。通知由「同一時間只有一條執行緒」依佇列順序送出，
//! 所以訂閱者看到的順序與狀態變化的順序一致，且訂閱者回呼內可以再讀 `current()`、訂閱，
//! 甚至讓來源再發布（該次更新只會排進佇列，由正在送的執行緒接著送），都不會死結。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

use super::{NowPlayingSource, SharedSnapshotCallback, SnapshotCallback};
use crate::artwork_cache::ArtworkCache;
use crate::runtime::{lock, Notifier, Subscription};
use crate::types::TrackSnapshot;

struct State {
    /// 各來源的最新值，索引與 `new` 收到的 `sources` 相同。
    latest: Vec<Option<TrackSnapshot>>,
    current: Option<TrackSnapshot>,
    artwork: ArtworkCache,
    /// 已算出、尚未通知的結果（依產生順序）。
    pending: VecDeque<Option<TrackSnapshot>>,
    /// 最後一個已交給通知流程的結果；新訂閱者以它重播，之後佇列裡的值會依序接上。
    delivered: Option<TrackSnapshot>,
    /// 是否已有一條執行緒正在送通知。
    draining: bool,
}

struct Inner {
    state: Mutex<State>,
    notifier: Notifier<Option<TrackSnapshot>>,
}

/// 通知流程若因訂閱者 panic 而中途離開，要把 `draining` 放掉，否則之後再也沒有人會送通知。
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
                st.pending.push_back(winner);
            }
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
            let next = {
                let mut st = lock(&self.state);
                match st.pending.pop_front() {
                    Some(v) => {
                        st.delivered = v.clone();
                        v
                    }
                    None => {
                        st.draining = false;
                        guard.armed = false;
                        return;
                    }
                }
            };
            self.notifier.notify(&next);
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
                delivered: None,
                draining: false,
            }),
            notifier: Notifier::new(),
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

    /// 訂閱 `current` 的變動：訂閱當下立即以最新結果呼叫一次（同 Combine 的 `@Published`），
    /// 之後只在結果真的改變時呼叫。drop 回傳的把手即取消。
    pub fn subscribe(&self, f: SnapshotCallback) -> Subscription {
        let f: SharedSnapshotCallback = Arc::from(f);
        let forward = f.clone();
        // 在狀態鎖內登記並取重播值，才不會漏掉或重複登記前後之間的更新。
        let (subscription, replay) = {
            let st = lock(&self.inner.state);
            let sub = self.inner.notifier.subscribe(Box::new(move |v| forward(v)));
            (sub, st.delivered.clone())
        };
        f(&replay);
        subscription
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
}
