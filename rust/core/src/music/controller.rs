//! 音樂控制器：餵給音樂畫面的目前快照、該曲目的歌詞，以及播放控制指令
//! （移植自 Swift `MusicViewModel.swift`）。
//!
//! 歌詞在**曲目改變**時才查（以 `TrackIdentity::cache_key` 判斷），只有播放狀態或位置錨點改變時不重查、
//! 也不清掉歌詞。同一個 key 的**曲長**改變也算換曲：查詢發出時不知道曲長、現在知道了，或和查詢發出時的
//! 曲長差超過 2 秒（版本清單是按曲長把關的，記住的選擇也按曲長分開）。比的是查詢發出時的曲長而不是
//! 上一個快照，所以一點一點漂移也會超過上限；沒有曲長（或非有限值、不大於 0）的快照沿用目前的查詢。
//! 換曲時：丟掉舊查詢的 [`TaskHandle`]（drop 即取消）、清空歌詞與本曲的版本和偏移，
//! 有 identity 才用 [`Spawner`] 起新查詢。查詢完成時只在控制器「目前的 key」仍等於該查詢的 key 時才寫入
//! （過期結果守門）：即使舊查詢沒有被取消（例如執行器沒有原生取消），晚到的結果也不會蓋掉新歌。
//! 守門同時比對世代計數（[`Core::generation`]）：A→B→A 時，第一個 A 的查詢與第二個 A 的 key 相同，
//! 只比 key 擋不住它（因曲長改變而重查時也一樣）；世代每次換曲都加一，所以被取代的查詢一定過期。
//!
//! 版本與偏移：查詢結束時以**這次查詢發出時**的 identity 找記住的選擇（[`ChoiceBook::find`]）並套到
//! 結果上（[`apply_choice`]）；在那之前不顯示存下的內容。只有使用者的明確動作
//! （[`MusicController::next_version`]、[`MusicController::nudge_offset`]、
//! [`MusicController::reset_offset`]）而且真的改變了什麼，才寫進紀錄並呼叫 [`ChoiceSaver`]；單純播放不寫。
//! 紀錄用同一個 identity，存的是未偏移、未轉繁的行。偏移每個版本網址各一個，換曲就清掉。
//! 不用目前快照的曲長：它會在容忍內晃動或暫時消失，同一次播放的選擇會散成兩筆，或記到下次找不到的
//! 地方；曲長真的晚到或改變時已經重查，所以查詢發出時的曲長就是這次播放的曲長。
//!
//! 顯示的欄位（[`MusicState::lyrics`]、`lyrics_url`、`version`）只由 [`Core::rebuild`] 從版本清單、
//! 索引、偏移與簡轉繁開關產生。簡轉繁（[`MusicController::set_traditional_lyrics`]）只影響顯示：
//! 控制器保留提供者給的原始版本，開關切換時重新產生，不重查。
//!
//! 並行：狀態（快照、版本與偏移、紀錄、目前 key、查詢的 identity、世代、查詢把手）由單一 mutex
//! 保護；通知訂閱者與呼叫 [`ChoiceSaver`] 一律在**鎖外**，所以回呼內可以再讀 `state()` 或訂閱。
//! `spawn` 與舊把手的 drop 也都在鎖外（有的執行器會在 `spawn` 內就地執行，或在 drop 把手時同步執行
//! 取消邏輯）。
//! 每個動作的讀取、修改與寫進紀錄在同一次上鎖內完成：和換曲競爭時，動作要嘛整個落在舊歌，
//! 要嘛看到已清空的新歌而什麼都不做。
//! 監視器訂閱的回呼與查詢工作都只持有控制器的 `Weak`：控制器 drop 時沒有 Arc 循環，
//! 把手隨之 drop 而取消進行中的查詢。
//!
//! 監視器的 `subscribe` 會重播目前值，但重播可能在 `subscribe` 返回之後才抵達（別的執行緒正在送通知時），
//! 所以初始值的處理是冪等的：同一個快照再套用一次不產生任何變化。
//!
//! 通知的先後：通知在鎖外送出，所以**不保證**訂閱者依狀態變化的順序收到。監視器的回呼本身已被序列化，
//! 查詢完成的通知則來自執行器的執行緒；兩者交錯時（或回呼內又觸發較新的變化而重入時），
//! 較新的狀態可能先於較舊的抵達。訂閱者必須把通知當成「有變化」的訊號，並重讀 [`MusicController::state`]
//! （它永遠是最新的），不可信任傳入的值。外殼讓兩者都在主執行緒上時，這個交錯在實務上不會發生，
//! 但這是外殼的性質，不是本型別的保證。
//!
//! 存紀錄的先後：[`ChoiceSaver`] 在動作的呼叫端執行緒上同步呼叫，而且在該動作的通知**之前**
//! （訂閱者在回呼內又做動作時，較新的紀錄才會較晚寫出）。多條執行緒同時做動作時**不保證**寫出的先後，
//! 所以三個動作要在同一條執行緒上呼叫（外殼用主執行緒），`save` 內也不可再呼叫它們。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use crate::lyrics::identity::TrackIdentity;
use crate::lyrics::normalizer::identity_from;
use crate::lyrics::repository::LyricsResolving;
use crate::lyrics::traditional::to_traditional;
use crate::lyrics::{Lyrics, LyricsHit};
use crate::music::choices::{
    apply_choice, clamp_offset, duration_distance, known_duration, shift_lyrics, ChoiceBook,
    SavedChoice,
};
use crate::nowplaying::{NowPlayingMonitor, PlaybackCommand, PlaybackControl};
use crate::runtime::{lock, Notifier, Spawner, Subscription, TaskHandle};
use crate::types::TrackSnapshot;

/// 目前顯示的是第幾個時間軸版本，以及它的偏移。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VersionInfo {
    /// 從 0 起算。
    pub index: usize,
    pub count: usize,
    /// 秒；正值讓歌詞提早出現。
    pub offset: f64,
}

/// 把整份紀錄寫出去的函式（由 app 提供）；呼叫時機與執行緒見模組說明。
pub type ChoiceSaver = Arc<dyn Fn(&ChoiceBook) + Send + Sync>;

/// 音樂畫面讀取的狀態：目前快照、（已查到的）該曲目歌詞、歌詞的來源網址與版本資訊。
///
/// `lyrics` 與 `lyrics_url` 永遠一起設定、一起清除：有歌詞才有網址，沒命中兩者皆為 `None`。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MusicState {
    pub snapshot: Option<TrackSnapshot>,
    /// 要顯示的歌詞：目前版本套上它的偏移（開了簡轉繁時再轉換）。
    pub lyrics: Option<Lyrics>,
    /// 目前版本的來源頁面（`https://` 開頭）；沒有歌詞時為 `None`。
    pub lyrics_url: Option<String>,
    /// 目前曲目的歌詞查詢還沒有結果。沒有查詢（沒有曲目、缺演出者）時為 `false`。
    pub searching: bool,
    /// 只有正在顯示時間軸歌詞時為 `Some`。
    pub version: Option<VersionInfo>,
}

/// 依開關產生要顯示的歌詞。
fn displayed(lyrics: Option<Lyrics>, traditional: bool) -> Option<Lyrics> {
    if traditional {
        lyrics.as_ref().map(to_traditional)
    } else {
        lyrics
    }
}

#[derive(Default)]
struct Core {
    state: MusicState,
    /// 歌詞轉繁體中文開關。
    traditional: bool,
    /// 目前曲目的 `cache_key`；沒有 identity（沒有快照、缺演出者…）時為 `None`。
    current_key: Option<String>,
    /// 目前這次查詢發出時的 identity（含那時的曲長）；只在換曲時改變，判斷曲長是否改變、配紀錄與
    /// 存紀錄都用它。
    lookup_identity: Option<TrackIdentity>,
    /// 進行中的查詢；drop 即取消。
    lookup: Option<TaskHandle>,
    /// 查詢世代：每次（重新）開始或作廢查詢（換曲，含曲長改變）就加一。每個查詢記下自己開始時的世代，
    /// 完成時與 key 一起比對，所以 A→B→A 時第一個、被取代的 A 查詢即使沒被取消也寫不進第二個 A 的狀態。
    generation: u64,
    /// 本曲的歌詞版本，保持原樣（未偏移、未轉繁）；查詢還沒結束或查無時是空的。
    versions: Vec<LyricsHit>,
    /// 目前版本在 `versions` 裡的索引。
    index: usize,
    /// 本曲各版本（以網址辨認）的偏移。
    offsets: BTreeMap<String, f64>,
    /// 全部歌曲記住的選擇。
    book: ChoiceBook,
}

impl Core {
    /// 目前的快照（`key`、已知的曲長 `duration`）是否需要放棄目前的查詢與歌詞（見模組說明）。
    fn is_another_track(&self, key: Option<&str>, duration: Option<f64>) -> bool {
        let issued = self.lookup_identity.as_ref().and_then(|i| i.duration);
        self.current_key.as_deref() != key
            || (duration.is_some() && duration_distance(issued, duration).is_none())
    }

    /// 換曲：作廢舊查詢、清掉上一首的一切。回傳舊查詢的把手，呼叫端要在**鎖外** drop。
    fn change_track(
        &mut self,
        key: Option<String>,
        identity: Option<TrackIdentity>,
    ) -> Option<TaskHandle> {
        self.generation += 1;
        // 有 key 的話呼叫端一定接著開始查詢。
        self.state.searching = key.is_some();
        self.current_key = key;
        self.lookup_identity = identity;
        self.versions.clear();
        self.index = 0;
        self.offsets.clear();
        self.rebuild();
        self.lookup.take()
    }

    /// 收下查詢結果：套上記住的選擇與它的偏移。
    fn adopt(&mut self, found: Vec<LyricsHit>) {
        let saved = self
            .lookup_identity
            .as_ref()
            .and_then(|identity| self.book.find(identity));
        let (versions, index) = apply_choice(found, saved);
        self.offsets = saved.map(|s| s.offsets.clone()).unwrap_or_default();
        self.versions = versions;
        self.index = index;
    }

    /// 該版本的偏移；紀錄來自檔案，所以讀的時候也限制範圍。
    fn offset_of(&self, url: &str) -> f64 {
        clamp_offset(self.offsets.get(url).copied().unwrap_or(0.0))
    }

    /// 由版本清單、索引、偏移與簡轉繁開關重建要顯示的欄位；`lyrics`、`lyrics_url`、`version` 只在這裡寫。
    fn rebuild(&mut self) {
        let current = self.versions.get(self.index);
        let offset = current.map_or(0.0, |hit| self.offset_of(&hit.url));
        let shifted = current.map(|hit| match &hit.lyrics {
            Lyrics::Synced(lines) => Lyrics::Synced(shift_lyrics(lines, offset)),
            plain => plain.clone(),
        });
        let url = current.map(|hit| hit.url.clone());
        let version = current
            .filter(|hit| hit.lyrics.is_synced())
            .map(|_| VersionInfo {
                index: self.index,
                count: self.versions.len(),
                offset,
            });
        self.state.lyrics = displayed(shifted, self.traditional);
        self.state.lyrics_url = url;
        self.state.version = version;
    }

    /// 把目前版本與本曲的偏移寫進紀錄。
    fn remember(&mut self) {
        let current = self.versions.get(self.index);
        let (Some(identity), Some(hit)) = (&self.lookup_identity, current) else {
            return;
        };
        let Lyrics::Synced(lines) = &hit.lyrics else {
            return;
        };
        self.book.upsert(SavedChoice {
            key: identity.cache_key(),
            duration: known_duration(identity.duration),
            url: hit.url.clone(),
            lines: lines.clone(),
            offsets: self.offsets.clone(),
        });
    }
}

pub struct MusicController {
    core: Mutex<Core>,
    notifier: Notifier<MusicState>,
    resolver: Arc<dyn LyricsResolving>,
    control: Option<Arc<dyn PlaybackControl>>,
    spawner: Arc<dyn Spawner>,
    save: ChoiceSaver,
    /// 對監視器的訂閱；控制器 drop 時一併取消。包在 mutex 裡只是為了建構後再放進去、並讓控制器是 `Sync`。
    _monitor_subscription: Mutex<Option<Subscription>>,
}

impl MusicController {
    /// 同 [`Self::new_with_choices`]，但沒有記住的選擇、動作也不寫到任何地方（只在本次執行期間有效）。
    ///
    /// **監視器的存活期必須由呼叫端負責**：控制器**不**持有 `Arc<NowPlayingMonitor>`，
    /// 對監視器的訂閱也隨監視器 drop 而取消。監視器一旦被 drop，快照更新就悄悄停止
    /// （狀態與依賴它的 `is_active` 卡在最後的值，沒有任何錯誤）。外殼要在控制器存活期間保有該 `Arc`。
    pub fn new(
        monitor: Arc<NowPlayingMonitor>,
        resolver: Arc<dyn LyricsResolving>,
        control: Option<Arc<dyn PlaybackControl>>,
        spawner: Arc<dyn Spawner>,
    ) -> Arc<Self> {
        Self::new_with_choices(
            monitor,
            resolver,
            control,
            spawner,
            ChoiceBook::default(),
            Arc::new(|_| {}),
        )
    }

    /// 建立控制器並訂閱 `monitor`（目前值會被冪等地套用）。`book` 是記住的歌詞版本與偏移；
    /// 使用者改變它時以整份新紀錄呼叫 `save`。監視器的存活期見 [`Self::new`]。
    pub fn new_with_choices(
        monitor: Arc<NowPlayingMonitor>,
        resolver: Arc<dyn LyricsResolving>,
        control: Option<Arc<dyn PlaybackControl>>,
        spawner: Arc<dyn Spawner>,
        book: ChoiceBook,
        save: ChoiceSaver,
    ) -> Arc<Self> {
        let controller = Arc::new(Self {
            core: Mutex::new(Core {
                book,
                ..Core::default()
            }),
            notifier: Notifier::new(),
            resolver,
            control,
            spawner,
            save,
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

    /// 訂閱狀態變動（狀態真的改變才通知）。不會在訂閱當下重播目前值；要的話先讀 [`Self::state`]。
    /// 回呼在鎖外執行；drop 回傳的把手即取消。
    ///
    /// **傳給回呼的值只是「這次變動當下」的狀態，不保證是最新的，也不保證通知依狀態的先後順序抵達**：
    /// 若另一條執行緒（例如執行器完成查詢）在這次通知送出前又改了狀態，較新的通知可能先到、
    /// 這個較舊的後到。訂閱者應把通知當成「有變化」的訊號，回呼內一律重讀 [`Self::state`]
    /// （永遠是最新的），不要信任傳入的參數。
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

    /// 開或關「歌詞轉繁體中文」（見 [`to_traditional`]：日文與本來就是繁體的歌詞不受影響）。
    /// 立即套用到目前的歌詞與之後查到的歌詞；顯示內容真的改變時才通知訂閱者。
    pub fn set_traditional_lyrics(&self, on: bool) {
        let state = {
            let mut core = lock(&self.core);
            let before = core.state.lyrics.clone();
            core.traditional = on;
            core.rebuild();
            if core.state.lyrics == before {
                return;
            }
            core.state.clone()
        };
        self.notifier.notify(&state);
    }

    /// 換到下一個歌詞版本（最後一個之後回到第一個），套用那個版本自己的偏移並記住。
    /// 沒有顯示時間軸歌詞，或版本少於兩個時什麼都不做。
    pub fn next_version(&self) {
        self.act(|core| {
            if core.versions.len() < 2 {
                return false;
            }
            core.index = (core.index + 1) % core.versions.len();
            true
        });
    }

    /// 目前版本的偏移加 `delta` 秒（正值讓歌詞提早出現），限制在
    /// ±[`OFFSET_LIMIT`](crate::music::choices::OFFSET_LIMIT) 並記住。
    /// 沒有顯示時間軸歌詞、偏移沒有改變或 `delta` 是 NaN 時什麼都不做。
    pub fn nudge_offset(&self, delta: f64) {
        self.change_offset(|offset| offset + delta);
    }

    /// 目前版本的偏移歸零並記住；沒有顯示時間軸歌詞或本來就是零時什麼都不做。
    pub fn reset_offset(&self) {
        self.change_offset(|_| 0.0);
    }

    fn change_offset(&self, change: impl FnOnce(f64) -> f64) {
        self.act(|core| {
            let Some(url) = core.state.lyrics_url.clone() else {
                return false;
            };
            let old = core.offset_of(&url);
            let new = change(old);
            // NaN 不能交給 `clamp_offset`：它會變成 0，把原本的偏移清掉。
            if new.is_nan() || clamp_offset(new) == old {
                return false;
            }
            core.offsets.insert(url, clamp_offset(new));
            true
        });
    }

    /// 使用者的明確動作，也是唯一寫紀錄的路徑。只在顯示時間軸歌詞時呼叫 `change`；
    /// 它回傳 `false`（什麼都沒變）時不存也不通知。
    fn act(&self, change: impl FnOnce(&mut Core) -> bool) {
        let (state, book) = {
            let mut core = lock(&self.core);
            if core.state.version.is_none() || !change(&mut core) {
                return;
            }
            core.rebuild();
            core.remember();
            (core.state.clone(), core.book.clone())
        };
        // 先存再通知（見模組說明）。
        (self.save)(&book);
        self.notifier.notify(&state);
    }

    fn apply(self: &Arc<Self>, new: Option<TrackSnapshot>) {
        let identity = new.as_ref().and_then(identity_from);
        let key = identity.as_ref().map(|i| i.cache_key());
        let duration = identity.as_ref().and_then(|i| known_duration(i.duration));

        let (changed, state, old_lookup, new_track, generation) = {
            let mut core = lock(&self.core);
            let new_track = core.is_another_track(key.as_deref(), duration);
            let before = core.state.clone();
            core.state.snapshot = new;
            let old_lookup = if new_track {
                core.change_track(key.clone(), identity.clone())
            } else {
                None
            };
            (
                core.state != before,
                core.state.clone(),
                old_lookup,
                new_track,
                core.generation,
            )
        };
        // 舊查詢在鎖外取消。
        drop(old_lookup);

        if new_track {
            if let (Some(identity), Some(key)) = (identity, key) {
                self.start_lookup(identity, key, generation);
            }
        }
        if changed {
            self.notifier.notify(&state);
        }
    }

    fn start_lookup(self: &Arc<Self>, identity: TrackIdentity, key: String, generation: u64) {
        let resolver = self.resolver.clone();
        let weak: Weak<Self> = Arc::downgrade(self);
        let task_key = key.clone();
        let handle = self.spawner.spawn(Box::pin(async move {
            let result = resolver.lyrics(identity).await;
            if let Some(controller) = weak.upgrade() {
                controller.finish_lookup(&task_key, generation, result);
            }
        }));
        let stale = {
            let mut core = lock(&self.core);
            if core.generation == generation && core.current_key.as_deref() == Some(key.as_str()) {
                core.lookup = Some(handle);
                None
            } else {
                // spawn 期間曲目又變了：這個查詢已過期，不保留把手（drop 即取消）。
                Some(handle)
            }
        };
        drop(stale);
    }

    /// 查詢完成：曲目可能在請求期間改變；key 或世代不同（含 A→B→A 的舊 A）就丟棄。
    /// 查詢中結束（包含沒命中），或顯示的內容改變才通知。不寫紀錄。
    fn finish_lookup(&self, key: &str, generation: u64, found: Vec<LyricsHit>) {
        let state = {
            let mut core = lock(&self.core);
            if core.generation != generation || core.current_key.as_deref() != Some(key) {
                return;
            }
            let before = core.state.clone();
            core.state.searching = false;
            core.adopt(found);
            core.rebuild();
            if core.state == before {
                return;
            }
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
    use crate::lyrics::{LyricLine, LyricsHit};
    use crate::music::choices::SavedChoice;
    use crate::nowplaying::{NowPlayingSource, SharedSnapshotCallback, SnapshotCallback};
    use crate::runtime::{BoxFuture, Notifier};
    use crate::testing::LocalSpawner;
    use crate::types::PositionAnchor;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::OnceLock;
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

    /// 測試用命中：網址固定。
    fn hit(lyrics: Lyrics) -> LyricsHit {
        LyricsHit::new(lyrics, "https://test.example/0")
    }

    /// 測試用命中：指定網址。
    fn hit_at(lyrics: Option<Lyrics>, url: &str) -> Option<LyricsHit> {
        lyrics.map(|l| LyricsHit::new(l, url))
    }

    /// 一次查詢的閘門：test 呼叫 `answer` 之前，對應的 future 一直 Pending。
    #[derive(Default)]
    struct Gate {
        slot: Mutex<GateSlot>,
    }

    #[derive(Default)]
    struct GateSlot {
        answer: Option<Vec<LyricsHit>>,
        waker: Option<Waker>,
    }

    impl Gate {
        fn answer(&self, versions: Vec<LyricsHit>) {
            let waker = {
                let mut slot = lock(&self.slot);
                slot.answer = Some(versions);
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
        type Output = Vec<LyricsHit>;
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
            self.respond_hit(index, lyrics.map(hit));
        }

        fn respond_hit(&self, index: usize, hit: Option<LyricsHit>) {
            self.respond_versions(index, hit.into_iter().collect());
        }

        fn respond_versions(&self, index: usize, versions: Vec<LyricsHit>) {
            let gate = lock(&self.gates)[index].clone();
            gate.answer(versions);
        }
    }

    impl LyricsResolving for GatedResolver {
        fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Vec<LyricsHit>> {
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

        fn respond_hit(&self, index: usize, hit: Option<LyricsHit>) {
            self.resolver.respond_hit(index, hit);
            self.spawner.run_until_stalled();
        }
    }

    /// 不取消被丟掉的把手：模擬「把手被 detach」或執行器沒有原生取消的情境，
    /// 此時過期查詢仍會跑到完成，只能靠完成時的守門擋下。
    struct DetachingSpawner(LocalSpawner);

    impl Spawner for DetachingSpawner {
        fn spawn(&self, f: BoxFuture<'static, ()>) -> TaskHandle {
            self.0.spawn(f).detach();
            TaskHandle::new(|| {})
        }
    }

    /// 解析器回傳版本清單；沒有記住的選擇時，控制器顯示第一個（預設版本）。
    #[test]
    fn shows_the_first_of_the_resolved_versions() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));

        let versions = vec![
            LyricsHit::new(Lyrics::Plain("one".into()), "https://test.example/1"),
            LyricsHit::new(Lyrics::Plain("two".into()), "https://test.example/2"),
        ];
        fx.resolver.respond_versions(0, versions);
        fx.spawner.run_until_stalled();

        let state = controller.state();
        assert_eq!(state.lyrics, plain("one"));
        assert_eq!(state.lyrics_url.as_deref(), Some("https://test.example/1"));
        assert!(!state.searching);
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

    /// A→B→A：第一個（被取代的）A 查詢沒被取消、先於第二個 A 回來時，只比 key 會把它當成目前曲目的結果。
    /// 世代計數讓它被擋下；第二個 A 的答案才是有效的。
    #[test]
    fn superseded_lookup_for_the_same_track_is_rejected_by_generation() {
        let fx = Fixture::new();
        let controller = MusicController::new(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(DetachingSpawner(fx.spawner.clone())),
        );
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        fx.emit(song("Song A"));
        assert_eq!(
            fx.resolver.requested_titles(),
            ["Song A", "Song B", "Song A"]
        );
        assert_eq!(
            fx.resolver.dropped.load(Ordering::SeqCst),
            0,
            "沒有任何查詢被取消"
        );

        fx.respond(0, plain("first A (superseded)"));
        assert_eq!(
            controller.state().lyrics,
            None,
            "the superseded lookup shares the key but not the generation"
        );

        fx.respond(1, plain("lyrics of B"));
        assert_eq!(controller.state().lyrics, None, "B is no longer the track");

        fx.respond(2, plain("second A"));
        assert_eq!(controller.state().lyrics, plain("second A"));
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
    fn searching_is_set_while_the_lookup_is_in_flight_and_cleared_by_a_hit() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        assert!(!controller.state().searching);

        fx.emit(song("Song A"));
        assert!(controller.state().searching);
        fx.respond(0, plain("lyrics of A"));
        assert!(!controller.state().searching);

        // 同一首只是暫停：不重查，也不回到查詢中。
        fx.emit(snapshot("Song A", Some("Artist"), false, None));
        assert!(!controller.state().searching);
        // 換歌重新查；沒有曲目就沒有查詢。
        fx.emit(song("Song B"));
        assert!(controller.state().searching);
        fx.emit(None);
        assert!(!controller.state().searching);
    }

    #[test]
    fn a_miss_ends_the_search_and_notifies_once() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        let seen: Arc<Mutex<Vec<MusicState>>> = Arc::default();
        let s = seen.clone();
        let _sub = controller.subscribe(Box::new(move |state| lock(&s).push(state.clone())));

        fx.respond(0, None);

        assert!(!controller.state().searching);
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(lock(&seen).len(), 1);
    }

    #[test]
    fn a_track_without_artist_is_not_searching() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.emit(snapshot("No Artist Song", None, true, None));
        assert!(!controller.state().searching);
    }

    #[test]
    fn a_stale_result_does_not_end_the_new_tracks_search() {
        let fx = Fixture::new();
        let controller = MusicController::new(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(DetachingSpawner(fx.spawner.clone())),
        );
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        fx.respond(0, None);
        assert!(controller.state().searching);
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
    fn hit_sets_lyrics_and_url_together() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        assert_eq!(controller.state().lyrics_url, None);

        fx.respond_hit(0, hit_at(plain("words"), "https://lrclib.net/api/get/7"));

        let state = controller.state();
        assert_eq!(state.lyrics, plain("words"));
        assert_eq!(
            state.lyrics_url.as_deref(),
            Some("https://lrclib.net/api/get/7")
        );

        // 只有播放狀態改變：歌詞與網址都留著。
        fx.emit(snapshot("Song A", Some("Artist"), false, None));
        assert_eq!(controller.state().lyrics, plain("words"));
        assert_eq!(
            controller.state().lyrics_url.as_deref(),
            Some("https://lrclib.net/api/get/7")
        );
    }

    #[test]
    fn a_miss_leaves_both_lyrics_and_url_empty() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond_hit(0, None);
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);
    }

    #[test]
    fn track_change_clears_lyrics_and_url_together() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond_hit(0, hit_at(plain("lyrics of A"), "https://a.example/1"));
        assert!(controller.state().lyrics.is_some());
        assert!(controller.state().lyrics_url.is_some());

        fx.emit(song("Song B"));

        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);
    }

    #[test]
    fn no_artist_or_no_snapshot_clears_the_url() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond_hit(0, hit_at(plain("lyrics of A"), "https://a.example/1"));
        assert!(controller.state().lyrics_url.is_some());

        fx.emit(snapshot("No Artist Song", None, true, None));
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);

        fx.emit(song("Song B"));
        fx.respond_hit(1, hit_at(plain("lyrics of B"), "https://b.example/1"));
        assert!(controller.state().lyrics_url.is_some());

        fx.emit(None);
        assert_eq!(controller.state().snapshot, None);
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);
    }

    /// 上一首的查詢在換歌之後才回來：它的網址絕不能留在新曲目上（取消與不取消兩種執行器都一樣）。
    #[test]
    fn slow_lookup_for_the_previous_track_never_leaves_its_url_on_the_new_track() {
        for detaching in [false, true] {
            let fx = Fixture::new();
            let spawner: Arc<dyn Spawner> = if detaching {
                Arc::new(DetachingSpawner(fx.spawner.clone()))
            } else {
                Arc::new(fx.spawner.clone())
            };
            let controller =
                MusicController::new(fx.monitor.clone(), fx.resolver.clone(), None, spawner);
            fx.emit(song("Song A"));
            fx.emit(song("Song B"));
            assert_eq!(fx.resolver.requested().len(), 2);

            // B 還在查：A 的晚到結果不得填入歌詞或網址。
            fx.respond_hit(0, hit_at(plain("lyrics of A"), "https://a.example/1"));
            assert_eq!(controller.state().lyrics, None);
            assert_eq!(controller.state().lyrics_url, None);

            fx.respond_hit(1, hit_at(plain("lyrics of B"), "https://b.example/1"));
            assert_eq!(controller.state().lyrics, plain("lyrics of B"));
            assert_eq!(
                controller.state().lyrics_url.as_deref(),
                Some("https://b.example/1")
            );

            // A 的查詢（若還活著）在 B 已完成之後才回來：網址仍是 B 的。
            fx.respond_hit(0, hit_at(plain("lyrics of A"), "https://a.example/1"));
            assert_eq!(controller.state().lyrics, plain("lyrics of B"));
            assert_eq!(
                controller.state().lyrics_url.as_deref(),
                Some("https://b.example/1")
            );
        }
    }

    /// A→B→A：被取代的第一個 A 查詢（沒被取消）與第二個 A 的 key 相同，只靠世代擋下；網址也不得寫入。
    #[test]
    fn stale_generation_result_does_not_write_the_url() {
        let fx = Fixture::new();
        let controller = MusicController::new(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(DetachingSpawner(fx.spawner.clone())),
        );
        fx.emit(song("Song A"));
        fx.emit(song("Song B"));
        fx.emit(song("Song A"));

        fx.respond_hit(0, hit_at(plain("first A"), "https://first-a.example/1"));
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);

        fx.respond_hit(2, hit_at(plain("second A"), "https://second-a.example/1"));
        assert_eq!(controller.state().lyrics, plain("second A"));
        assert_eq!(
            controller.state().lyrics_url.as_deref(),
            Some("https://second-a.example/1")
        );

        // 被取代的查詢之後才回來也一樣擋下。
        fx.respond_hit(1, hit_at(plain("lyrics of B"), "https://b.example/1"));
        assert_eq!(
            controller.state().lyrics_url.as_deref(),
            Some("https://second-a.example/1")
        );
    }

    /// 同一份歌詞、不同網址也算變化（通知一次）；完全相同的重複結果不通知。
    /// 正常流程一次查詢只完成一次，所以這裡直接以同一個 key／世代再呼叫 `finish_lookup`。
    #[test]
    fn subscribers_are_notified_when_only_the_url_changes() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond_hit(0, hit_at(plain("words"), "https://one.example/1"));

        let seen: Arc<Mutex<Vec<MusicState>>> = Arc::default();
        let s = seen.clone();
        let _sub = controller.subscribe(Box::new(move |state| lock(&s).push(state.clone())));

        let (key, generation) = {
            let core = lock(&controller.core);
            (core.current_key.clone().unwrap(), core.generation)
        };
        let again = |url: &str| {
            let found = hit_at(plain("words"), url).into_iter().collect();
            controller.finish_lookup(&key, generation, found);
        };

        again("https://one.example/1"); // 完全相同
        assert_eq!(lock(&seen).len(), 0);

        again("https://two.example/2"); // 只有網址不同
        assert_eq!(lock(&seen).len(), 1);
        assert_eq!(lock(&seen)[0].lyrics, plain("words"));
        assert_eq!(
            lock(&seen)[0].lyrics_url.as_deref(),
            Some("https://two.example/2")
        );
        assert_eq!(
            controller.state().lyrics_url.as_deref(),
            Some("https://two.example/2")
        );

        again("https://two.example/2"); // 重複
        assert_eq!(lock(&seen).len(), 1);
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
        // 查無結果：歌詞仍是 None，但「查詢中」結束了，畫面要換標籤，所以通知。
        fx.emit(song("Song B"));
        assert_eq!(lock(&seen).len(), 3);
        assert_eq!(lock(&seen)[2].lyrics, None);
        assert!(lock(&seen)[2].searching);
        fx.respond(1, None);
        assert_eq!(lock(&seen).len(), 4);
        assert!(!lock(&seen)[3].searching);
    }

    /// 通知在鎖外送出，所以重入（回呼內又觸發較新的變化）時，較新的通知可能先於較舊的抵達。
    /// 因此訂閱者收到的值不保證單調；`state()` 才是權威，永遠是最新的。
    /// 此測試固定這個行為（若之後改成保證順序，更新此測試與 `subscribe` 的文件）。
    #[test]
    fn subscribers_may_see_an_older_value_after_a_newer_one_but_state_is_always_latest() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        let spawner = fx.spawner.clone();
        let resolver = fx.resolver.clone();
        let reentered = Arc::new(AtomicUsize::new(0));

        // 第一位訂閱者在收到「A 剛出現、歌詞尚未到」的通知時，當場讓歌詞查詢完成，
        // 於是「A + 歌詞」的新通知在原本那一輪通知還沒送完時就先送出。
        let r = reentered.clone();
        let _first = controller.subscribe(Box::new(move |state| {
            if state.lyrics.is_none() && r.fetch_add(1, Ordering::SeqCst) == 0 {
                spawner.run_until_stalled(); // 讓查詢真正發出
                resolver.respond(0, plain("words"));
                spawner.run_until_stalled();
            }
        }));
        /// （傳入回呼的歌詞，回呼當下 `state()` 的歌詞）
        type Seen = Vec<(Option<Lyrics>, Option<Lyrics>)>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let (s, c) = (seen.clone(), Arc::downgrade(&controller));
        let _second = controller.subscribe(Box::new(move |state| {
            let latest = c.upgrade().unwrap().state().lyrics;
            lock(&s).push((state.lyrics.clone(), latest));
        }));

        fx.source.send(song("Song A"));
        fx.spawner.run_until_stalled();

        let seen = lock(&seen).clone();
        assert_eq!(
            seen.iter()
                .map(|(passed, _)| passed.clone())
                .collect::<Vec<_>>(),
            [plain("words"), None],
            "newer value first, then the older one"
        );
        assert!(
            seen.iter().all(|(_, latest)| *latest == plain("words")),
            "state() already holds the newest value in every callback"
        );
        assert_eq!(controller.state().lyrics, plain("words"));
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

    const SIMPLIFIED: &str = "我们的时间过得很快";
    const TRADITIONAL: &str = "我們的時間過得很快";

    #[test]
    fn lyrics_are_shown_as_provided_by_default() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain(SIMPLIFIED));
        assert_eq!(controller.state().lyrics, plain(SIMPLIFIED));
    }

    #[test]
    fn traditional_toggle_converts_current_lyrics_and_back_without_refetching() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain(SIMPLIFIED));
        let url = controller.state().lyrics_url;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let _sub = controller.subscribe(Box::new(move |s| lock(&log).push(s.lyrics.clone())));

        controller.set_traditional_lyrics(true);
        assert_eq!(controller.state().lyrics, plain(TRADITIONAL));
        // 同值再設一次不通知。
        controller.set_traditional_lyrics(true);
        controller.set_traditional_lyrics(false);
        assert_eq!(controller.state().lyrics, plain(SIMPLIFIED));

        assert_eq!(*lock(&seen), [plain(TRADITIONAL), plain(SIMPLIFIED)]);
        assert_eq!(
            controller.state().lyrics_url,
            url,
            "source url is untouched"
        );
        assert_eq!(fx.resolver.requested().len(), 1, "no second lookup");
    }

    #[test]
    fn lyrics_arriving_while_the_toggle_is_on_are_converted() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        controller.set_traditional_lyrics(true);
        fx.emit(song("Song A"));
        fx.respond(0, plain(SIMPLIFIED));
        assert_eq!(controller.state().lyrics, plain(TRADITIONAL));

        // 換歌後開關仍然有效。
        fx.emit(song("Song B"));
        fx.respond(1, plain("这里"));
        assert_eq!(controller.state().lyrics, plain("這裡"));
    }

    #[test]
    fn toggling_after_a_track_change_does_not_bring_back_old_lyrics() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain(SIMPLIFIED));
        fx.emit(song("Song B"));

        controller.set_traditional_lyrics(true);
        controller.set_traditional_lyrics(false);

        assert_eq!(controller.state().lyrics, None);
    }

    #[test]
    fn toggling_with_lyrics_that_need_no_conversion_does_not_notify() {
        let fx = Fixture::new();
        let controller = fx.controller(None);
        fx.emit(song("Song A"));
        fx.respond(0, plain("words"));
        let count = Arc::new(AtomicUsize::new(0));
        let n = count.clone();
        let _sub = controller.subscribe(Box::new(move |_| {
            n.fetch_add(1, Ordering::SeqCst);
        }));

        controller.set_traditional_lyrics(true);

        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    // ---- 版本與偏移 ----

    const URL_A: &str = "https://test.example/a";
    const URL_B: &str = "https://test.example/b";
    const URL_C: &str = "https://test.example/c";

    fn timed(lines: &[(f64, &str)]) -> Vec<LyricLine> {
        lines
            .iter()
            .map(|(time, text)| LyricLine {
                time: *time,
                text: (*text).to_string(),
            })
            .collect()
    }

    fn lines_a() -> Vec<LyricLine> {
        timed(&[(10.0, "a1"), (20.0, "a2"), (30.0, "a3")])
    }

    fn lines_b() -> Vec<LyricLine> {
        timed(&[(15.0, "b1"), (25.0, "b2"), (35.0, "b3")])
    }

    fn lines_c() -> Vec<LyricLine> {
        timed(&[(40.0, "c1"), (50.0, "c2"), (60.0, "c3")])
    }

    fn version(url: &str, lines: Vec<LyricLine>) -> LyricsHit {
        LyricsHit::new(Lyrics::Synced(lines), url)
    }

    /// A（預設）與 B 兩個版本。
    fn two_versions() -> Vec<LyricsHit> {
        vec![version(URL_A, lines_a()), version(URL_B, lines_b())]
    }

    fn synced(lines: Vec<LyricLine>) -> Option<Lyrics> {
        Some(Lyrics::Synced(lines))
    }

    fn info(index: usize, count: usize, offset: f64) -> Option<VersionInfo> {
        Some(VersionInfo {
            index,
            count,
            offset,
        })
    }

    fn lasting(title: &str, duration: Option<f64>) -> Option<TrackSnapshot> {
        song(title).map(|snapshot| TrackSnapshot {
            duration,
            ..snapshot
        })
    }

    fn choice(
        title: &str,
        duration: Option<f64>,
        url: &str,
        lines: Vec<LyricLine>,
        offsets: &[(&str, f64)],
    ) -> SavedChoice {
        let identity = TrackIdentity {
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: None,
            duration,
        };
        SavedChoice {
            key: identity.cache_key(),
            duration,
            url: url.to_string(),
            lines,
            offsets: offsets
                .iter()
                .map(|(url, offset)| (url.to_string(), *offset))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn book(songs: Vec<SavedChoice>) -> ChoiceBook {
        ChoiceBook { songs }
    }

    /// 每次呼叫 `save` 拿到的整份紀錄。
    type Saved = Arc<Mutex<Vec<ChoiceBook>>>;

    fn saves(saved: &Saved) -> usize {
        lock(saved).len()
    }

    fn last_saved(saved: &Saved) -> Vec<SavedChoice> {
        lock(saved).last().expect("nothing was saved").songs.clone()
    }

    fn act_all(controller: &MusicController) {
        controller.next_version();
        controller.nudge_offset(0.5);
        controller.nudge_offset(-5.0);
        controller.reset_offset();
    }

    fn count_notifications(controller: &MusicController) -> (Arc<AtomicUsize>, Subscription) {
        let count = Arc::new(AtomicUsize::new(0));
        let n = count.clone();
        let subscription = controller.subscribe(Box::new(move |_| {
            n.fetch_add(1, Ordering::SeqCst);
        }));
        (count, subscription)
    }

    impl Fixture {
        fn controller_with(&self, book: ChoiceBook) -> (Arc<MusicController>, Saved) {
            let saved: Saved = Arc::default();
            let log = saved.clone();
            let controller = MusicController::new_with_choices(
                self.monitor.clone(),
                self.resolver.clone(),
                None,
                Arc::new(self.spawner.clone()),
                book,
                Arc::new(move |book: &ChoiceBook| lock(&log).push(book.clone())),
            );
            (controller, saved)
        }

        fn answer(&self, index: usize, versions: Vec<LyricsHit>) {
            self.resolver.respond_versions(index, versions);
            self.spawner.run_until_stalled();
        }

        fn requested_durations(&self) -> Vec<Option<f64>> {
            self.resolver
                .requested()
                .iter()
                .map(|t| t.duration)
                .collect()
        }
    }

    #[test]
    fn first_version_is_shown_with_url_and_version_info() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());

        let state = controller.state();
        assert_eq!(state.lyrics, synced(lines_a()));
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_A));
        assert_eq!(state.version, info(0, 2, 0.0));
        assert!(!state.searching);
        assert_eq!(saves(&saved), 0);
    }

    #[test]
    fn next_version_cycles_and_saves_the_choice() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());

        controller.next_version();
        let state = controller.state();
        assert_eq!(state.version, info(1, 2, 0.0));
        assert_eq!(state.lyrics, synced(lines_b()));
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_B));
        assert_eq!(saves(&saved), 1);
        assert_eq!(
            last_saved(&saved),
            [choice("Song A", Some(200.0), URL_B, lines_b(), &[])]
        );

        controller.next_version();
        let state = controller.state();
        assert_eq!(state.version, info(0, 2, 0.0));
        assert_eq!(state.lyrics, synced(lines_a()));
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_A));
        assert_eq!(saves(&saved), 2);
        assert_eq!(
            last_saved(&saved),
            [choice("Song A", Some(200.0), URL_A, lines_a(), &[])]
        );
        assert_eq!(fx.resolver.requested().len(), 1, "切換版本不重查");
    }

    #[test]
    fn nudging_shifts_the_shown_lines_and_saves() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());

        controller.nudge_offset(0.5);

        let state = controller.state();
        assert_eq!(
            state.lyrics,
            synced(timed(&[(9.5, "a1"), (19.5, "a2"), (29.5, "a3")]))
        );
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_A));
        assert_eq!(state.version, info(0, 2, 0.5));
        assert_eq!(saves(&saved), 1);
        // 存下的是未偏移的行。
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_A,
                lines_a(),
                &[(URL_A, 0.5)]
            )]
        );

        // 第二次偏移從原始行算起，不是在偏移過的行上再偏移一次整個偏移量。
        controller.nudge_offset(0.5);
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(9.0, "a1"), (19.0, "a2"), (29.0, "a3")]))
        );
        assert_eq!(controller.state().version, info(0, 2, 1.0));
        assert_eq!(last_saved(&saved)[0].lines, lines_a());
    }

    #[test]
    fn offsets_belong_to_their_version() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        controller.nudge_offset(0.5);

        controller.next_version();
        assert_eq!(controller.state().version, info(1, 2, 0.0));
        assert_eq!(controller.state().lyrics, synced(lines_b()));

        controller.next_version();
        assert_eq!(controller.state().version, info(0, 2, 0.5));
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(9.5, "a1"), (19.5, "a2"), (29.5, "a3")]))
        );

        controller.next_version();
        controller.nudge_offset(-1.0);
        assert_eq!(controller.state().version, info(1, 2, -1.0));
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_B,
                lines_b(),
                &[(URL_A, 0.5), (URL_B, -1.0)]
            )]
        );
    }

    #[test]
    fn offset_is_clamped_and_reset_returns_to_zero() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        let (notified, _sub) = count_notifications(&controller);

        controller.nudge_offset(500.0);
        assert_eq!(controller.state().version, info(0, 2, 120.0));
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(-110.0, "a1"), (-100.0, "a2"), (-90.0, "a3")]))
        );
        assert_eq!((saves(&saved), notified.load(Ordering::SeqCst)), (1, 1));

        // 已在上限：偏移沒變，不存也不通知。NaN 與 0 也一樣。
        for unchanged in [0.5, 0.0, f64::NAN] {
            controller.nudge_offset(unchanged);
            assert_eq!(controller.state().version, info(0, 2, 120.0));
        }
        assert_eq!((saves(&saved), notified.load(Ordering::SeqCst)), (1, 1));

        controller.nudge_offset(-500.0);
        assert_eq!(controller.state().version, info(0, 2, -120.0));
        assert_eq!((saves(&saved), notified.load(Ordering::SeqCst)), (2, 2));

        controller.reset_offset();
        assert_eq!(controller.state().version, info(0, 2, 0.0));
        assert_eq!(controller.state().lyrics, synced(lines_a()));
        assert_eq!((saves(&saved), notified.load(Ordering::SeqCst)), (3, 3));
        assert_eq!(
            last_saved(&saved)[0]
                .offsets
                .get(URL_A)
                .copied()
                .unwrap_or(0.0),
            0.0
        );

        controller.reset_offset();
        assert_eq!((saves(&saved), notified.load(Ordering::SeqCst)), (3, 3));
    }

    #[test]
    fn a_saved_choice_is_selected_with_its_offset() {
        let fx = Fixture::new();
        let recorded = choice(
            "Song A",
            Some(200.0),
            URL_B,
            lines_b(),
            &[(URL_A, -2.0), (URL_B, 1.5)],
        );
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(song("Song A"));

        // 查詢結束前不先顯示存下的內容。
        let state = controller.state();
        assert_eq!(state.lyrics, None);
        assert_eq!(state.lyrics_url, None);
        assert_eq!(state.version, None);
        assert!(state.searching);

        fx.answer(0, two_versions());
        let state = controller.state();
        assert_eq!(state.version, info(1, 2, 1.5));
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_B));
        assert_eq!(
            state.lyrics,
            synced(timed(&[(13.5, "b1"), (23.5, "b2"), (33.5, "b3")]))
        );
        assert_eq!(saves(&saved), 0, "套用記住的選擇不寫入");

        controller.next_version();
        assert_eq!(controller.state().version, info(0, 2, -2.0));
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(12.0, "a1"), (22.0, "a2"), (32.0, "a3")]))
        );
    }

    #[test]
    fn a_saved_choice_shows_when_the_lookup_finds_nothing() {
        let fx = Fixture::new();
        let recorded = choice("Song A", Some(200.0), URL_B, lines_b(), &[(URL_B, 1.5)]);
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(song("Song A"));
        fx.answer(0, Vec::new());

        let state = controller.state();
        assert!(!state.searching);
        assert_eq!(state.version, info(0, 1, 1.5));
        assert_eq!(state.lyrics_url.as_deref(), Some(URL_B));
        assert_eq!(
            state.lyrics,
            synced(timed(&[(13.5, "b1"), (23.5, "b2"), (33.5, "b3")]))
        );

        // 只有一個版本：不能切換。
        controller.next_version();
        assert_eq!(saves(&saved), 0);

        controller.nudge_offset(0.5);
        assert_eq!(controller.state().version, info(0, 1, 2.0));
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_B,
                lines_b(),
                &[(URL_B, 2.0)]
            )]
        );
    }

    #[test]
    fn plain_missing_or_searching_lyrics_have_no_controls_and_save_nothing() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        let (notified, _sub) = count_notifications(&controller);
        let unchanged = |expected_notifications: usize| {
            let before = controller.state();
            assert_eq!(before.version, None);
            act_all(&controller);
            assert_eq!(controller.state(), before);
            assert_eq!(saves(&saved), 0);
            assert_eq!(notified.load(Ordering::SeqCst), expected_notifications);
        };

        unchanged(0); // 沒有曲目
        fx.emit(song("Song A"));
        unchanged(1); // 查詢中
        fx.respond(0, plain("words"));
        assert_eq!(controller.state().lyrics, plain("words"));
        unchanged(2); // 純文字
        fx.emit(song("Song B"));
        fx.respond(1, None);
        unchanged(4); // 查無
    }

    #[test]
    fn playing_without_touching_the_controls_saves_nothing() {
        let fx = Fixture::new();
        let recorded = choice("Song A", Some(200.0), URL_B, lines_b(), &[(URL_B, 1.5)]);
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        fx.emit(snapshot("Song A", Some("Artist"), false, None));
        controller.set_traditional_lyrics(true);
        fx.emit(lasting("Song A", Some(201.0)));
        fx.emit(song("Song B"));
        fx.answer(1, two_versions());
        controller.set_traditional_lyrics(false);
        fx.emit(None);

        assert_eq!(saves(&saved), 0);
    }

    #[test]
    fn actions_after_a_track_change_never_write_the_previous_song() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        controller.nudge_offset(0.5);
        assert_eq!(saves(&saved), 1);

        fx.emit(song("Song B"));
        assert_eq!(controller.state().version, None);
        act_all(&controller);
        assert_eq!(saves(&saved), 1);
        assert_eq!(controller.state().lyrics, None);
        assert_eq!(controller.state().lyrics_url, None);

        // 同樣網址的版本出現在新歌：上一首的偏移不跟過來。
        fx.answer(1, two_versions());
        assert_eq!(controller.state().version, info(0, 2, 0.0));
        controller.next_version();
        assert_eq!(
            last_saved(&saved),
            [
                choice("Song A", Some(200.0), URL_A, lines_a(), &[(URL_A, 0.5)]),
                choice("Song B", Some(200.0), URL_B, lines_b(), &[]),
            ]
        );

        fx.emit(None);
        act_all(&controller);
        assert_eq!(saves(&saved), 2);
        assert_eq!(controller.state().version, None);
    }

    #[test]
    fn a_choice_made_this_session_is_applied_when_the_song_comes_back() {
        let fx = Fixture::new();
        let (controller, _saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        controller.next_version();
        controller.nudge_offset(0.5);
        fx.emit(song("Song B"));
        fx.emit(song("Song A"));
        fx.answer(2, two_versions());

        assert_eq!(controller.state().version, info(1, 2, 0.5));
        assert_eq!(controller.state().lyrics_url.as_deref(), Some(URL_B));
    }

    #[test]
    fn a_duration_that_arrives_late_still_matches_the_saved_choice() {
        let fx = Fixture::new();
        let recorded = choice("Song A", Some(200.0), URL_B, lines_b(), &[(URL_B, 1.5)]);
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(lasting("Song A", None));
        fx.emit(lasting("Song A", Some(200.0)));

        // 曲長晚到會重新查詢，回答第二次查詢。
        assert_eq!(fx.resolver.requested().len(), 2);
        fx.answer(1, two_versions());

        assert_eq!(controller.state().version, info(1, 2, 1.5));
        assert_eq!(controller.state().lyrics_url.as_deref(), Some(URL_B));
        assert_eq!(saves(&saved), 0);
    }

    /// 快照的曲長在容忍內晃動不重查，紀錄一直認查詢發出時的 200：同一次播放只寫同一筆。
    #[test]
    fn saving_uses_the_lookup_length_while_the_snapshot_wanders() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(lasting("Song A", Some(200.0)));
        fx.answer(0, two_versions());

        fx.emit(lasting("Song A", Some(201.5)));
        controller.next_version();
        assert_eq!(
            last_saved(&saved),
            [choice("Song A", Some(200.0), URL_B, lines_b(), &[])]
        );

        // 和上一次晃到的 201.5 差超過 2 秒，仍是同一筆。
        fx.emit(lasting("Song A", Some(198.5)));
        controller.nudge_offset(0.5);
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_B,
                lines_b(),
                &[(URL_B, 0.5)]
            )]
        );
        assert_eq!(fx.resolver.requested().len(), 1);
    }

    #[test]
    fn matching_uses_the_lookup_length_while_the_snapshot_wanders() {
        let fx = Fixture::new();
        // 198.5 在查詢時的 200 的容忍內，不在目前的 201.5 的容忍內。
        let recorded = choice("Song A", Some(198.5), URL_B, lines_b(), &[(URL_B, 1.5)]);
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(lasting("Song A", Some(200.0)));
        fx.emit(lasting("Song A", Some(201.5)));
        fx.answer(0, two_versions());
        assert_eq!(controller.state().version, info(1, 2, 1.5));

        // 存回配到的那一筆，不另外新增。
        controller.nudge_offset(0.5);
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_B,
                lines_b(),
                &[(URL_B, 2.0)]
            )]
        );
    }

    #[test]
    fn a_dropped_length_keeps_matching_and_saving_at_the_lookup_length() {
        let fx = Fixture::new();
        let recorded = choice("Song A", Some(200.0), URL_B, lines_b(), &[(URL_B, 1.5)]);
        let (controller, saved) = fx.controller_with(book(vec![recorded]));
        fx.emit(lasting("Song A", Some(200.0)));
        fx.emit(lasting("Song A", None));
        fx.answer(0, two_versions());
        assert_eq!(controller.state().version, info(1, 2, 1.5));

        controller.nudge_offset(0.5);
        assert_eq!(
            last_saved(&saved),
            [choice(
                "Song A",
                Some(200.0),
                URL_B,
                lines_b(),
                &[(URL_B, 2.0)]
            )]
        );
        assert_eq!(fx.resolver.requested().len(), 1);
    }

    #[test]
    fn an_unusable_length_is_saved_as_unknown() {
        for unusable in [Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            let fx = Fixture::new();
            let (controller, saved) = fx.controller_with(ChoiceBook::default());
            fx.emit(lasting("Song A", unusable));
            fx.answer(0, two_versions());
            controller.next_version();
            assert_eq!(last_saved(&saved)[0].duration, None, "{unusable:?}");
        }
    }

    #[test]
    fn the_traditional_toggle_keeps_the_offset() {
        let simplified = || timed(&[(10.0, SIMPLIFIED), (20.0, "这里")]);
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, vec![version(URL_A, simplified())]);
        controller.nudge_offset(0.5);

        controller.set_traditional_lyrics(true);
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(9.5, TRADITIONAL), (19.5, "這裡")]))
        );
        assert_eq!(controller.state().version, info(0, 1, 0.5));
        assert_eq!(saves(&saved), 1, "開關不寫入紀錄");

        controller.nudge_offset(0.5);
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(9.0, TRADITIONAL), (19.0, "這裡")]))
        );
        // 存下的是提供者給的行：未偏移、未轉繁。
        assert_eq!(last_saved(&saved)[0].lines, simplified());

        controller.set_traditional_lyrics(false);
        assert_eq!(
            controller.state().lyrics,
            synced(timed(&[(9.0, SIMPLIFIED), (19.0, "这里")]))
        );
        assert_eq!(controller.state().version, info(0, 1, 1.0));
    }

    #[test]
    fn a_stored_offset_outside_the_range_is_clamped() {
        for (stored, shown) in [(500.0, 120.0), (-500.0, -120.0), (f64::NAN, 0.0)] {
            let fx = Fixture::new();
            let recorded = choice("Song A", Some(200.0), URL_A, lines_a(), &[(URL_A, stored)]);
            let (controller, _saved) = fx.controller_with(book(vec![recorded]));
            fx.emit(song("Song A"));
            fx.answer(0, two_versions());

            assert_eq!(controller.state().version, info(0, 2, shown), "{stored}");
            assert_eq!(
                controller.state().lyrics,
                synced(shift_a(shown)),
                "{stored}"
            );
        }
    }

    fn shift_a(offset: f64) -> Vec<LyricLine> {
        timed(&[
            (10.0 - offset, "a1"),
            (20.0 - offset, "a2"),
            (30.0 - offset, "a3"),
        ])
    }

    #[test]
    fn every_action_notifies_once_with_the_new_state() {
        let fx = Fixture::new();
        let (controller, _saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        let seen: Arc<Mutex<Vec<Option<VersionInfo>>>> = Arc::default();
        let log = seen.clone();
        let _sub = controller.subscribe(Box::new(move |s| lock(&log).push(s.version)));

        controller.next_version();
        controller.nudge_offset(0.5);
        controller.reset_offset();

        assert_eq!(
            *lock(&seen),
            [info(1, 2, 0.0), info(1, 2, 0.5), info(1, 2, 0.0)]
        );
    }

    #[test]
    fn the_saver_is_called_without_the_controller_lock_held() {
        let fx = Fixture::new();
        let slot: Arc<OnceLock<Weak<MusicController>>> = Arc::default();
        let unlocked: Arc<Mutex<Vec<bool>>> = Arc::default();
        let (c, log) = (slot.clone(), unlocked.clone());
        let controller = MusicController::new_with_choices(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(fx.spawner.clone()),
            ChoiceBook::default(),
            Arc::new(move |_| {
                let controller = c.get().unwrap().upgrade().unwrap();
                // 用 try_lock：持鎖呼叫時是斷言失敗而不是死結。
                let free = controller.core.try_lock().is_ok();
                lock(&log).push(free);
            }),
        );
        slot.set(Arc::downgrade(&controller)).unwrap();
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());

        controller.next_version();
        controller.nudge_offset(0.5);
        controller.reset_offset();

        assert_eq!(*lock(&unlocked), [true, true, true]);
    }

    /// 訂閱者在通知裡又做了一個動作：較新的紀錄必須最後寫出，所以存檔要在通知之前。
    #[test]
    fn a_subscriber_acting_again_leaves_the_newest_choice_saved_last() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        let (c, acted) = (Arc::downgrade(&controller), AtomicBool::new(false));
        let _sub = controller.subscribe(Box::new(move |_| {
            if !acted.swap(true, Ordering::SeqCst) {
                c.upgrade().unwrap().nudge_offset(0.5);
            }
        }));

        controller.next_version();

        assert_eq!(saves(&saved), 2);
        assert_eq!(last_saved(&saved)[0].offsets.get(URL_B), Some(&0.5));
    }

    // ---- 曲長改變時重查 ----

    #[test]
    fn a_length_that_arrives_late_restarts_the_lookup_once() {
        let fx = Fixture::new();
        let (controller, _saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(lasting("Song A", None));
        fx.emit(lasting("Song A", Some(200.0)));
        assert_eq!(fx.requested_durations(), [None, Some(200.0)]);
        assert_eq!(fx.resolver.dropped.load(Ordering::SeqCst), 1);
        assert!(controller.state().searching);

        fx.emit(snapshot("Song A", Some("Artist"), false, None));
        fx.emit(lasting("Song A", Some(201.0)));
        assert_eq!(fx.resolver.requested().len(), 2);
    }

    #[test]
    fn a_length_that_arrives_after_the_answer_clears_the_lyrics_and_searches_again() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(lasting("Song A", None));
        fx.answer(0, two_versions());
        assert_eq!(controller.state().version, info(0, 2, 0.0));

        fx.emit(lasting("Song A", Some(200.0)));

        assert_eq!(fx.requested_durations(), [None, Some(200.0)]);
        let state = controller.state();
        assert_eq!(state.lyrics, None);
        assert_eq!(state.lyrics_url, None);
        assert_eq!(state.version, None);
        assert!(state.searching);
        act_all(&controller);
        assert_eq!(saves(&saved), 0);
    }

    #[test]
    fn a_length_change_over_two_seconds_restarts_and_clears_lyrics() {
        for changed in [202.1, 197.9, 260.0] {
            let fx = Fixture::new();
            let (controller, saved) = fx.controller_with(ChoiceBook::default());
            fx.emit(song("Song A"));
            fx.answer(0, two_versions());
            controller.nudge_offset(0.5);
            let seen: Arc<Mutex<Vec<MusicState>>> = Arc::default();
            let log = seen.clone();
            let _sub = controller.subscribe(Box::new(move |s| lock(&log).push(s.clone())));

            fx.emit(lasting("Song A", Some(changed)));

            assert_eq!(
                fx.requested_durations(),
                [Some(200.0), Some(changed)],
                "{changed}"
            );
            let state = controller.state();
            assert_eq!(state.snapshot.as_ref().unwrap().duration, Some(changed));
            assert_eq!(state.lyrics, None);
            assert_eq!(state.lyrics_url, None);
            assert_eq!(state.version, None);
            assert!(state.searching);
            assert_eq!(*lock(&seen), [state], "清空只通知一次");
            act_all(&controller);
            assert_eq!(saves(&saved), 1);
        }
    }

    #[test]
    fn a_small_change_or_a_missing_length_keeps_the_lookup() {
        let fx = Fixture::new();
        let (controller, _saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        controller.nudge_offset(0.5);
        let shown = controller.state().lyrics;
        for length in [
            Some(202.0),
            Some(198.0),
            None,
            Some(200.0),
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(201.0),
        ] {
            fx.emit(lasting("Song A", length));
            assert_eq!(fx.resolver.requested().len(), 1, "{length:?}");
            assert_eq!(controller.state().lyrics, shown, "{length:?}");
            assert_eq!(controller.state().version, info(0, 2, 0.5), "{length:?}");
            assert!(!controller.state().searching, "{length:?}");
        }

        // 沒有曲長的快照不會讓控制器忘記查詢時的曲長。
        fx.emit(lasting("Song A", None));
        fx.emit(lasting("Song A", Some(205.0)));
        assert_eq!(fx.requested_durations(), [Some(200.0), Some(205.0)]);
    }

    /// 寫起來剛好差 2 秒的小數（`256.1 - 254.1 > 2.0`）不受浮點誤差影響。
    #[test]
    fn exactly_two_seconds_in_decimals_keeps_the_lookup() {
        let fx = Fixture::new();
        let _controller = fx.controller(None);
        fx.emit(lasting("Song A", Some(254.1)));
        fx.emit(lasting("Song A", Some(256.1)));
        assert_eq!(fx.resolver.requested().len(), 1);
    }

    /// 和查詢發出時的曲長比，不是和上一個快照比。
    #[test]
    fn slow_drift_cannot_creep_past_the_limit() {
        let fx = Fixture::new();
        let _controller = fx.controller(None);
        for length in [200.0, 201.5, 203.0] {
            fx.emit(lasting("Song A", Some(length)));
        }
        assert_eq!(fx.requested_durations(), [Some(200.0), Some(203.0)]);

        // 重查之後以新的曲長為準。
        fx.emit(lasting("Song A", Some(204.5)));
        fx.emit(lasting("Song A", Some(201.5)));
        assert_eq!(fx.resolver.requested().len(), 2);
    }

    #[test]
    fn a_stale_lookup_of_the_same_song_writes_nothing() {
        let fx = Fixture::new();
        let controller = MusicController::new(
            fx.monitor.clone(),
            fx.resolver.clone(),
            None,
            Arc::new(DetachingSpawner(fx.spawner.clone())),
        );
        fx.emit(lasting("Song A", None));
        fx.emit(lasting("Song A", Some(200.0)));
        assert_eq!(fx.resolver.requested().len(), 2);

        fx.answer(0, vec![version(URL_C, lines_c())]);
        let state = controller.state();
        assert_eq!(state.lyrics, None);
        assert_eq!(state.lyrics_url, None);
        assert_eq!(state.version, None);
        assert!(state.searching);

        fx.answer(1, two_versions());
        fx.answer(0, vec![version(URL_C, lines_c())]);
        assert_eq!(controller.state().lyrics_url.as_deref(), Some(URL_A));
        assert_eq!(controller.state().version, info(0, 2, 0.0));
    }

    #[test]
    fn another_length_of_the_same_title_neither_reuses_nor_overwrites_the_previous_versions() {
        let fx = Fixture::new();
        let (controller, saved) = fx.controller_with(ChoiceBook::default());
        fx.emit(song("Song A"));
        fx.answer(0, two_versions());
        controller.next_version();
        controller.nudge_offset(0.5);
        let first = choice("Song A", Some(200.0), URL_B, lines_b(), &[(URL_B, 0.5)]);
        assert_eq!(last_saved(&saved), std::slice::from_ref(&first));

        fx.emit(lasting("Song A", Some(260.0)));
        act_all(&controller);
        assert_eq!(saves(&saved), 2);
        // 同樣網址的版本也不帶 200 秒那首的選擇與偏移。
        fx.answer(1, two_versions());
        assert_eq!(controller.state().version, info(0, 2, 0.0));
        controller.next_version();
        assert_eq!(controller.state().version, info(1, 2, 0.0));
        assert_eq!(
            last_saved(&saved),
            [first, choice("Song A", Some(260.0), URL_B, lines_b(), &[]),]
        );
    }
}
