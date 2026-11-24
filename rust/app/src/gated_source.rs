//! 可在執行時開關、第一次需要時才建立的現正播放來源（給 macOS 的 perl 宿主開關用）。
//!
//! 問題：`NowPlayingMonitor` 與 `MusicModuleState` 的來源清單在建立時就固定了，但 perl 宿主可以從選單即時開關，
//! 而且啟動時 `perl_host = false` 就**不應該建立**監管者。[`GatedSource`] 是一個固定放在清單裡的代理：
//!
//! - 內層來源（`T`）由工廠在「第一次同時滿足『被要求執行』與『開關打開』」時才建立，之後重複使用。
//! - `start()`／`stop()` 記下模組的要求（`wanted`）；開關打開才轉給內層。
//! - [`GatedSource::set_enabled`]：打開且已被要求執行 → 建立（若還沒有）並 `start()`；
//!   關閉 → 內層 `stop()`，並發布 `None`。
//! - 內層發布的值只在開關打開時轉發；關閉期間內層殘留的輸出一律丟棄。
//! - 訂閱語意與一般來源相同（[`PublishedSnapshot`]：第一次呼叫是目前值的重播，之後依序、不倒退）。
//!
//! 並行：狀態在單一 `Mutex` 裡；呼叫工廠、內層的 `start`／`stop`／`subscribe` 與送出通知都在鎖外。
//! 內層的回呼可能在任意執行緒上呼叫，它只在鎖內檢查開關並排入發布、放鎖後送出。

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use musicat_core::nowplaying::{NowPlayingSource, PublishedSnapshot, SnapshotCallback};
use musicat_core::runtime::Subscription;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Factory<T> = Box<dyn Fn() -> Arc<T> + Send + Sync>;

struct Built<T> {
    source: Arc<T>,
    _subscription: Subscription,
}

struct State<T> {
    enabled: bool,
    /// 模組要求執行（`start()` 之後、`stop()` 之前）。
    wanted: bool,
    built: Option<Built<T>>,
}

/// 見模組文件。
pub struct GatedSource<T: NowPlayingSource + 'static> {
    id: String,
    factory: Factory<T>,
    state: Mutex<State<T>>,
    published: PublishedSnapshot,
    me: Weak<Self>,
}

/// 鎖外要對內層做的事。
enum Action<T> {
    None,
    Start(Arc<T>),
    Stop(Arc<T>),
    BuildAndStart,
}

impl<T: NowPlayingSource + 'static> GatedSource<T> {
    /// `id` 是這個來源對外的 id（通常就是內層來源的 id）；`enabled` 是開關的初始值。
    /// 建立時**不會**呼叫工廠。
    pub fn new(
        id: impl Into<String>,
        enabled: bool,
        factory: impl Fn() -> Arc<T> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            id: id.into(),
            factory: Box::new(factory),
            state: Mutex::new(State {
                enabled,
                wanted: false,
                built: None,
            }),
            published: PublishedSnapshot::new(),
            me: me.clone(),
        })
    }

    /// 已建立的內層來源（例如要呼叫 `HelperSupervisor::wake_from_sleep`）；還沒建立時為 `None`。
    pub fn inner(&self) -> Option<Arc<T>> {
        lock(&self.state).built.as_ref().map(|b| b.source.clone())
    }

    pub fn is_enabled(&self) -> bool {
        lock(&self.state).enabled
    }

    /// 打開或關閉（規則見模組文件）。值沒變時什麼都不做。
    pub fn set_enabled(&self, enabled: bool) {
        let action = {
            let mut st = lock(&self.state);
            if st.enabled == enabled {
                return;
            }
            st.enabled = enabled;
            if enabled {
                Self::start_action(&st)
            } else {
                // 關閉：先發布 None（在鎖內排入，之後內層殘留的輸出因開關已關而被丟棄）。
                self.published.queue_set(None);
                st.built
                    .as_ref()
                    .map_or(Action::None, |b| Action::Stop(b.source.clone()))
            }
        };
        self.published.flush();
        self.run(action);
    }

    fn start_action(st: &State<T>) -> Action<T> {
        if !(st.enabled && st.wanted) {
            return Action::None;
        }
        match &st.built {
            Some(b) => Action::Start(b.source.clone()),
            None => Action::BuildAndStart,
        }
    }

    fn run(&self, action: Action<T>) {
        match action {
            Action::None => {}
            Action::Start(source) => source.start(),
            Action::Stop(source) => source.stop(),
            Action::BuildAndStart => {
                let source = (self.factory)();
                let weak = self.me.clone();
                let subscription = source.subscribe(Box::new(move |snapshot| {
                    if let Some(gate) = weak.upgrade() {
                        gate.forward(snapshot.clone());
                    }
                }));
                let start = {
                    let mut st = lock(&self.state);
                    if st.built.is_none() {
                        st.built = Some(Built {
                            source: source.clone(),
                            _subscription: subscription,
                        });
                    }
                    // 建立期間狀態可能又變了：照目前狀態決定要不要啟動（已被別的呼叫建好時用那一個）。
                    match Self::start_action(&st) {
                        Action::Start(s) | Action::Stop(s) => Some(s),
                        Action::BuildAndStart | Action::None => None,
                    }
                };
                if let Some(source) = start {
                    source.start();
                }
            }
        }
    }

    fn forward(&self, snapshot: Option<musicat_core::types::TrackSnapshot>) {
        {
            let st = lock(&self.state);
            if !st.enabled {
                return;
            }
            self.published.queue_set(snapshot);
        }
        self.published.flush();
    }
}

impl<T: NowPlayingSource + 'static> NowPlayingSource for GatedSource<T> {
    fn id(&self) -> &str {
        &self.id
    }

    fn start(&self) {
        let action = {
            let mut st = lock(&self.state);
            st.wanted = true;
            Self::start_action(&st)
        };
        self.run(action);
    }

    fn stop(&self) {
        let action = {
            let mut st = lock(&self.state);
            st.wanted = false;
            self.published.queue_set(None);
            st.built
                .as_ref()
                .map_or(Action::None, |b| Action::Stop(b.source.clone()))
        };
        self.published.flush();
        self.run(action);
    }

    fn subscribe(&self, f: SnapshotCallback) -> Subscription {
        self.published.subscribe(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use musicat_core::types::TrackSnapshot;

    #[derive(Default)]
    struct Fake {
        starts: AtomicUsize,
        stops: AtomicUsize,
        published: PublishedSnapshot,
    }

    impl Fake {
        fn emit(&self, title: &str) {
            self.published.queue_set(Some(snap(title)));
            self.published.flush();
        }
    }

    impl NowPlayingSource for Fake {
        fn id(&self) -> &str {
            "fake"
        }
        fn start(&self) {
            self.starts.fetch_add(1, Ordering::SeqCst);
        }
        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.published.queue_set(None);
            self.published.flush();
        }
        fn subscribe(&self, f: SnapshotCallback) -> Subscription {
            self.published.subscribe(f)
        }
    }

    fn snap(title: &str) -> TrackSnapshot {
        TrackSnapshot {
            title: title.into(),
            artist: None,
            album: None,
            duration: None,
            is_playing: true,
            position: None,
            artwork: None,
            source_id: "fake".into(),
        }
    }

    /// 回傳 gate、工廠被呼叫的次數、最近一次建出的 Fake。
    type LastBuilt = Arc<Mutex<Option<Arc<Fake>>>>;

    fn gate(enabled: bool) -> (Arc<GatedSource<Fake>>, Arc<AtomicUsize>, LastBuilt) {
        let builds = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let (b, l) = (builds.clone(), last.clone());
        let g = GatedSource::new("helper", enabled, move || {
            b.fetch_add(1, Ordering::SeqCst);
            let fake = Arc::new(Fake::default());
            *l.lock().unwrap() = Some(fake.clone());
            fake
        });
        (g, builds, last)
    }

    fn record(g: &GatedSource<Fake>) -> (Arc<Mutex<Vec<Option<String>>>>, Subscription) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let sub = g.subscribe(Box::new(move |v| {
            s.lock().unwrap().push(v.as_ref().map(|t| t.title.clone()));
        }));
        (seen, sub)
    }

    #[test]
    fn disabled_gate_never_builds_even_when_started() {
        let (g, builds, _) = gate(false);
        g.start();
        g.stop();
        g.start();
        assert_eq!(builds.load(Ordering::SeqCst), 0);
        assert!(g.inner().is_none());
        assert_eq!(g.id(), "helper");
    }

    #[test]
    fn construction_does_not_build_and_enabled_gate_builds_on_first_start() {
        let (g, builds, last) = gate(true);
        assert_eq!(builds.load(Ordering::SeqCst), 0, "lazy");
        g.start();
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        let fake = last.lock().unwrap().clone().unwrap();
        assert_eq!(fake.starts.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&g.inner().unwrap(), &fake));
    }

    #[test]
    fn enabling_after_start_builds_and_starts_once_and_reuses_the_inner_source() {
        let (g, builds, last) = gate(false);
        g.start();
        g.set_enabled(true);
        let fake = last.lock().unwrap().clone().unwrap();
        assert_eq!(fake.starts.load(Ordering::SeqCst), 1);

        g.set_enabled(false);
        assert_eq!(fake.stops.load(Ordering::SeqCst), 1);
        g.set_enabled(true);
        assert_eq!(builds.load(Ordering::SeqCst), 1, "built only once");
        assert_eq!(fake.starts.load(Ordering::SeqCst), 2);
        g.set_enabled(true); // 沒變：什麼都不做
        assert_eq!(fake.starts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn enabling_before_anyone_asked_to_run_does_not_build() {
        let (g, builds, _) = gate(false);
        g.set_enabled(true);
        assert_eq!(builds.load(Ordering::SeqCst), 0);
        assert!(g.is_enabled());
    }

    #[test]
    fn values_are_forwarded_only_while_enabled_and_disabling_publishes_none() {
        let (g, _, last) = gate(true);
        let (seen, _sub) = record(&g);
        g.start();
        let fake = last.lock().unwrap().clone().unwrap();
        fake.emit("A");
        g.set_enabled(false);
        fake.emit("late output while disabled");
        g.set_enabled(true);
        fake.emit("B");
        assert_eq!(
            *seen.lock().unwrap(),
            [None, Some("A".to_string()), None, Some("B".to_string())]
        );
    }

    #[test]
    fn stop_stops_the_inner_source_and_publishes_none() {
        let (g, _, last) = gate(true);
        let (seen, _sub) = record(&g);
        g.start();
        let fake = last.lock().unwrap().clone().unwrap();
        fake.emit("A");
        g.stop();
        assert_eq!(fake.stops.load(Ordering::SeqCst), 1);
        assert_eq!(seen.lock().unwrap().last().unwrap(), &None);
        // stop 之後打開開關不會啟動（沒有人要求執行）。
        g.set_enabled(false);
        g.set_enabled(true);
        assert_eq!(fake.starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn subscribe_replays_the_current_value() {
        let (g, _, last) = gate(true);
        g.start();
        last.lock().unwrap().clone().unwrap().emit("now");
        let (seen, _sub) = record(&g);
        assert_eq!(*seen.lock().unwrap(), [Some("now".to_string())]);
    }

    #[test]
    fn dropping_the_gate_releases_the_inner_source() {
        let (g, _, last) = gate(true);
        g.start();
        let weak = Arc::downgrade(&last.lock().unwrap().take().unwrap());
        drop(g);
        assert!(
            weak.upgrade().is_none(),
            "no reference cycle through the subscription"
        );
    }
}
