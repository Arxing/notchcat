//! 音樂模組的狀態（移植自 Swift `Modules/Music/MusicModule.swift`；畫面在 `shell`）。
//!
//! - `is_active` = 目前快照的 `is_playing`（沒有快照或暫停都不是），同 Swift `monitor.isPlayingPublisher`。
//! - Preview 額外高度 = [`LYRIC_STRIP_HEIGHT`] 當且僅當 `is_playing && strip_visible`，否則 0。
//!   沒有同步歌詞時歌詞條改顯示曲名，高度不變，所以高度與歌詞內容無關。
//! - 訂閱者只在 `is_active` 或額外高度真的改變時被通知（Swift `removeDuplicates`）。
//!
//! 並行：`last`（上次通知的值）的 mutex 同時串行化「讀控制器狀態、與上次比較、存入」，
//! 所以不同執行緒的更新不會把舊值寫在新值之後；通知在鎖外。控制器訂閱的回呼只持有內部狀態的 `Weak`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::modules::ModuleState;
use crate::music::controller::MusicController;
use crate::nowplaying::NowPlayingSource;
use crate::runtime::{lock, Notifier, Subscription};

/// Preview 中瀏海下方歌詞條的高度（需實機調整）。
pub const LYRIC_STRIP_HEIGHT: f64 = 24.0;
/// 歌詞條黑底的不透明度（需實機調整）。
pub const LYRIC_STRIP_OPACITY: f64 = 0.6;

pub struct MusicModuleState {
    inner: Arc<Inner>,
    /// 對控制器的訂閱；drop 時一併取消。包在 mutex 裡只是為了讓本型別是 `Sync`。
    _controller_subscription: Mutex<Option<Subscription>>,
}

struct Inner {
    controller: Arc<MusicController>,
    strip_visible: Arc<AtomicBool>,
    sources: Vec<Arc<dyn NowPlayingSource>>,
    last: Mutex<(bool, f64)>,
    notifier: Notifier<()>,
}

impl Inner {
    /// `(is_active, preview_extra_height)`，每次都從控制器與 `strip_visible` 現讀。
    fn compute(&self) -> (bool, f64) {
        let playing = self
            .controller
            .state()
            .snapshot
            .is_some_and(|snapshot| snapshot.is_playing);
        let visible = self.strip_visible.load(Ordering::SeqCst);
        (
            playing,
            if playing && visible {
                LYRIC_STRIP_HEIGHT
            } else {
                0.0
            },
        )
    }

    /// 重讀狀態，與上次通知的值不同才通知（鎖外）。
    fn refresh(&self) {
        let changed = {
            let mut last = lock(&self.last);
            let now = self.compute();
            let changed = *last != now;
            *last = now;
            changed
        };
        if changed {
            self.notifier.notify(&());
        }
    }
}

impl MusicModuleState {
    pub const ID: &'static str = "com.musicat.music";
    pub const PRIORITY: i32 = 100;

    /// 注意：模組只持有控制器，而控制器不持有監視器（見 [`MusicController::new`]）。
    /// 外殼必須在模組存活期間自行持有 `Arc<NowPlayingMonitor>`；否則快照更新會悄悄停止，
    /// `is_active` 卡在最後的值，沒有任何錯誤。
    ///
    /// `strip_visible` 與設定共用：歌詞條是否展開（沒有被收起）。`sources` 依優先序，
    /// 由 [`ModuleState::on_start`]／[`ModuleState::on_stop`] 啟動與停止（來源只能在主執行緒上啟停，
    /// 由呼叫端保證）。
    pub fn new(
        controller: Arc<MusicController>,
        strip_visible: Arc<AtomicBool>,
        sources: Vec<Arc<dyn NowPlayingSource>>,
    ) -> Self {
        let inner = Arc::new(Inner {
            controller,
            strip_visible,
            sources,
            last: Mutex::new((false, 0.0)),
            notifier: Notifier::new(),
        });
        *lock(&inner.last) = inner.compute();
        // 先建好 Arc 再訂閱；回呼只持弱參照。控制器不會在訂閱時重播，所以訂閱後再 refresh 一次，
        // 補上「讀初始值」與「訂閱」之間可能漏掉的變動（冪等）。
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        let subscription = inner.controller.subscribe(Box::new(move |_state| {
            if let Some(inner) = weak.upgrade() {
                inner.refresh();
            }
        }));
        inner.refresh();
        Self {
            inner,
            _controller_subscription: Mutex::new(Some(subscription)),
        }
    }

    /// 歌詞條展開或收起：寫入共用的旗標並立刻更新額外高度。
    /// 別處直接改旗標不會通知；要讓高度立即跟上請走這個方法。
    pub fn set_strip_visible(&self, visible: bool) {
        self.inner.strip_visible.store(visible, Ordering::SeqCst);
        self.inner.refresh();
    }
}

impl ModuleState for MusicModuleState {
    fn id(&self) -> &str {
        Self::ID
    }

    fn priority(&self) -> i32 {
        Self::PRIORITY
    }

    fn is_active(&self) -> bool {
        self.inner.compute().0
    }

    fn preview_extra_height(&self) -> f64 {
        self.inner.compute().1
    }

    fn subscribe(&self, f: Box<dyn Fn() + Send + Sync>) -> Subscription {
        self.inner.notifier.subscribe(Box::new(move |_| f()))
    }

    fn on_start(&self) {
        for source in &self.inner.sources {
            source.start();
        }
    }

    fn on_stop(&self) {
        for source in &self.inner.sources {
            source.stop();
        }
    }

    /// 同 `on_stop`；停兩次無害。
    fn on_remove(&self) {
        self.on_stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artwork_cache::ArtworkCache;
    use crate::lyrics::identity::TrackIdentity;
    use crate::lyrics::repository::LyricsResolving;
    use crate::lyrics::Lyrics;
    use crate::nowplaying::{NowPlayingMonitor, SharedSnapshotCallback, SnapshotCallback};
    use crate::runtime::BoxFuture;
    use crate::testing::LocalSpawner;
    use crate::types::TrackSnapshot;
    use std::sync::atomic::AtomicUsize;

    /// 對應 Swift 的 `FakeSource`（`CurrentValueSubject`）：訂閱當下重播目前值；另外計算 start／stop 次數。
    struct FakeSource {
        value: Mutex<Option<TrackSnapshot>>,
        notifier: Notifier<Option<TrackSnapshot>>,
        started: AtomicUsize,
        stopped: AtomicUsize,
    }

    impl FakeSource {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                value: Mutex::new(None),
                notifier: Notifier::new(),
                started: AtomicUsize::new(0),
                stopped: AtomicUsize::new(0),
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
        fn start(&self) {
            self.started.fetch_add(1, Ordering::SeqCst);
        }
        fn stop(&self) {
            self.stopped.fetch_add(1, Ordering::SeqCst);
        }
        fn subscribe(&self, f: SnapshotCallback) -> Subscription {
            let f: SharedSnapshotCallback = Arc::from(f);
            let f2 = f.clone();
            let sub = self.notifier.subscribe(Box::new(move |v| f2(v)));
            let current = lock(&self.value).clone();
            f(&current);
            sub
        }
    }

    /// Swift `NoLyrics`。
    struct NoLyrics;

    impl LyricsResolving for NoLyrics {
        fn lyrics(&self, _track: TrackIdentity) -> BoxFuture<'_, Option<Lyrics>> {
            Box::pin(std::future::ready(None))
        }
    }

    struct Fixture {
        source: Arc<FakeSource>,
        strip_visible: Arc<AtomicBool>,
        spawner: LocalSpawner,
        /// 控制器只對監視器持弱參照，必須由外部（生產環境是外殼）持有監視器。
        _monitor: Arc<NowPlayingMonitor>,
        module: Arc<MusicModuleState>,
        /// 每次額外高度「通知」後記下的值；開頭是訂閱當下的初始值（對應 Swift 訂閱時的重播）。
        heights: Arc<Mutex<Vec<f64>>>,
        _sub: Subscription,
    }

    impl Fixture {
        fn new() -> Self {
            let source = FakeSource::new();
            let strip_visible = Arc::new(AtomicBool::new(true));
            let spawner = LocalSpawner::new();
            let monitor = Arc::new(NowPlayingMonitor::new(
                vec![source.clone() as Arc<dyn NowPlayingSource>],
                ArtworkCache::default(),
            ));
            let controller = MusicController::new(
                monitor.clone(),
                Arc::new(NoLyrics),
                None,
                Arc::new(spawner.clone()),
            );
            let module = Arc::new(MusicModuleState::new(
                controller,
                strip_visible.clone(),
                vec![source.clone() as Arc<dyn NowPlayingSource>],
            ));
            let heights = Arc::new(Mutex::new(vec![module.preview_extra_height()]));
            let (h, weak) = (heights.clone(), Arc::downgrade(&module));
            let sub = module.subscribe(Box::new(move || {
                // 訂閱者收到通知後重讀狀態，不信任傳入的值（模組的通知不帶值）。
                if let Some(module) = weak.upgrade() {
                    lock(&h).push(module.preview_extra_height());
                }
            }));
            Self {
                source,
                strip_visible,
                spawner,
                _monitor: monitor,
                module,
                heights,
                _sub: sub,
            }
        }

        fn play(&self, is_playing: bool) {
            self.source.send(Some(TrackSnapshot {
                title: "Song".to_string(),
                artist: None,
                album: None,
                duration: None,
                is_playing,
                position: None,
                artwork: None,
                source_id: "fake".to_string(),
            }));
            self.spawner.run_until_stalled();
        }

        fn last_height(&self) -> Option<f64> {
            lock(&self.heights).last().copied()
        }
    }

    #[test]
    fn constants() {
        let fx = Fixture::new();
        assert_eq!(LYRIC_STRIP_HEIGHT, 24.0);
        assert_eq!(LYRIC_STRIP_OPACITY, 0.6);
        assert_eq!(fx.module.id(), "com.musicat.music");
        assert_eq!(fx.module.priority(), 100);
    }

    #[test]
    fn extra_height_is_zero_when_inactive() {
        let fx = Fixture::new();
        assert_eq!(fx.last_height(), Some(0.0));
        fx.play(false);
        assert_eq!(fx.last_height(), Some(0.0));
        assert_eq!(fx.module.preview_extra_height(), 0.0);
    }

    #[test]
    fn extra_height_is_24_when_active_and_visible() {
        let fx = Fixture::new();
        fx.play(true);
        assert_eq!(fx.last_height(), Some(24.0));
        assert_eq!(fx.module.preview_extra_height(), 24.0);
    }

    #[test]
    fn extra_height_is_zero_when_folded() {
        let fx = Fixture::new();
        fx.module.set_strip_visible(false);
        fx.play(true);
        assert_eq!(fx.last_height(), Some(0.0));
        assert_eq!(fx.module.preview_extra_height(), 0.0);
    }

    #[test]
    fn folding_updates_height_immediately() {
        let fx = Fixture::new();
        fx.play(true);
        assert_eq!(fx.last_height(), Some(24.0));

        fx.module.set_strip_visible(false);
        assert_eq!(fx.last_height(), Some(0.0));

        fx.module.set_strip_visible(true);
        assert_eq!(fx.last_height(), Some(24.0));
    }

    #[test]
    fn extra_height_does_not_repeat_values() {
        let fx = Fixture::new();
        fx.play(true);
        fx.play(true);
        fx.module.set_strip_visible(true);
        assert_eq!(*lock(&fx.heights), [0.0, 24.0]);
    }

    #[test]
    fn is_active_follows_playback() {
        let fx = Fixture::new();
        let values = Arc::new(Mutex::new(vec![fx.module.is_active()]));
        let (v, weak) = (values.clone(), Arc::downgrade(&fx.module));
        let _sub = fx.module.subscribe(Box::new(move || {
            if let Some(module) = weak.upgrade() {
                lock(&v).push(module.is_active());
            }
        }));
        fx.play(true);
        fx.play(false);
        assert_eq!(*lock(&values), [false, true, false]);
    }

    #[test]
    fn on_remove_stops_sources_like_on_stop() {
        let fx = Fixture::new();
        fx.module.on_start();
        fx.module.on_stop();
        fx.module.on_remove();

        assert_eq!(fx.source.started.load(Ordering::SeqCst), 1);
        assert_eq!(fx.source.stopped.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unrelated_changes_do_not_notify() {
        let fx = Fixture::new();
        fx.play(true);
        let before = lock(&fx.heights).len();

        // 換曲、暫停位置等不影響 is_active 與高度的變化：不通知。
        fx.source.send(Some(TrackSnapshot {
            title: "Another".to_string(),
            artist: Some("Someone".to_string()),
            album: None,
            duration: Some(200.0),
            is_playing: true,
            position: None,
            artwork: None,
            source_id: "fake".to_string(),
        }));
        fx.spawner.run_until_stalled();
        fx.module.set_strip_visible(true);

        assert_eq!(lock(&fx.heights).len(), before);
    }

    #[test]
    fn folding_while_paused_changes_nothing_and_does_not_notify() {
        let fx = Fixture::new();
        fx.play(false);
        fx.module.set_strip_visible(false);
        fx.module.set_strip_visible(true);
        assert_eq!(*lock(&fx.heights), [0.0]);
    }

    #[test]
    fn a_track_already_playing_at_construction_is_active_immediately() {
        let fx = Fixture::new();
        fx.play(true);
        // 控制器只持有對監視器的弱參照：監視器要活到斷言結束。
        let monitor = Arc::new(NowPlayingMonitor::new(
            vec![fx.source.clone() as Arc<dyn NowPlayingSource>],
            ArtworkCache::default(),
        ));
        let controller = MusicController::new(
            monitor.clone(),
            Arc::new(NoLyrics),
            None,
            Arc::new(fx.spawner.clone()),
        );
        let late = MusicModuleState::new(controller, fx.strip_visible.clone(), vec![]);
        assert!(late.is_active());
        assert_eq!(late.preview_extra_height(), 24.0);
        drop(monitor);
    }

    #[test]
    fn works_through_the_module_manager() {
        use crate::modules::ModuleManager;
        let fx = Fixture::new();
        let manager = ModuleManager::new();
        manager.register(fx.module.clone());
        assert_eq!(manager.active_module_id(), None);
        assert_eq!(manager.preview_extra_height(), 0.0);

        fx.play(true);
        assert_eq!(
            manager.active_module_id().as_deref(),
            Some("com.musicat.music")
        );
        assert_eq!(manager.preview_extra_height(), 24.0);

        fx.module.set_strip_visible(false);
        assert_eq!(manager.preview_extra_height(), 0.0);
    }

    #[test]
    fn dropping_the_module_unsubscribes_from_the_controller() {
        let fx = Fixture::new();
        let weak = Arc::downgrade(&fx.module);
        let Fixture {
            source,
            spawner,
            module,
            _sub,
            ..
        } = fx;
        drop(_sub);
        drop(module);
        assert!(weak.upgrade().is_none(), "模組必須能被釋放");
        source.send(None); // 不得 panic
        spawner.run_until_stalled();
    }
}
