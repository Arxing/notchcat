//! 視窗標題備援來源（移植自 Swift `WindowTitleSource.swift`）：讀 YouTube Music 視窗的標題。
//!
//! 只回報標題：沒有演出者、專輯、曲長與封面；位置由 [`PlaybackEstimator`] 推估，
//! 且只有在看到換曲之後才有（已在播放的曲目起點未知，位置為 `None`，絕不是 0）。
//! 播放中每秒輪詢一次，否則每五秒一次。沒有任何符合的視窗（包括沒有螢幕錄製權限、
//! 視窗名稱都讀不到而列舉結果為空）時發布 `None`。
//!
//! 並行：Swift 版要求所有呼叫都在同一個佇列上；這裡 `start`／`stop` 可在任意執行緒呼叫，
//! 輪詢在注入的 [`Spawner`] 上跑。視窗列舉是阻塞的（約 9 ms），所以第一次輪詢也排在 spawner 上，
//! 不在 `start()` 的呼叫端執行緒上做。狀態放在單一 `Mutex<State>`，持鎖時只更新狀態、記下要做的事；
//! 列舉視窗、`Spawner::spawn`、`TaskHandle` 的取消與訂閱者回呼都在放鎖後才執行。
//!
//! 每次輪詢是一個 spawn 出去的工作（先 `clock.sleep` 再輪詢；第一次不用睡），把手放在 `State.timer_task`：
//! `stop()` 或新的 `start()` 把它 drop 即取消。`generation` 在 `start()` 與 `stop()` 各加一，
//! 工作醒來與列舉返回後都在鎖內重新檢查它，過期的輪詢什麼都不做（不發布、不再排程）。
//!
//! 發布用 `musicat_core::nowplaying::PublishedSnapshot`（與 `HelperSupervisor`、`NowPlayingMonitor` 共用）：
//! 在鎖內排入、放鎖後 flush；訂閱的重播保證見該模組。
//!
//! 視窗列舉的系統實作 [`CgWindowLister`] 只在 macOS 編譯；來源與 [`WindowLister`] 介面在所有目標上編譯並測試。
//! `CGWindowListCopyWindowInfo` 是公開的 CoreGraphics API，不在私有 API 圍欄的範圍內。

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use musicat_core::estimator::PlaybackEstimator;
use musicat_core::nowplaying::{
    parse_window_title, NowPlayingSource, ParsedTrack, PublishedSnapshot, SnapshotCallback,
    WindowInfo,
};
use musicat_core::runtime::{Clock, Spawner, Subscription, TaskHandle};
use musicat_core::types::TrackSnapshot;

/// 來源識別字（與 Swift `WindowTitleSource.id` 相同）。
pub const SOURCE_ID: &str = "windowtitle";

/// 有曲目在播放時的輪詢間隔。
pub const PLAYING_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 沒有曲目、或曲目暫停時的輪詢間隔。
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// 列出各應用程式有標題的視窗。阻塞呼叫，所以來源只在 spawner 的工作裡呼叫它。
pub trait WindowLister: Send + Sync {
    fn windows(&self) -> Vec<WindowInfo>;
}

/// 忽略 mutex 中毒：持鎖時不執行使用者程式碼，中毒只可能來自內部 panic，狀態仍一致。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct State {
    is_running: bool,
    /// `start()` 與 `stop()` 各加一，所以在兩者之前排定（或進行到一半）的輪詢會被忽略。
    generation: u64,
    estimator: PlaybackEstimator,
    /// 目前有效的輪詢工作 id；被取消、被取代或已開始執行時為 `None`。
    timer_id: Option<u64>,
    /// 目前有效輪詢工作的把手（drop 即取消）。
    timer_task: Option<TaskHandle>,
    next_timer_id: u64,
}

impl State {
    /// 換一個新的輪詢工作 id，並把舊的把手交還給呼叫端（要在放鎖後 drop）。
    fn begin_timer(&mut self) -> (u64, Option<TaskHandle>) {
        let id = self.next_timer_id;
        self.next_timer_id += 1;
        self.timer_id = Some(id);
        (id, self.timer_task.take())
    }
}

struct Inner {
    lister: Arc<dyn WindowLister>,
    clock: Arc<dyn Clock>,
    spawner: Arc<dyn Spawner>,
    published: PublishedSnapshot,
    state: Mutex<State>,
}

impl Inner {
    fn on_start(self: &Arc<Self>) {
        let (id, generation, old) = {
            let mut st = lock(&self.state);
            if st.is_running {
                return;
            }
            st.is_running = true;
            st.generation += 1;
            let (id, old) = st.begin_timer();
            (id, st.generation, old)
        };
        drop(old);
        self.spawn_poll(id, generation, None);
    }

    fn on_stop(self: &Arc<Self>) {
        let task = {
            let mut st = lock(&self.state);
            st.is_running = false;
            st.generation += 1;
            st.estimator = PlaybackEstimator::new();
            st.timer_id = None;
            // Swift `subject.send(nil)`：無條件發布。
            self.published.queue_publish(None);
            st.timer_task.take()
        };
        drop(task);
        self.published.flush();
    }

    /// 排一個輪詢工作：先睡 `delay`（`None` 表示立刻），醒來後輪詢。工作只持有弱參照。
    fn spawn_poll(self: &Arc<Self>, id: u64, generation: u64, delay: Option<Duration>) {
        let weak: Weak<Inner> = Arc::downgrade(self);
        let sleep = delay.map(|d| self.clock.sleep(d));
        let task = self.spawner.spawn(Box::pin(async move {
            if let Some(sleep) = sleep {
                sleep.await;
            }
            if let Some(inner) = weak.upgrade() {
                inner.poll(id, generation);
            }
        }));
        let stale = {
            let mut st = lock(&self.state);
            if st.timer_id == Some(id) {
                st.timer_task = Some(task);
                None
            } else {
                // 排程期間已被取消、被取代，或工作已經開始執行。
                Some(task)
            }
        };
        drop(stale);
    }

    fn poll(self: &Arc<Self>, id: u64, generation: u64) {
        {
            let mut st = lock(&self.state);
            if st.timer_id == Some(id) {
                st.timer_id = None;
                // 這個工作正在跑：把手 detach 而不是 drop，不在工作自己裡面取消自己。
                if let Some(task) = st.timer_task.take() {
                    task.detach();
                }
            }
            if !st.is_running || st.generation != generation {
                return;
            }
        }

        // 列舉約 9 ms 且阻塞：不持鎖。期間 stop()／start() 可以進來，所以返回後要重新檢查 generation。
        let windows = self.lister.windows();
        let now = self.clock.now();

        // 優先選正在播放的視窗，而不是只是開著的。
        let tracks: Vec<ParsedTrack> = windows.iter().filter_map(parse_window_title).collect();
        let track = tracks.iter().find(|t| t.is_playing).or(tracks.first());
        let is_playing = track.is_some_and(|t| t.is_playing);

        let (next_id, old) = {
            let mut st = lock(&self.state);
            if !st.is_running || st.generation != generation {
                return;
            }
            st.estimator
                .observe(track.map(|t| t.title.as_str()), is_playing, now);
            let snapshot = track.map(|t| TrackSnapshot {
                title: t.title.clone(),
                artist: None,
                album: None,
                duration: None,
                is_playing: t.is_playing,
                position: st.estimator.anchor(),
                artwork: None,
                source_id: SOURCE_ID.to_string(),
            });
            self.published.queue_set(snapshot);
            st.begin_timer()
        };
        drop(old);
        self.published.flush();

        let interval = if is_playing {
            PLAYING_POLL_INTERVAL
        } else {
            IDLE_POLL_INTERVAL
        };
        self.spawn_poll(next_id, generation, Some(interval));
    }
}

/// 讀視窗標題的備援來源。行為見模組文件。
pub struct WindowTitleSource {
    inner: Arc<Inner>,
}

impl WindowTitleSource {
    /// 輪詢間隔是固定的（[`PLAYING_POLL_INTERVAL`]、[`IDLE_POLL_INTERVAL`]，與 Swift 版相同）。
    pub fn new(
        lister: Arc<dyn WindowLister>,
        clock: Arc<dyn Clock>,
        spawner: Arc<dyn Spawner>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                lister,
                clock,
                spawner,
                published: PublishedSnapshot::new(),
                state: Mutex::new(State {
                    is_running: false,
                    generation: 0,
                    estimator: PlaybackEstimator::new(),
                    timer_id: None,
                    timer_task: None,
                    next_timer_id: 0,
                }),
            }),
        })
    }
}

impl NowPlayingSource for WindowTitleSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    /// 已在執行時無害。第一次輪詢排在 spawner 上，不在呼叫端執行緒做。
    fn start(&self) {
        self.inner.on_start();
    }

    /// 停止輪詢、忘掉估算器狀態並發布 `None`，所以之後再 `start()` 不會重播這一輪的最後快照。
    fn stop(&self) {
        self.inner.on_stop();
    }

    fn subscribe(&self, f: SnapshotCallback) -> Subscription {
        self.inner.published.subscribe(f)
    }
}

/// 以 `CGWindowListCopyWindowInfo` 列出所有應用程式的視窗，包括其他 Space 與已最小化的
/// （`kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements`；每次呼叫約 9 ms）。
///
/// 擁有者名稱一定有；`kCGWindowName` 可能不存在或是空字串，這種視窗略過。
/// 沒有「螢幕錄製」權限時系統不給視窗名稱，所以結果是空陣列，來源因此發布 `None`；
/// 這裡不觸發、也不請求該權限。
#[cfg(target_os = "macos")]
pub struct CgWindowLister;

#[cfg(target_os = "macos")]
impl WindowLister for CgWindowLister {
    fn windows(&self) -> Vec<WindowInfo> {
        use core_foundation::base::{CFType, TCFType};
        use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
        use core_foundation::string::CFString;
        use core_graphics::window::{
            copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
            kCGWindowListOptionAll, kCGWindowName, kCGWindowOwnerName,
        };

        let Some(list) = copy_window_info(
            kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        ) else {
            return Vec::new();
        };
        // SAFETY: 這兩個是 CoreGraphics 匯出的常數 CFString，行程存活期間都有效；get rule 會自行 retain。
        let (owner_key, name_key) = unsafe {
            (
                CFString::wrap_under_get_rule(kCGWindowOwnerName),
                CFString::wrap_under_get_rule(kCGWindowName),
            )
        };

        let mut windows = Vec::new();
        for entry in list.iter() {
            // SAFETY: `CGWindowListCopyWindowInfo` 的陣列元素是 CFDictionary（文件保證）；元素由陣列擁有，
            // 以 get rule 取得會多 retain 一次，之後隨 `dict` 釋放。
            let dict: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(*entry as CFDictionaryRef) };
            let string = |key: &CFString| {
                dict.find(key)
                    .and_then(|value| value.downcast::<CFString>())
                    .map(|s| s.to_string())
            };
            let (Some(owner), Some(title)) = (string(&owner_key), string(&name_key)) else {
                continue;
            };
            if !title.is_empty() {
                windows.push(WindowInfo { owner, title });
            }
        }
        windows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicat_core::runtime::BoxFuture;
    use musicat_core::testing::{FakeClock, LocalSpawner};
    use musicat_core::types::{PositionAnchor, TrackSnapshot};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ---------------------------------------------------------------- 測試替身

    /// 對應 Swift 測試的 `windows` 閉包：測試改 `set`，來源每次輪詢時讀取；記錄被呼叫的次數與執行緒。
    #[derive(Default)]
    struct FakeLister {
        windows: Mutex<Vec<WindowInfo>>,
        calls: AtomicUsize,
        /// 設定時，下一次 `windows()` 先呼叫它（用來在「列舉進行中」插入 stop 等動作）。
        on_call: Mutex<Option<Box<dyn Fn() + Send>>>,
    }

    impl FakeLister {
        fn set(&self, windows: Vec<WindowInfo>) {
            *self.windows.lock().unwrap() = windows;
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl WindowLister for FakeLister {
        fn windows(&self) -> Vec<WindowInfo> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let hook = self.on_call.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
            self.windows.lock().unwrap().clone()
        }
    }

    /// 對應 Swift 的 `FakeScheduler.delays`：轉交給 `FakeClock`，並記下每次 `sleep` 的長度。
    struct RecordingClock {
        inner: FakeClock,
        delays: Mutex<Vec<Duration>>,
    }

    impl Clock for RecordingClock {
        fn now(&self) -> f64 {
            self.inner.now()
        }

        fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
            self.delays.lock().unwrap().push(d);
            self.inner.sleep(d)
        }
    }

    struct Harness {
        lister: Arc<FakeLister>,
        clock: Arc<RecordingClock>,
        spawner: LocalSpawner,
        source: Arc<WindowTitleSource>,
        published: Arc<Mutex<Vec<Option<TrackSnapshot>>>>,
        _sub: Subscription,
    }

    const T0: f64 = 1_000_000.0;

    impl Harness {
        fn new() -> Self {
            let lister = Arc::new(FakeLister::default());
            let clock = Arc::new(RecordingClock {
                inner: FakeClock::new(T0),
                delays: Mutex::new(Vec::new()),
            });
            let spawner = LocalSpawner::new();
            let source =
                WindowTitleSource::new(lister.clone(), clock.clone(), Arc::new(spawner.clone()));
            let published = Arc::new(Mutex::new(Vec::new()));
            let p = published.clone();
            let sub = source.subscribe(Box::new(move |v| p.lock().unwrap().push(v.clone())));
            Self {
                lister,
                clock,
                spawner,
                source,
                published,
                _sub: sub,
            }
        }

        /// 輪詢在 spawner 上跑：`start()` 之後要跑一次執行器，第一次輪詢才會發生。
        fn start(&self) {
            self.source.start();
            self.spawner.run_until_stalled();
        }

        fn stop(&self) {
            self.source.stop();
            self.spawner.run_until_stalled();
        }

        /// 對應 Swift `tick(advancing:)`：推進時間（喚醒到期的計時器）再跑到停滯。
        fn tick(&self, seconds: f64) {
            self.clock.inner.advance(Duration::from_secs_f64(seconds));
            self.spawner.run_until_stalled();
        }

        fn delays(&self) -> Vec<Duration> {
            self.clock.delays.lock().unwrap().clone()
        }

        fn published_count(&self) -> usize {
            self.published.lock().unwrap().len()
        }

        /// `published.last ?? nil`
        fn latest(&self) -> Option<TrackSnapshot> {
            self.published.lock().unwrap().last().cloned().flatten()
        }
    }

    /// 測試用的 YouTube Music 視窗。
    fn ytm(title: &str, playing: bool) -> WindowInfo {
        WindowInfo {
            owner: "YouTube Music".to_string(),
            title: format!(
                "YouTube Music - {title} | YouTube Music{}",
                if playing { " 🔊" } else { "" }
            ),
        }
    }

    fn window(owner: &str, title: &str) -> WindowInfo {
        WindowInfo {
            owner: owner.to_string(),
            title: title.to_string(),
        }
    }

    // ---------------------------------------------------------------- 移植自 WindowTitleSourceTests

    #[test]
    fn publishes_snapshot_for_matching_window() {
        let h = Harness::new();
        h.lister.set(vec![ytm("假日而已", true)]);
        h.start();
        let snapshot = h.latest().expect("a snapshot");
        assert_eq!(snapshot.title, "假日而已");
        assert_eq!(snapshot.artist, None);
        assert_eq!(snapshot.album, None);
        assert_eq!(snapshot.duration, None);
        assert_eq!(snapshot.artwork, None);
        assert!(snapshot.is_playing);
        assert_eq!(snapshot.source_id, h.source.id());
    }

    #[test]
    fn publishes_nil_when_no_window_matches() {
        let h = Harness::new();
        h.lister.set(vec![window("Finder", "Documents")]);
        h.start();
        assert_eq!(h.published_count(), 1);
        assert_eq!(h.latest(), None);
    }

    #[test]
    fn ignores_non_matching_owner() {
        let h = Harness::new();
        h.lister.set(vec![window(
            "Safari",
            "YouTube Music - 假日而已 | YouTube Music 🔊",
        )]);
        h.start();
        assert_eq!(h.latest(), None);
    }

    #[test]
    fn ignores_unparsable_title_of_matching_owner() {
        let h = Harness::new();
        h.lister.set(vec![window("YouTube Music", "YouTube Music")]);
        h.start();
        assert_eq!(h.latest(), None);
    }

    #[test]
    fn prefers_playing_window_over_paused_one() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", false), ytm("B", true)]);
        h.start();
        assert_eq!(h.latest().map(|s| s.title), Some("B".to_string()));
    }

    #[test]
    fn snapshot_carries_anchor_only_after_observed_change() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        assert_eq!(h.latest().map(|s| s.title), Some("A".to_string()));
        assert_eq!(h.latest().and_then(|s| s.position), None);
        h.tick(1.0);
        assert_eq!(h.latest().and_then(|s| s.position), None);

        h.lister.set(vec![ytm("B", true)]);
        h.tick(1.0);
        assert_eq!(h.latest().map(|s| s.title), Some("B".to_string()));
        assert_eq!(
            h.latest().and_then(|s| s.position),
            Some(PositionAnchor {
                elapsed: 0.0,
                timestamp: T0 + 2.0,
                rate: 1.0
            })
        );
    }

    #[test]
    fn pause_freezes_anchor_in_snapshot() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.lister.set(vec![ytm("B", true)]);
        h.tick(1.0);
        h.lister.set(vec![ytm("B", false)]);
        h.tick(4.0);
        let latest = h.latest().expect("a snapshot");
        assert!(!latest.is_playing);
        assert_eq!(
            latest.position,
            Some(PositionAnchor {
                elapsed: 4.0,
                timestamp: T0 + 5.0,
                rate: 0.0
            })
        );
    }

    #[test]
    fn polls_every_second_while_playing_and_every_five_when_not() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.tick(1.0);
        h.lister.set(vec![ytm("A", false)]);
        h.tick(1.0);
        h.lister.set(vec![]);
        h.tick(5.0);
        let secs = |d: &Duration| d.as_secs();
        assert_eq!(
            h.delays().iter().map(secs).collect::<Vec<_>>(),
            vec![1, 1, 5, 5]
        );
    }

    #[test]
    fn does_not_republish_unchanged_snapshot() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.tick(1.0);
        h.tick(1.0);
        assert_eq!(
            h.published_count(),
            2,
            "the initial nil, then the one snapshot"
        );
    }

    #[test]
    fn stop_publishes_nil_and_cancels_polling() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.stop();
        assert_eq!(h.latest(), None);
        let count = h.published_count();
        let calls = h.lister.calls();
        h.tick(1.0);
        h.tick(5.0);
        assert_eq!(h.published_count(), count);
        assert_eq!(h.lister.calls(), calls, "the pending poll is cancelled");
        assert_eq!(h.clock.inner.pending_sleeps(), 0);
    }

    #[test]
    fn restart_after_stop_forgets_estimator_state() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.lister.set(vec![ytm("B", true)]);
        h.tick(1.0);
        assert!(h.latest().and_then(|s| s.position).is_some());
        h.stop();
        h.start();
        assert_eq!(h.latest().map(|s| s.title), Some("B".to_string()));
        assert_eq!(h.latest().and_then(|s| s.position), None);
    }

    // ---------------------------------------------------------------- 並行與排程（Rust 版新增）

    #[test]
    fn start_does_not_list_windows_on_the_caller_thread() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.source.start();
        assert_eq!(
            h.lister.calls(),
            0,
            "listing is blocking: it runs on the spawner"
        );
        assert_eq!(h.latest(), None);
        h.spawner.run_until_stalled();
        assert_eq!(h.lister.calls(), 1);
        assert_eq!(h.latest().map(|s| s.title), Some("A".to_string()));
    }

    #[test]
    fn start_when_already_running_does_not_poll_again() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.start();
        assert_eq!(h.lister.calls(), 1);
        h.tick(1.0);
        assert_eq!(
            h.lister.calls(),
            2,
            "one poll per interval, no second chain"
        );
    }

    #[test]
    fn stop_then_start_leaves_a_single_poll_chain() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        h.source.stop();
        h.source.start();
        h.spawner.run_until_stalled();
        let calls = h.lister.calls();
        h.tick(1.0);
        assert_eq!(
            h.lister.calls(),
            calls + 1,
            "the pre-stop timer must not also fire"
        );
    }

    #[test]
    fn stop_while_listing_discards_the_stale_poll_and_does_not_deadlock() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        assert_eq!(h.latest().map(|s| s.title), Some("A".to_string()));

        // 下一次列舉進行中（沒有持鎖）有人呼叫 stop()：這次輪詢的結果已過期，不得蓋掉 stop 發布的 None，
        // 也不得再排下一次輪詢。
        let source = h.source.clone();
        *h.lister.on_call.lock().unwrap() = Some(Box::new(move || source.stop()));
        h.lister.set(vec![ytm("B", true)]);
        h.tick(1.0);

        assert_eq!(h.latest(), None);
        assert_eq!(h.clock.inner.pending_sleeps(), 0);
        let calls = h.lister.calls();
        h.tick(5.0);
        assert_eq!(h.lister.calls(), calls);
    }

    #[test]
    fn dropping_the_source_cancels_the_pending_poll() {
        let Harness {
            lister,
            clock,
            spawner,
            source,
            ..
        } = Harness::new();
        lister.set(vec![ytm("A", true)]);
        source.start();
        spawner.run_until_stalled();
        assert_eq!(clock.inner.pending_sleeps(), 1);
        drop(source);
        spawner.run_until_stalled();
        assert_eq!(clock.inner.pending_sleeps(), 0);
        let calls = lister.calls();
        clock.inner.advance(Duration::from_secs(10));
        spawner.run_until_stalled();
        assert_eq!(lister.calls(), calls);
    }

    #[test]
    fn subscribe_replays_the_current_snapshot() {
        let h = Harness::new();
        h.lister.set(vec![ytm("A", true)]);
        h.start();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let _sub = h
            .source
            .subscribe(Box::new(move |v| s.lock().unwrap().push(v.clone())));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].as_ref().map(|s| s.title.as_str()), Some("A"));
    }

    #[test]
    fn poll_intervals_match_swift() {
        assert_eq!(PLAYING_POLL_INTERVAL, Duration::from_secs(1));
        assert_eq!(IDLE_POLL_INTERVAL, Duration::from_secs(5));
    }

    /// 手動檢查（`cargo test -p musicat-platform-macos -- --ignored real_lister`）：真的呼叫系統 API，
    /// 只確認不當機並印出視窗數量；視窗標題與擁有者是私人資料，絕不印出。
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "calls the real CoreGraphics API; needs a window server"]
    fn real_lister_does_not_crash() {
        let windows = CgWindowLister.windows();
        assert!(windows.iter().all(|w| !w.title.is_empty()));
        println!("CgWindowLister returned {} windows", windows.len());
    }
}
