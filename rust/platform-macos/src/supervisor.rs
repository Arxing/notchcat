//! helper 行程監管（移植自 Swift `MediaRemoteSource.swift`）：以注入的 [`HelperLauncher`] 啟動讀取
//! 現正播放的 helper 行程，把它的輸出行以 [`parse_line`] 解碼後發布，並在行程意外結束時依退避重啟。
//!
//! 行為（逐項照 Swift 的文件註解）：
//! - 意外結束後，等 `retry_delays` 的下一項再重啟；收到任何一行就把這個序列歸零。
//! - 延遲用完後發布 `None` 並停在那裡；`give_up_retry_interval` 後以歸零的計數重試一次，
//!   `start()` 或系統喚醒（[`HelperSupervisor::wake_from_sleep`]）也會立刻重試。`launch` 回 `Err` 也算一次失敗。
//! - 喚醒時，正在跑的 helper 可能已經失效：終止它、換一個新的（計數歸零）。
//! - `stop()` 之後重試與喚醒都不做任何事；`stop()` 終止 helper、取消排定的計時器並發布 `None`，
//!   所以之後再 `start()` 不會重播這一輪的最後快照。
//! - 每次啟動與 `stop()` 都把 generation 加一，屬於舊行程的回呼與計時器一律忽略；
//!   行程結束後才到的殘留輸出不得發布，也不得把失敗計數歸零（否則放棄上限形同虛設）。
//!
//! 並行：Swift 版要求所有呼叫都在同一個佇列上；這裡 launcher 的 sink 可能在任意執行緒（讀取執行緒）上被呼叫，
//! 計時器在注入的執行器上跑。所有狀態放在單一 `Mutex<State>`，事件入口只有
//! `Line`／`Exited`／`Timer`／`Wake`／`Start`／`Stop`：每個入口持鎖更新狀態並記下要做的副作用
//! （啟動、終止、排程計時器、釋放把手），放鎖後才執行。呼叫 launcher、終止或釋放把手、
//! 呼叫 `Spawner::spawn`、執行訂閱者回呼時都不持鎖，所以同步的 launcher 在 `launch()` 內就送出事件也不會死結。
//!
//! 發布採與 `musicat_core::nowplaying::monitor` 相同的序列化佇列：發布在鎖內依狀態變化順序排入，
//! 由同一時間只有一條執行緒的送出者在鎖外依序送給訂閱者；訂閱時的重播也是佇列中的一個事件，
//! 所以訂閱者不會在新值之後收到舊值（例如讀取執行緒的曲目與 `stop()` 的 `None` 交錯時）。
//!
//! 純邏輯、沒有 OS API，所以在所有目標上都編譯並跑測試。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use musicat_core::nowplaying::{NowPlayingSource, SnapshotCallback};
use musicat_core::runtime::{Clock, Spawner, Subscription, TaskHandle};
use musicat_core::types::TrackSnapshot;

use crate::helper_wire::{parse_line, HelperMessage, SOURCE_ID};

/// 一個執行中的 helper 行程。
pub trait HelperHandle: Send {
    fn terminate(&mut self);
}

/// launcher 回報行程事件的出口；可能在任意執行緒上被呼叫，也可能在 `launch()` 返回前就被呼叫。
pub type HelperSink = Arc<dyn Fn(HelperEvent) + Send + Sync>;

/// 啟動現正播放 helper 行程。每一行完整輸出以 [`HelperEvent::Line`]、結束狀態以 [`HelperEvent::Exited`]
/// 送進 `sink`。行程根本無法啟動時回 `Err`。
pub trait HelperLauncher: Send + Sync {
    fn launch(&self, sink: HelperSink) -> Result<Box<dyn HelperHandle>, LaunchError>;
}

/// helper 行程的事件。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelperEvent {
    /// 一行完整輸出（不含換行）。
    Line(String),
    /// 行程結束，附結束狀態。
    Exited(i32),
}

/// 無法啟動 helper 行程的原因（Swift `PerlHelperLauncher.LaunchError`，加上一般的啟動失敗）。
/// 定義在這裡供 `perl_launcher`（Task 16）共用。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LaunchError {
    PerlNotFound,
    DylibNotFound,
    /// 啟動行程本身失敗（附 OS 的錯誤訊息）。
    Spawn(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::PerlNotFound => f.write_str("perl not found"),
            LaunchError::DylibNotFound => f.write_str("helper dylib not found"),
            LaunchError::Spawn(msg) => write!(f, "failed to spawn helper: {msg}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// 重啟策略。
#[derive(Clone, PartialEq, Debug)]
pub struct SupervisorConfig {
    /// 連續失敗時依序使用的重啟延遲；用完即放棄。
    pub retry_delays: Vec<Duration>,
    /// 放棄後多久以歸零的計數再試一次。
    pub give_up_retry_interval: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            retry_delays: [1, 2, 4, 8, 16, 30].map(Duration::from_secs).to_vec(),
            give_up_retry_interval: Duration::from_secs(300),
        }
    }
}

/// 忽略 mutex 中毒：持鎖時不執行任何使用者程式碼，中毒只可能來自內部 panic，狀態仍一致。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TimerKind {
    /// 依退避重啟；只在仍在跑且 generation 未變時動作。
    Retry,
    /// 放棄後的重試；只在仍是放棄狀態且 generation 未變時動作。
    GiveUp,
}

/// 放鎖後才執行的副作用。
enum Effect {
    Launch {
        generation: u64,
    },
    Terminate(Box<dyn HelperHandle>),
    /// 只釋放、不終止（行程已自行結束）。
    Release(Box<dyn HelperHandle>),
    Schedule {
        id: u64,
        generation: u64,
        delay: Duration,
        kind: TimerKind,
    },
    /// 取消（drop）計時器工作。
    Cancel(TaskHandle),
}

/// 訂閱回呼（[`SnapshotCallback`] 的共享版）。
type SharedCallback = Arc<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>;

struct Sub {
    id: u64,
    /// 登記當下已發出的最後一個廣播序號；只接收序號更大的廣播。
    since_seq: u64,
    cancelled: AtomicBool,
    cb: SharedCallback,
}

enum Publication {
    Broadcast {
        seq: u64,
        value: Option<TrackSnapshot>,
    },
    Replay {
        target: Arc<Sub>,
        value: Option<TrackSnapshot>,
    },
}

struct State {
    is_running: bool,
    /// 從延遲用完到下一次啟動或 `stop()` 之間為真。
    has_given_up: bool,
    failure_count: usize,
    /// 每次啟動與 `stop()` 都加一；舊行程的回呼與計時器以此判斷是否過期。
    generation: u64,
    handle: Option<Box<dyn HelperHandle>>,
    /// 目前有效的計時器 id；被取消或已觸發時為 `None`。
    timer_id: Option<u64>,
    /// 目前有效計時器的工作把手（drop 即取消）。
    timer_task: Option<TaskHandle>,
    next_timer_id: u64,

    // ---- 發布
    current: Option<TrackSnapshot>,
    seq: u64,
    pending: VecDeque<Publication>,
    next_sub_id: u64,
    subscribers: Vec<Arc<Sub>>,
    draining: bool,
}

impl State {
    fn publish(&mut self, value: Option<TrackSnapshot>) {
        self.current = value.clone();
        self.seq += 1;
        let seq = self.seq;
        self.pending
            .push_back(Publication::Broadcast { seq, value });
    }

    fn cancel_timer(&mut self, effects: &mut Vec<Effect>) {
        self.timer_id = None;
        if let Some(task) = self.timer_task.take() {
            effects.push(Effect::Cancel(task));
        }
    }

    fn schedule(&mut self, delay: Duration, kind: TimerKind, effects: &mut Vec<Effect>) {
        self.cancel_timer(effects);
        let id = self.next_timer_id;
        self.next_timer_id += 1;
        self.timer_id = Some(id);
        effects.push(Effect::Schedule {
            id,
            generation: self.generation,
            delay,
            kind,
        });
    }

    /// Swift `launch()` 的鎖內部分：換新的 generation，啟動本身在放鎖後執行。
    fn begin_launch(&mut self, effects: &mut Vec<Effect>) {
        self.generation += 1;
        self.has_given_up = false;
        self.cancel_timer(effects);
        effects.push(Effect::Launch {
            generation: self.generation,
        });
    }

    fn fail(&mut self, cfg: &SupervisorConfig, effects: &mut Vec<Effect>) {
        if self.failure_count >= cfg.retry_delays.len() {
            self.is_running = false;
            self.has_given_up = true;
            self.publish(None);
            self.schedule(cfg.give_up_retry_interval, TimerKind::GiveUp, effects);
            return;
        }
        let delay = cfg.retry_delays[self.failure_count];
        self.failure_count += 1;
        self.schedule(delay, TimerKind::Retry, effects);
    }
}

struct Inner {
    launcher: Arc<dyn HelperLauncher>,
    clock: Arc<dyn Clock>,
    spawner: Arc<dyn Spawner>,
    cfg: SupervisorConfig,
    state: Mutex<State>,
}

/// 送出流程若因訂閱者 panic 而中途離開，要把 `draining` 放掉，否則之後再也沒有人會送。
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
    /// 在鎖內跑 `f`，放鎖後執行它記下的副作用，最後送出排入的發布。
    fn transition(
        self: &Arc<Self>,
        f: impl FnOnce(&mut State, &SupervisorConfig, &mut Vec<Effect>),
    ) {
        let mut effects = Vec::new();
        {
            let mut st = lock(&self.state);
            f(&mut st, &self.cfg, &mut effects);
        }
        self.run(effects);
        self.drain();
    }

    fn run(self: &Arc<Self>, effects: Vec<Effect>) {
        let mut queue: VecDeque<Effect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                Effect::Launch { generation } => self.launch(generation, &mut queue),
                Effect::Terminate(mut handle) => handle.terminate(),
                Effect::Release(handle) => drop(handle),
                Effect::Cancel(task) => drop(task),
                Effect::Schedule {
                    id,
                    generation,
                    delay,
                    kind,
                } => self.schedule(id, generation, delay, kind),
            }
        }
    }

    fn launch(self: &Arc<Self>, generation: u64, queue: &mut VecDeque<Effect>) {
        let weak: Weak<Inner> = Arc::downgrade(self);
        let sink: HelperSink = Arc::new(move |event| {
            if let Some(inner) = weak.upgrade() {
                match event {
                    HelperEvent::Line(line) => inner.on_line(generation, &line),
                    HelperEvent::Exited(_) => inner.on_exited(generation),
                }
            }
        });
        let result = self.launcher.launch(sink);
        let mut effects = Vec::new();
        {
            let mut st = lock(&self.state);
            let current = st.is_running && st.generation == generation;
            match result {
                // 啟動期間若已 stop、被喚醒換掉、或行程已在 launch() 內結束，這個把手就不再屬於現況。
                Ok(handle) if current => st.handle = Some(handle),
                Ok(handle) => effects.push(Effect::Terminate(handle)),
                Err(_) if current => st.fail(&self.cfg, &mut effects),
                Err(_) => {}
            }
        }
        queue.extend(effects);
    }

    fn schedule(self: &Arc<Self>, id: u64, generation: u64, delay: Duration, kind: TimerKind) {
        let weak: Weak<Inner> = Arc::downgrade(self);
        let sleep = self.clock.sleep(delay);
        let task = self.spawner.spawn(Box::pin(async move {
            sleep.await;
            if let Some(inner) = weak.upgrade() {
                inner.on_timer(id, generation, kind);
            }
        }));
        let stale = {
            let mut st = lock(&self.state);
            if st.timer_id == Some(id) {
                st.timer_task = Some(task);
                None
            } else {
                // 排程期間已被取消、被新的計時器取代，或已經觸發。
                Some(task)
            }
        };
        drop(stale);
    }

    // ---- 事件入口

    fn on_line(self: &Arc<Self>, generation: u64, line: &str) {
        // 解碼可能很重（base64 封面），在鎖外先做。
        let message = parse_line(line);
        self.transition(|st, _, _| {
            if !st.is_running || st.generation != generation {
                return;
            }
            st.failure_count = 0;
            match message {
                Some(HelperMessage::Track(snapshot)) => st.publish(Some(snapshot)),
                Some(HelperMessage::None) => st.publish(None),
                None => {}
            }
        });
    }

    fn on_exited(self: &Arc<Self>, generation: u64) {
        self.transition(|st, cfg, effects| {
            if !st.is_running || st.generation != generation {
                return;
            }
            // 已死行程仍在緩衝中的輸出不得發布、也不得把失敗計數歸零。
            st.generation += 1;
            if let Some(handle) = st.handle.take() {
                effects.push(Effect::Release(handle));
            }
            st.fail(cfg, effects);
        });
    }

    fn on_timer(self: &Arc<Self>, id: u64, generation: u64, kind: TimerKind) {
        self.transition(|st, _, effects| {
            if st.timer_id != Some(id) {
                return;
            }
            st.timer_id = None;
            // 這個工作正在跑：把手 detach 而不是 drop，不在工作自己裡面取消自己。
            if let Some(task) = st.timer_task.take() {
                task.detach();
            }
            if st.generation != generation {
                return;
            }
            match kind {
                TimerKind::Retry if st.is_running => st.begin_launch(effects),
                TimerKind::GiveUp if st.has_given_up => {
                    st.has_given_up = false;
                    st.is_running = true;
                    st.failure_count = 0;
                    st.begin_launch(effects);
                }
                _ => {}
            }
        });
    }

    fn on_start(self: &Arc<Self>) {
        self.transition(|st, _, effects| {
            if st.is_running {
                return;
            }
            st.is_running = true;
            st.failure_count = 0;
            st.begin_launch(effects);
        });
    }

    fn on_stop(self: &Arc<Self>) {
        self.transition(|st, _, effects| {
            st.is_running = false;
            st.has_given_up = false;
            st.generation += 1;
            if let Some(handle) = st.handle.take() {
                effects.push(Effect::Terminate(handle));
            }
            st.cancel_timer(effects);
            st.publish(None);
        });
    }

    fn on_wake(self: &Arc<Self>) {
        self.transition(|st, _, effects| {
            if !st.is_running && !st.has_given_up {
                return;
            }
            if let Some(handle) = st.handle.take() {
                effects.push(Effect::Terminate(handle));
            }
            st.is_running = true;
            st.has_given_up = false;
            st.failure_count = 0;
            st.begin_launch(effects);
        });
    }

    // ---- 發布

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
                    Some(Publication::Broadcast { seq, value }) => {
                        let targets: Vec<Arc<Sub>> = st
                            .subscribers
                            .iter()
                            .filter(|s| s.since_seq < seq)
                            .cloned()
                            .collect();
                        (targets, value)
                    }
                    Some(Publication::Replay { target, value }) => (vec![target], value),
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
            drop(sub);
        }
    }
}

/// 系統現正播放的來源：由 helper 行程讀取（見 `perl_launcher`），以 [`parse_line`] 解碼。
/// 行為見模組文件。
pub struct HelperSupervisor {
    inner: Arc<Inner>,
}

impl HelperSupervisor {
    pub fn new(
        launcher: Arc<dyn HelperLauncher>,
        clock: Arc<dyn Clock>,
        spawner: Arc<dyn Spawner>,
        cfg: SupervisorConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                launcher,
                clock,
                spawner,
                cfg,
                state: Mutex::new(State {
                    is_running: false,
                    has_given_up: false,
                    failure_count: 0,
                    generation: 0,
                    handle: None,
                    timer_id: None,
                    timer_task: None,
                    next_timer_id: 0,
                    current: None,
                    seq: 0,
                    pending: VecDeque::new(),
                    next_sub_id: 0,
                    subscribers: Vec::new(),
                    draining: false,
                }),
            }),
        })
    }

    /// 系統從睡眠喚醒時由平台層呼叫（Swift 版自己訂閱 `NSWorkspace.didWakeNotification`）。
    /// helper 可能已失效：替換正在跑的那個，已放棄時重新開始；已停止（或從未開始）時什麼都不做。
    pub fn wake_from_sleep(&self) {
        self.inner.on_wake();
    }
}

impl NowPlayingSource for HelperSupervisor {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn start(&self) {
        self.inner.on_start();
    }

    /// 終止 helper、取消排定的重試並發布 `None`，所以之後再 `start()` 不會重播這一輪的最後快照。
    fn stop(&self) {
        self.inner.on_stop();
    }

    /// 第一次呼叫一定是登記當下目前值的重播，之後依發布順序呼叫（不在新值之後收到舊值）。
    fn subscribe(&self, f: SnapshotCallback) -> Subscription {
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
            st.pending
                .push_back(Publication::Replay { target: sub, value });
            id
        };
        self.inner.drain();
        let weak = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.unsubscribe(id);
            }
        })
    }
}

impl Drop for HelperSupervisor {
    /// 不留下孤兒行程：終止仍在跑的 helper（不發布，訂閱者隨監管者一起消失）。
    fn drop(&mut self) {
        let (handle, timer) = {
            let mut st = lock(&self.inner.state);
            st.is_running = false;
            st.has_given_up = false;
            st.generation += 1;
            st.timer_id = None;
            (st.handle.take(), st.timer_task.take())
        };
        drop(timer);
        if let Some(mut handle) = handle {
            handle.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    // MediaRemoteSourceTests 的 supervisor 案例（逐案移植），加上計畫指定的額外案例。

    use super::*;
    use crate::helper_wire::{parse_line, HelperMessage};
    use musicat_core::runtime::BoxFuture;
    use musicat_core::testing::{FakeClock, LocalSpawner};
    use musicat_core::types::TrackSnapshot;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TRACK_LINE: &str = r#"{"title":"聲聲慢","artist":"鄧福如AFÜ","duration":233.9,"elapsed":12.5,"timestamp":1793777309.5,"rate":1}"#;

    // ---------------------------------------------------------------- 測試替身

    type Sink = Arc<dyn Fn(HelperEvent) + Send + Sync>;

    struct FakeHandle {
        terminate_count: Arc<AtomicUsize>,
    }

    impl HelperHandle for FakeHandle {
        fn terminate(&mut self) {
            self.terminate_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct FakeLauncherState {
        /// 設定時每次 launch 都回這個錯誤（對應 Swift `shouldThrow`）。
        error: Option<LaunchError>,
        launch_count: usize,
        /// 每個成功啟動的行程的 terminate 次數。
        handles: Vec<Arc<AtomicUsize>>,
        /// 每個成功啟動的行程的 sink（依啟動順序）；`emit`／`exit` 送給最後一個。
        sinks: Vec<Sink>,
    }

    /// 對應 Swift 的 `FakeLauncher`：可設定 launch 失敗、記錄 terminate，並讓測試餵行與結束事件。
    /// 保留每一代的 sink，所以也能從已死的舊行程送出殘留輸出。
    #[derive(Default)]
    struct FakeLauncher {
        state: Mutex<FakeLauncherState>,
    }

    impl FakeLauncher {
        fn set_error(&self, error: Option<LaunchError>) {
            self.state.lock().unwrap().error = error;
        }

        fn launch_count(&self) -> usize {
            self.state.lock().unwrap().launch_count
        }

        fn terminate_count(&self, index: usize) -> usize {
            self.state.lock().unwrap().handles[index].load(Ordering::SeqCst)
        }

        fn sink(&self, index: Option<usize>) -> Option<Sink> {
            let st = self.state.lock().unwrap();
            match index {
                Some(i) => st.sinks.get(i).cloned(),
                None => st.sinks.last().cloned(),
            }
        }

        /// 由最新的行程送出一行（鎖外呼叫 sink）。
        fn emit(&self, line: &str) {
            if let Some(sink) = self.sink(None) {
                sink(HelperEvent::Line(line.to_string()));
            }
        }

        /// 由第 `index` 個（從 0 起算）行程送出一行。
        fn emit_from(&self, index: usize, line: &str) {
            if let Some(sink) = self.sink(Some(index)) {
                sink(HelperEvent::Line(line.to_string()));
            }
        }

        fn exit(&self) {
            self.exit_with(1);
        }

        fn exit_with(&self, status: i32) {
            if let Some(sink) = self.sink(None) {
                sink(HelperEvent::Exited(status));
            }
        }
    }

    impl HelperLauncher for FakeLauncher {
        fn launch(&self, sink: Sink) -> Result<Box<dyn HelperHandle>, LaunchError> {
            let mut st = self.state.lock().unwrap();
            st.launch_count += 1;
            if let Some(error) = st.error.clone() {
                return Err(error);
            }
            let count = Arc::new(AtomicUsize::new(0));
            st.handles.push(count.clone());
            st.sinks.push(sink);
            Ok(Box::new(FakeHandle {
                terminate_count: count,
            }))
        }
    }

    /// 包住 `FakeClock`，記錄每次排程的延遲（對應 Swift `FakeScheduler.delays`）。
    struct RecordingClock {
        inner: FakeClock,
        delays: Mutex<Vec<f64>>,
    }

    impl Clock for RecordingClock {
        fn now(&self) -> f64 {
            self.inner.now()
        }

        fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
            self.delays.lock().unwrap().push(d.as_secs_f64());
            self.inner.sleep(d)
        }
    }

    struct Harness {
        launcher: Arc<FakeLauncher>,
        clock: Arc<RecordingClock>,
        spawner: LocalSpawner,
        /// 所有發布（不含訂閱當下的重播，對應 Swift 的 `dropFirst()`）。
        published: Arc<Mutex<Vec<Option<TrackSnapshot>>>>,
        _subscription: Subscription,
    }

    impl Harness {
        fn new() -> Self {
            Self::with_launcher(Arc::new(FakeLauncher::default()))
        }

        fn with_launcher(launcher: Arc<FakeLauncher>) -> Self {
            Self {
                launcher,
                clock: Arc::new(RecordingClock {
                    inner: FakeClock::new(0.0),
                    delays: Mutex::new(Vec::new()),
                }),
                spawner: LocalSpawner::new(),
                published: Arc::new(Mutex::new(Vec::new())),
                _subscription: Subscription::new(|| {}),
            }
        }

        fn make_source(&mut self) -> Arc<HelperSupervisor> {
            self.make_source_with(&[1.0, 2.0, 4.0, 8.0, 16.0, 30.0], 300.0)
        }

        fn make_source_with(
            &mut self,
            retry_delays: &[f64],
            give_up_retry_interval: f64,
        ) -> Arc<HelperSupervisor> {
            let source = HelperSupervisor::new(
                self.launcher.clone(),
                self.clock.clone(),
                Arc::new(self.spawner.clone()),
                SupervisorConfig {
                    retry_delays: retry_delays
                        .iter()
                        .map(|s| Duration::from_secs_f64(*s))
                        .collect(),
                    give_up_retry_interval: Duration::from_secs_f64(give_up_retry_interval),
                },
            );
            let published = self.published.clone();
            let first = Mutex::new(true);
            self._subscription = source.subscribe(Box::new(move |snapshot| {
                let mut first = first.lock().unwrap();
                if *first {
                    *first = false;
                    return;
                }
                published.lock().unwrap().push(snapshot.clone());
            }));
            source
        }

        /// 對應 Swift `runPending()`：執行目前已排程的所有工作；執行途中新排程的不跑。
        fn run_pending(&self) {
            self.spawner.run_until_stalled();
            self.clock.inner.advance(Duration::from_secs(1_000_000));
            self.spawner.run_until_stalled();
        }

        fn delays(&self) -> Vec<f64> {
            self.clock.delays.lock().unwrap().clone()
        }

        fn published(&self) -> Vec<Option<TrackSnapshot>> {
            self.published.lock().unwrap().clone()
        }
    }

    /// 把 Swift 的 `makeGivenUpSource()`：`retryDelays: [1, 2]` 驅動到放棄狀態。
    fn make_given_up_source(h: &mut Harness) -> Arc<HelperSupervisor> {
        let source = h.make_source_with(&[1.0, 2.0], 300.0);
        source.start();
        h.launcher.exit();
        h.run_pending();
        h.launcher.exit();
        h.run_pending();
        h.launcher.exit();
        source
    }

    // ---------------------------------------------------------------- 逐案移植

    #[test]
    fn publishes_track_from_line() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.emit(TRACK_LINE);

        let published = h.published();
        assert_eq!(published.len(), 1);
        let snapshot = published[0].clone().expect("a track");
        let expected = match parse_line(TRACK_LINE) {
            Some(HelperMessage::Track(t)) => t,
            other => panic!("fixture must parse: {other:?}"),
        };
        assert_eq!(snapshot, expected);
        assert_eq!(snapshot.title, "聲聲慢");
        assert_eq!(source.id(), "mediaremote");
    }

    #[test]
    fn none_line_publishes_nil() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.emit(TRACK_LINE);
        h.launcher.emit(r#"{"none":true}"#);

        let published = h.published();
        assert_eq!(published.len(), 2);
        assert!(published[0].is_some());
        assert!(published[1].is_none());
    }

    #[test]
    fn garbage_line_is_ignored() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.emit("keys=14");
        h.launcher.emit(r#"{"title":""}"#);

        assert!(h.published().is_empty());
    }

    #[test]
    fn restarts_after_unexpected_exit_with_backoff() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        for _ in 0..3 {
            h.launcher.exit();
            h.run_pending();
        }

        assert_eq!(h.delays(), vec![1.0, 2.0, 4.0]);
        assert_eq!(h.launcher.launch_count(), 4);
    }

    #[test]
    fn successful_line_resets_backoff() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.exit();
        h.run_pending();
        h.launcher.emit(TRACK_LINE);
        h.launcher.exit();

        assert_eq!(h.delays(), vec![1.0, 1.0]);
    }

    #[test]
    fn gives_up_after_exhausting_delays() {
        let mut h = Harness::new();
        let source = h.make_source_with(&[1.0, 2.0], 300.0);
        source.start();
        h.launcher.emit(TRACK_LINE);
        h.launcher.exit();
        h.run_pending();
        h.launcher.exit();
        h.run_pending();
        h.launcher.exit();

        assert_eq!(
            h.delays(),
            vec![1.0, 2.0, 300.0],
            "the last delay is the give-up retry"
        );
        assert_eq!(h.launcher.launch_count(), 3);
        let published = h.published();
        assert_eq!(published.len(), 2);
        assert!(published.last().unwrap().is_none());
    }

    #[test]
    fn launch_throwing_counts_as_failure() {
        let mut h = Harness::new();
        h.launcher
            .set_error(Some(LaunchError::Spawn("fake".into())));
        let source = h.make_source_with(&[1.0, 2.0, 4.0], 300.0);
        source.start();
        for _ in 0..3 {
            h.run_pending();
        }

        assert_eq!(h.delays(), vec![1.0, 2.0, 4.0, 300.0]);
        assert_eq!(h.launcher.launch_count(), 4);
        let published = h.published();
        assert_eq!(published.len(), 1);
        assert!(published[0].is_none());
    }

    #[test]
    fn stop_terminates_and_does_not_restart() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        source.stop();
        h.launcher.exit_with(15);
        h.run_pending();

        assert_eq!(h.launcher.terminate_count(0), 1);
        assert!(h.delays().is_empty());
        assert_eq!(h.launcher.launch_count(), 1);
    }

    #[test]
    fn stop_cancels_pending_restart() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.exit();
        source.stop();
        h.run_pending();

        assert_eq!(h.launcher.launch_count(), 1);
    }

    #[test]
    fn line_after_exit_is_ignored_and_does_not_reset_backoff() {
        let mut h = Harness::new();
        let source = h.make_source_with(&[1.0, 2.0], 300.0);
        source.start();
        h.launcher.exit();
        h.launcher.emit(TRACK_LINE);
        h.run_pending();
        h.launcher.exit();

        assert!(h.published().is_empty());
        assert_eq!(h.delays(), vec![1.0, 2.0]);
        assert_eq!(h.launcher.launch_count(), 2);
    }

    #[test]
    fn stop_publishes_nil() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.emit(TRACK_LINE);
        source.stop();

        let published = h.published();
        assert_eq!(published.len(), 2);
        assert!(published.last().unwrap().is_none());
        source.start();
        assert_eq!(
            h.published().len(),
            2,
            "a restart must not replay the old snapshot"
        );
    }

    // ---- Give-up retry and wake

    #[test]
    fn give_up_schedules_retry_with_injected_interval() {
        let mut h = Harness::new();
        let source = h.make_source_with(&[1.0], 42.0);
        source.start();
        h.launcher.exit();
        h.run_pending();
        h.launcher.exit();

        assert_eq!(h.delays(), vec![1.0, 42.0]);
        assert_eq!(h.launcher.launch_count(), 2);
    }

    #[test]
    fn give_up_retry_relaunches_with_counters_reset() {
        let mut h = Harness::new();
        let _source = make_given_up_source(&mut h);
        assert_eq!(h.launcher.launch_count(), 3);

        h.run_pending();
        assert_eq!(h.launcher.launch_count(), 4);

        h.launcher.exit();
        assert_eq!(
            h.delays(),
            vec![1.0, 2.0, 300.0, 1.0],
            "the delay sequence restarts at its first entry"
        );
    }

    #[test]
    fn stop_cancels_pending_give_up_retry() {
        let mut h = Harness::new();
        let source = make_given_up_source(&mut h);
        source.stop();
        h.run_pending();

        assert_eq!(h.launcher.launch_count(), 3);
    }

    #[test]
    fn stale_give_up_retry_does_nothing() {
        let mut h = Harness::new();
        let source = make_given_up_source(&mut h);
        source.start();
        assert_eq!(h.launcher.launch_count(), 4);
        h.run_pending();

        assert_eq!(
            h.launcher.launch_count(),
            4,
            "the retry belongs to an earlier generation"
        );
    }

    #[test]
    fn wake_restarts_running_helper_with_counters_reset() {
        let mut h = Harness::new();
        let source = h.make_source_with(&[1.0, 2.0], 300.0);
        source.start();
        h.launcher.exit();
        h.run_pending();
        assert_eq!(h.launcher.launch_count(), 2);

        source.wake_from_sleep();

        assert_eq!(h.launcher.launch_count(), 3);
        assert_eq!(h.launcher.terminate_count(1), 1);
        assert_eq!(h.launcher.terminate_count(2), 0);
        h.launcher.exit();
        assert_eq!(
            h.delays(),
            vec![1.0, 1.0],
            "the failure count was reset by the wake"
        );
    }

    #[test]
    fn wake_after_give_up_relaunches() {
        let mut h = Harness::new();
        let source = make_given_up_source(&mut h);
        assert_eq!(h.launcher.launch_count(), 3);

        source.wake_from_sleep();
        assert_eq!(h.launcher.launch_count(), 4);

        h.launcher.exit();
        assert_eq!(h.delays(), vec![1.0, 2.0, 300.0, 1.0]);
        h.run_pending();
        assert_eq!(
            h.launcher.launch_count(),
            5,
            "only the live retry runs; the give-up retry is stale"
        );
    }

    #[test]
    fn wake_after_stop_does_nothing() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        source.stop();
        source.wake_from_sleep();

        assert_eq!(h.launcher.launch_count(), 1);
    }

    #[test]
    fn wake_before_start_does_nothing() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.wake_from_sleep();

        assert_eq!(h.launcher.launch_count(), 0);
    }

    // ---------------------------------------------------------------- 計畫指定的額外案例

    #[test]
    fn launch_failure_counts_toward_giving_up() {
        // perl 或動態庫不存在：每次 launch 都回錯，不崩潰，延遲用完後發布 None 並排定放棄後的重試。
        for error in [LaunchError::PerlNotFound, LaunchError::DylibNotFound] {
            let mut h = Harness::new();
            h.launcher.set_error(Some(error));
            let source = h.make_source();
            source.start();
            for _ in 0..6 {
                h.run_pending();
            }

            assert_eq!(h.launcher.launch_count(), 7);
            assert_eq!(h.delays(), vec![1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 300.0]);
            assert_eq!(h.published(), vec![None]);
        }
    }

    #[test]
    fn stale_output_from_a_dead_process_does_not_reset_the_failure_count() {
        let mut h = Harness::new();
        let source = h.make_source_with(&[1.0, 2.0], 300.0);
        source.start();
        h.launcher.exit();
        // 已死的第一代還有緩衝中的輸出。
        h.launcher.emit(TRACK_LINE);
        h.run_pending();
        // 第二代已在跑時，第一代的殘留輸出才到。
        h.launcher.emit_from(0, TRACK_LINE);
        h.launcher.exit();
        h.launcher.emit_from(1, TRACK_LINE);
        h.run_pending();
        h.launcher.exit();

        assert!(
            h.published().iter().all(|p| p.is_none()),
            "stale output must not be published"
        );
        assert_eq!(h.published(), vec![None], "only the give-up publishes");
        assert_eq!(h.delays(), vec![1.0, 2.0, 300.0]);
        assert_eq!(h.launcher.launch_count(), 3);
    }

    #[test]
    fn stop_then_start_does_not_replay_the_last_snapshot() {
        let mut h = Harness::new();
        let source = h.make_source();
        source.start();
        h.launcher.emit(TRACK_LINE);
        source.stop();
        source.start();

        let published = h.published();
        assert_eq!(published.len(), 2);
        assert!(published[0].is_some());
        assert!(published[1].is_none());

        // 重新開始後才訂閱的人，重播的也是 None 而不是上一輪的曲目。
        let replayed = Arc::new(Mutex::new(Vec::new()));
        let r = replayed.clone();
        let _sub = source.subscribe(Box::new(move |s| r.lock().unwrap().push(s.clone())));
        assert_eq!(*replayed.lock().unwrap(), vec![None]);
    }

    /// 在 `launch()` 內就同步送出事件的啟動器：不得死結，事件照常處理。
    struct SynchronousLauncher {
        inner: FakeLauncher,
        lines: Vec<HelperEvent>,
    }

    impl HelperLauncher for SynchronousLauncher {
        fn launch(&self, sink: Sink) -> Result<Box<dyn HelperHandle>, LaunchError> {
            let handle = self.inner.launch(sink.clone())?;
            for event in &self.lines {
                sink(match event {
                    HelperEvent::Line(l) => HelperEvent::Line(l.clone()),
                    HelperEvent::Exited(s) => HelperEvent::Exited(*s),
                });
            }
            Ok(handle)
        }
    }

    #[test]
    fn synchronous_launcher_events_inside_launch_do_not_deadlock() {
        let launcher = Arc::new(SynchronousLauncher {
            inner: FakeLauncher::default(),
            lines: vec![
                HelperEvent::Line(TRACK_LINE.to_string()),
                HelperEvent::Exited(1),
            ],
        });
        let clock = Arc::new(RecordingClock {
            inner: FakeClock::new(0.0),
            delays: Mutex::new(Vec::new()),
        });
        let spawner = LocalSpawner::new();
        let source = HelperSupervisor::new(
            launcher.clone(),
            clock.clone(),
            Arc::new(spawner.clone()),
            SupervisorConfig::default(),
        );
        let published = Arc::new(Mutex::new(Vec::new()));
        let p = published.clone();
        let _sub = source.subscribe(Box::new(move |s| p.lock().unwrap().push(s.clone())));
        source.start();

        // 曲目被發布；行程在 launch 內就結束，所以排了一次重試，且那個已死的把手被 terminate。
        assert_eq!(published.lock().unwrap().len(), 2);
        assert!(published.lock().unwrap()[1].is_some());
        assert_eq!(*clock.delays.lock().unwrap(), vec![1.0]);
        assert_eq!(launcher.inner.launch_count(), 1);
        assert_eq!(launcher.inner.terminate_count(0), 1);

        clock.inner.advance(Duration::from_secs(1));
        spawner.run_until_stalled();
        assert_eq!(launcher.inner.launch_count(), 2);
        // 每次啟動都在 launch 內送出一行，所以失敗計數一直被重置。
        assert_eq!(*clock.delays.lock().unwrap(), vec![1.0, 1.0]);
    }

    #[test]
    fn default_config_matches_swift() {
        let cfg = SupervisorConfig::default();
        assert_eq!(
            cfg.retry_delays,
            [1, 2, 4, 8, 16, 30].map(Duration::from_secs).to_vec()
        );
        assert_eq!(cfg.give_up_retry_interval, Duration::from_secs(300));
    }
}
