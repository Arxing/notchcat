//! 音樂控制器：餵給音樂畫面的目前快照、該曲目的歌詞，以及播放控制指令
//! （移植自 Swift `MusicViewModel.swift`）。
//!
//! 歌詞在**曲目改變**時才查（以 `TrackIdentity::cache_key` 判斷），只有播放狀態或位置錨點改變時不重查、
//! 也不清掉歌詞。曲目改變時：丟掉舊查詢的 [`TaskHandle`]（drop 即取消）、清空歌詞，
//! 有 identity 才用 [`Spawner`] 起新查詢。查詢完成時只在控制器「目前的 key」仍等於該查詢的 key 時才寫入
//! （過期結果守門）：即使舊查詢沒有被取消（例如執行器沒有原生取消），晚到的結果也不會蓋掉新歌。
//!
//! 並行：狀態（快照、歌詞、目前 key、查詢把手）由單一 mutex 保護；通知訂閱者一律在**鎖外**，
//! 所以回呼內可以再讀 `state()` 或訂閱。`spawn` 與舊把手的 drop 也都在鎖外
//! （有的執行器會在 `spawn` 內就地執行，或在 drop 把手時同步執行取消邏輯）。
//! 監視器訂閱的回呼與查詢工作都只持有控制器的 `Weak`：控制器 drop 時沒有 Arc 循環，
//! 把手隨之 drop 而取消進行中的查詢。
//!
//! 監視器的 `subscribe` 會重播目前值，但重播可能在 `subscribe` 返回之後才抵達（別的執行緒正在送通知時），
//! 所以初始值的處理是冪等的：同一個快照再套用一次不產生任何變化。
//!
//! 通知的先後：監視器的回呼本身已被序列化；查詢完成的通知則來自執行器的執行緒。
//! 外殼讓兩者都在主執行緒上，所以訂閱者看到的順序與狀態變化一致。

use std::sync::{Arc, Mutex, Weak};

use crate::lyrics::normalizer::identity_from;
use crate::lyrics::repository::LyricsResolving;
use crate::lyrics::Lyrics;
use crate::nowplaying::{NowPlayingMonitor, PlaybackCommand, PlaybackControl};
use crate::runtime::{lock, Notifier, Spawner, Subscription, TaskHandle};
use crate::types::TrackSnapshot;

/// 音樂畫面讀取的狀態：目前快照，與（已查到的）該曲目歌詞。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MusicState {
    pub snapshot: Option<TrackSnapshot>,
    pub lyrics: Option<Lyrics>,
}

#[derive(Default)]
struct Core {
    state: MusicState,
    /// 目前曲目的 `cache_key`；沒有 identity（沒有快照、缺演出者…）時為 `None`。
    current_key: Option<String>,
    /// 進行中的查詢；drop 即取消。
    lookup: Option<TaskHandle>,
}

pub struct MusicController {
    core: Mutex<Core>,
    notifier: Notifier<MusicState>,
    resolver: Arc<dyn LyricsResolving>,
    control: Option<Arc<dyn PlaybackControl>>,
    spawner: Arc<dyn Spawner>,
    /// 對監視器的訂閱；控制器 drop 時一併取消。包在 mutex 裡只是為了建構後再放進去、並讓控制器是 `Sync`。
    _monitor_subscription: Mutex<Option<Subscription>>,
}

impl MusicController {
    pub fn new(
        monitor: Arc<NowPlayingMonitor>,
        resolver: Arc<dyn LyricsResolving>,
        control: Option<Arc<dyn PlaybackControl>>,
        spawner: Arc<dyn Spawner>,
    ) -> Arc<Self> {
        let controller = Arc::new(Self {
            core: Mutex::new(Core::default()),
            notifier: Notifier::new(),
            resolver,
            control,
            spawner,
            _monitor_subscription: Mutex::new(None),
        });
        // 先建好 Arc 再訂閱：重播可能在 subscribe 內同步抵達，那時回呼需要能 upgrade。
        // 回呼只持弱參照，監視器活得比控制器久也不會撐住它。
        let weak: Weak<Self> = Arc::downgrade(&controller);
        let subscription = monitor.subscribe(Box::new(move |snapshot| {
            if let Some(controller) = weak.upgrade() {
                controller.apply(snapshot.clone());
            }
        }));
        *lock(&controller._monitor_subscription) = Some(subscription);
        controller
    }

    pub fn state(&self) -> MusicState {
        lock(&self.core).state.clone()
    }

    /// 訂閱狀態變動（快照或歌詞真的改變才通知）。不會在訂閱當下重播目前值；要的話先讀 [`Self::state`]。
    /// 回呼在鎖外執行；drop 回傳的把手即取消。
    pub fn subscribe(&self, f: Box<dyn Fn(&MusicState) + Send + Sync>) -> Subscription {
        self.notifier.subscribe(f)
    }

    /// 播放控制指令是否到得了任何東西；沒有時按鈕應停用。
    pub fn has_control(&self) -> bool {
        self.control.is_some()
    }

    /// 無 control 時為 no-op。
    pub fn send(&self, cmd: PlaybackCommand) {
        if let Some(control) = &self.control {
            control.send(cmd);
        }
    }

    fn apply(self: &Arc<Self>, new: Option<TrackSnapshot>) {
        let identity = new.as_ref().and_then(identity_from);
        let key = identity.as_ref().map(|i| i.cache_key());

        let (changed, state, old_lookup, key_changed) = {
            let mut core = lock(&self.core);
            let key_changed = core.current_key != key;
            let before = core.state.clone();
            core.state.snapshot = new;
            let mut old_lookup = None;
            if key_changed {
                core.current_key = key.clone();
                old_lookup = core.lookup.take();
                core.state.lyrics = None;
            }
            (
                core.state != before,
                core.state.clone(),
                old_lookup,
                key_changed,
            )
        };
        // 舊查詢在鎖外取消。
        drop(old_lookup);

        if key_changed {
            if let (Some(identity), Some(key)) = (identity, key) {
                self.start_lookup(identity, key);
            }
        }
        if changed {
            self.notifier.notify(&state);
        }
    }

    fn start_lookup(
        self: &Arc<Self>,
        identity: crate::lyrics::identity::TrackIdentity,
        key: String,
    ) {
        let resolver = self.resolver.clone();
        let weak: Weak<Self> = Arc::downgrade(self);
        let task_key = key.clone();
        let handle = self.spawner.spawn(Box::pin(async move {
            let result = resolver.lyrics(identity).await;
            if let Some(controller) = weak.upgrade() {
                controller.finish_lookup(&task_key, result);
            }
        }));
        let stale = {
            let mut core = lock(&self.core);
            if core.current_key.as_deref() == Some(key.as_str()) {
                core.lookup = Some(handle);
                None
            } else {
                // spawn 期間曲目又變了：這個查詢已過期，不保留把手（drop 即取消）。
                Some(handle)
            }
        };
        drop(stale);
    }

    /// 查詢完成：曲目可能在請求期間改變，key 不同就丟棄。
    fn finish_lookup(&self, key: &str, result: Option<Lyrics>) {
        let state = {
            let mut core = lock(&self.core);
            if core.current_key.as_deref() != Some(key) || core.state.lyrics == result {
                return;
            }
            core.state.lyrics = result;
            core.state.clone()
        };
        self.notifier.notify(&state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artwork_cache::ArtworkCache;
    use crate::lyrics::identity::TrackIdentity;
    use crate::nowplaying::{NowPlayingSource, SharedSnapshotCallback, SnapshotCallback};
    use crate::runtime::{BoxFuture, Notifier};
    use crate::testing::LocalSpawner;
    use crate::types::PositionAnchor;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Waker};

    /// 對應 Swift 的 `CurrentValueSubject` 假來源：訂閱當下立即重播目前值。
    struct FakeSource {
        value: Mutex<Option<TrackSnapshot>>,
        notifier: Notifier<Option<TrackSnapshot>>,
    }

    impl FakeSource {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                value: Mutex::new(None),
                notifier: Notifier::new(),
            })
        }

        fn send(&self, v: Option<TrackSnapshot>) {
            *lock(&self.value) = v.clone();
            self.notifier.notify(&v);
        }
    }

    impl NowPlayingSource for FakeSource {
        fn id(&self) -> &str {
            "fake"
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

    /// 一次查詢的閘門：test 呼叫 `answer` 之前，對應的 future 一直 Pending。
    #[derive(Default)]
    struct Gate {
        slot: Mutex<GateSlot>,
    }

    #[derive(Default)]
    struct GateSlot {
        answer: Option<Option<Lyrics>>,
        waker: Option<Waker>,
    }

    impl Gate {
        fn answer(&self, lyrics: Option<Lyrics>) {
            let waker = {
                let mut slot = lock(&self.slot);
                slot.answer = Some(lyrics);
                slot.waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        }
    }

    /// 等閘門被回答；被 drop 時（取消）累加 `dropped`。
    struct GatedLookup {
        gate: Arc<Gate>,
        dropped: Arc<AtomicUsize>,
    }

    impl Future for GatedLookup {
        type Output = Option<Lyrics>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut slot = lock(&self.gate.slot);
            match slot.answer.take() {
                Some(a) => Poll::Ready(a),
                None => {
                    slot.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    impl Drop for GatedLookup {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 記錄每次查詢並卡住，直到 test 回答（對應 Swift 的 `GatedResolver`）。
    #[derive(Default)]
    struct GatedResolver {
        tracks: Mutex<Vec<TrackIdentity>>,
        gates: Mutex<Vec<Arc<Gate>>>,
        dropped: Arc<AtomicUsize>,
    }

    impl GatedResolver {
        fn requested_titles(&self) -> Vec<String> {
            lock(&self.tracks).iter().map(|t| t.title.clone()).collect()
        }

        fn requested(&self) -> Vec<TrackIdentity> {
            lock(&self.tracks).clone()
        }

        /// 回答第 `index` 次查詢（依到達順序）。
        fn respond(&self, index: usize, lyrics: Option<Lyrics>) {
            let gate = lock(&self.gates)[index].clone();
            gate.answer(lyrics);
        }
    }

    impl LyricsResolving for GatedResolver {
        fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<Lyrics>> {
            let gate = Arc::new(Gate::default());
            lock(&self.tracks).push(track);
            lock(&self.gates).push(gate.clone());
            Box::pin(GatedLookup {
                gate,
                dropped: self.dropped.clone(),
            })
        }
    }

    #[derive(Default)]
    struct FakeControl {
        sent: Mutex<Vec<PlaybackCommand>>,
    }

    impl PlaybackControl for FakeControl {
        fn send(&self, cmd: PlaybackCommand) {
            lock(&self.sent).push(cmd);
        }
    }

    fn snapshot(
        title: &str,
        artist: Option<&str>,
        is_playing: bool,
        position: Option<PositionAnchor>,
    ) -> Option<TrackSnapshot> {
        Some(TrackSnapshot {
            title: title.to_string(),
            artist: artist.map(str::to_string),
            album: None,
            duration: Some(200.0),
            is_playing,
            position,
            artwork: None,
            source_id: "fake".to_string(),
        })
    }

    fn song(title: &str) -> Option<TrackSnapshot> {
        snapshot(title, Some("Artist"), true, None)
    }

    fn plain(s: &str) -> Option<Lyrics> {
        Some(Lyrics::Plain(s.to_string()))
    }

    struct Fixture {
        source: Arc<FakeSource>,
        resolver: Arc<GatedResolver>,
        spawner: LocalSpawner,
        monitor: Arc<NowPlayingMonitor>,
    }

    impl Fixture {
        fn new() -> Self {
            let source = FakeSource::new();
            let monitor = Arc::new(NowPlayingMonitor::new(
                vec![source.clone() as Arc<dyn NowPlayingSource>],
                ArtworkCache::default(),
            ));
            Self {
                source,
                resolver: Arc::new(GatedResolver::default()),
                spawner: LocalSpawner::new(),
                monitor,
            }
        }

        fn controller(&self, control: Option<Arc<dyn PlaybackControl>>) -> Arc<MusicController> {
            MusicController::new(
                self.monitor.clone(),
                self.resolver.clone(),
                control,
                Arc::new(self.spawner.clone()),
            )
        }

        /// 送出快照並讓背景工作跑到停滯（取代 Swift 的 `waitUntil`，沒有睡眠、沒有逾時）。
        fn emit(&self, v: Option<TrackSnapshot>) {
            self.source.send(v);
            self.spawner.run_until_stalled();
        }

        fn respond(&self, index: usize, lyrics: Option<Lyrics>) {
            self.resolver.respond(index, lyrics);
            self.spawner.run_until_stalled();
        }
    }

    #[test]
    fn fetches_lyrics_on_track_change() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));

        let requested = fx.resolver.requested();
        assert_eq!(requested.len(), 1);
        assert_eq!(requested[0].title, "Song A");
        assert_eq!(requested[0].artist, "Artist");
        assert_eq!(controller.state().snapshot.unwrap().title, "Song A");

        fx.respond(0, plain("words"));
        assert_eq!(controller.state().lyrics, plain("words"));
    }

    #[test]
    fn changing_only_play_state_or_position_does_not_start_a_new_lookup() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        assert_eq!(fx.resolver.requested().len(), 1);
        fx.respond(0, plain("words"));
        assert!(controller.state().lyrics.is_some());

        // 只有播放狀態與位置錨點改變。
        fx.emit(snapshot("Song A", Some("Artist"), false, None));
        let anchor = PositionAnchor {
            elapsed: 30.0,
            timestamp: 1_000.0,
            rate: 1.0,
        };
        fx.emit(snapshot("Song A", Some("Artist"), true, Some(anchor)));
        assert_eq!(controller.state().lyrics, plain("words"));
        assert!(controller.state().snapshot.unwrap().is_playing);
        assert_eq!(fx.resolver.requested().len(), 1, "同一首歌不得重查");

        // 之後換歌作為哨兵：若 Song A 被重查，它會排在 Song B 之前被記錄。
        fx.emit(song("Song B"));
        assert_eq!(fx.resolver.requested_titles(), ["Song A", "Song B"]);
    }

    #[test]
    fn slow_lookup_for_the_previous_track_never_overwrites_the_new_tracks_lyrics() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        assert_eq!(fx.resolver.requested().len(), 2);
        fx.respond(1, plain("lyrics of B"));
        assert_eq!(controller.state().lyrics, plain("lyrics of B"));

        // A 的請求在換歌之後才回來。放棄的請求被 drop，晚到的回答沒有人接；
        // 就算它還活著，current_key 比對也會擋下（見下一個測試的另一條路徑）。
        fx.respond(0, plain("lyrics of A"));

        assert_eq!(controller.state().lyrics, plain("lyrics of B"));
    }

    /// 上一個測試靠「換歌時取消舊查詢」擋掉舊回答；這裡的 spawner 刻意不取消（把手被 detach 的情境），
    /// 單靠完成時的 key 比對也必須擋住。
    #[test]
    fn stale_result_is_rejected_by_key_even_when_the_old_lookup_is_not_cancelled() {
        struct DetachingSpawner(LocalSpawner);
        impl Spawner for DetachingSpawner {
            fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle {
                self.0.spawn(f).detach();
                TaskHandle::new(|| {})
            }
        }
        let fx = Fixture::new();
        let controller = MusicController::new(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(DetachingSpawner(fx.spawner.clone())),
        );
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        fx.respond(1, plain("lyrics of B"));
        fx.respond(0, plain("lyrics of A"));
        assert_eq!(controller.state().lyrics, plain("lyrics of B"));

        // C 還在查；A、B 兩個（未被取消的）舊查詢都已回完，C 的歌詞不得被它們填入。
        fx.emit(song("Song C"));
        assert_eq!(fx.resolver.requested().len(), 3);
        assert_eq!(controller.state().lyrics, None);
    }

    #[test]
    fn stale_response_is_discarded_while_current_track_is_still_loading() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        assert_eq!(fx.resolver.requested().len(), 2);

        fx.respond(0, plain("lyrics of A"));

        assert_eq!(controller.state().lyrics, None);
    }

    #[test]
    fn track_change_clears_previous_lyrics() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain("lyrics of A"));
        assert!(controller.state().lyrics.is_some());

        fx.emit(song("Song B"));

        assert_eq!(controller.state().lyrics, None);
    }

    #[test]
    fn track_without_artist_clears_lyrics_and_starts_no_lookup() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain("lyrics of A"));
        assert!(controller.state().lyrics.is_some());

        fx.emit(snapshot("No Artist Song", None, true, None));
        assert_eq!(controller.state().snapshot.unwrap().title, "No Artist Song");
        assert_eq!(controller.state().lyrics, None);

        // 哨兵：之後唯一到得了 resolver 的是下一首。
        fx.emit(song("With Artist"));
        assert_eq!(fx.resolver.requested_titles(), ["Song A", "With Artist"]);
    }

    #[test]
    fn no_snapshot_clears_lyrics() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain("lyrics of A"));
        assert!(controller.state().lyrics.is_some());

        fx.emit(None);

        assert_eq!(controller.state().snapshot, None);
        assert_eq!(controller.state().lyrics, None);
    }

    #[test]
    fn send_forwards_to_control() {
        let fx = Fixture::new();
        let control = Arc::new(FakeControl::default());
        let controller = fx.controller(Some(control.clone()));

        controller.send(PlaybackCommand::Next);
        controller.send(PlaybackCommand::TogglePlayPause);

        assert_eq!(
            *lock(&control.sent),
            [PlaybackCommand::Next, PlaybackCommand::TogglePlayPause]
        );
    }

    #[test]
    fn send_without_control_is_no_op() {
        let fx = Fixture::new();
        let controller = fx.controller(None);

        controller.send(PlaybackCommand::Next); // 不得 panic
    }

    #[test]
    fn has_control_reflects_availability() {
        let fx = Fixture::new();
        assert!(!fx.controller(None).has_control());
        let control: Arc<dyn PlaybackControl> = Arc::new(FakeControl::default());
        assert!(fx.controller(Some(control)).has_control());
    }

    #[test]
    fn dropping_the_controller_cancels_the_inflight_lookup() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        let weak = Arc::downgrade(&controller);
        fx.emit(song("Song A"));
        assert_eq!(fx.resolver.requested().len(), 1);
        assert_eq!(fx.resolver.dropped.load(Ordering::SeqCst), 0);

        drop(controller);
        fx.spawner.run_until_stalled();

        assert!(
            weak.upgrade().is_none(),
            "控制器必須能被釋放（沒有 Arc 循環）"
        );
        assert_eq!(
            fx.resolver.dropped.load(Ordering::SeqCst),
            1,
            "進行中的查詢被取消"
        );
        assert_eq!(fx.spawner.pending_tasks(), 0);
    }

    #[test]
    fn track_change_cancels_the_previous_lookup() {
        let fx = Fixture::new();
        let _controller = fx.controller(None);
        fx.emit(song("Song A"));
        assert_eq!(fx.resolver.dropped.load(Ordering::SeqCst), 0);

        fx.emit(song("Song B"));

        assert_eq!(fx.resolver.dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_track_already_playing_when_the_controller_is_created_is_picked_up() {
        let fx = Fixture::new();
        fx.source.send(song("Song A"));
        let controller = fx.controller(None);
        fx.spawner.run_until_stalled();

        assert_eq!(controller.state().snapshot.unwrap().title, "Song A");
        assert_eq!(fx.resolver.requested_titles(), ["Song A"]);
    }

    #[test]
    fn subscribers_hear_about_snapshot_and_lyrics_changes_but_not_repeats() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        let seen: Arc<Mutex<Vec<MusicState>>> = Arc::default();
        let s = seen.clone();
        let _sub = controller.subscribe(Box::new(move |state| lock(&s).push(state.clone())));

        fx.emit(song("Song A"));
        assert_eq!(lock(&seen).len(), 1);
        fx.emit(song("Song A")); // 完全相同：不通知（監視器本身也不會重送）
        assert_eq!(lock(&seen).len(), 1);
        fx.respond(0, plain("words"));
        assert_eq!(lock(&seen).len(), 2);
        assert_eq!(lock(&seen)[1].lyrics, plain("words"));
        // 查無結果且歌詞本來就是 None：沒有變化，不通知。
        fx.emit(song("Song B"));
        assert_eq!(lock(&seen).len(), 3);
        assert_eq!(lock(&seen)[2].lyrics, None);
        fx.respond(1, None);
        assert_eq!(lock(&seen).len(), 3);
    }

    #[test]
    fn subscribers_are_called_without_the_controller_lock_held() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        let reads = Arc::new(AtomicUsize::new(0));
        let (c, r) = (Arc::downgrade(&controller), reads.clone());
        let _sub = controller.subscribe(Box::new(move |_| {
            // 回呼內讀狀態：若通知時還持有控制器的鎖就會死結。
            let controller = c.upgrade().unwrap();
            let _ = controller.state();
            r.fetch_add(1, Ordering::SeqCst);
        }));

        fx.emit(song("Song A"));
        fx.respond(0, plain("words"));

        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }
}
