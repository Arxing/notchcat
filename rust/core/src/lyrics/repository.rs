//! 跨來源查歌詞、快取查到的結果（移植自 Swift `LyricsRepository.swift`）。
//!
//! 排序規則是重點：帶時間軸的結果勝過來源優先序，同類之中曲長吻合的勝過放寬的。高順位來源可能只有純文字，
//! 低順位來源卻有完整的時間軸版本；時間軸歌詞才是這個模組的目的，所以「先到先贏」是錯的規則。
//!
//! 這是單趟走訪，不是兩趟：純文字結果邊走邊留著，退回它不必多打任何網路請求。
//! 每個來源最多問一次，且各有時間上限；逾時或錯誤都算失敗。
//!
//! 非同步模型見 ADR-0001：同一首歌的併發呼叫以 `Coalescer` 共用一次查詢；快取寫入位於被合併的
//! future 內部的結尾，所以被放棄（所有呼叫者都 drop）的查詢根本跑不到寫入那一行，不會被記成「查無」。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::identity::TrackIdentity;
use super::provider::LyricsProvider;
use super::LyricsHit;
use crate::runtime::{lock, BoxFuture, Clock};
use crate::timing::{with_timeout, Coalescer};

/// 快取與逾時的設定。預設值即正式版數值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepositoryConfig {
    /// 「每個來源都答了、而且都沒有」要記多久。
    pub negative_ttl: Duration,
    /// 至少一個來源失敗或逾時、且沒找到同步歌詞時，結果要記多久。涵蓋「沒有」與「純文字保底」，
    /// 讓稍後重試仍有機會找到同步歌詞。遠短於 `negative_ttl`，因為離線與這首歌無關。
    pub failure_ttl: Duration,
    /// 每個來源的上限；超過即算失敗。可以是 `Duration::MAX`（永不逾時）。
    pub provider_timeout: Duration,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            negative_ttl: Duration::from_secs(86_400),
            failure_ttl: Duration::from_secs(120),
            provider_timeout: Duration::from_secs(5),
        }
    }
}

/// 依曲目解析歌詞。回傳的 future 被 drop 即代表這個呼叫者放棄。
pub trait LyricsResolving: Send + Sync {
    fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<LyricsHit>>;
}

/// `until`（Unix epoch 秒）為 `None` 表示永不過期。
#[derive(Clone)]
enum CacheEntry {
    Found { hit: LyricsHit, until: Option<f64> },
    Missing { until: f64 },
}

/// 跨來源解析歌詞並快取結果（見模組說明）。
pub struct LyricsRepository {
    inner: Arc<Inner>,
    flights: Coalescer<String, Option<LyricsHit>>,
}

/// 被合併的查詢 future 需要 `'static`，所以它用到的狀態放在 `Arc` 裡。
struct Inner {
    providers: Vec<Arc<dyn LyricsProvider>>,
    clock: Arc<dyn Clock>,
    cfg: RepositoryConfig,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

/// 單一來源的結果。
enum FetchOutcome {
    Hit(LyricsHit),
    /// 來源答了，而且沒有。
    Empty,
    /// 錯誤或逾時。
    Failed,
}

impl LyricsRepository {
    /// `providers` 依優先序排列；在最前面插入新來源不需要改其他任何程式碼。
    pub fn new(
        providers: Vec<Arc<dyn LyricsProvider>>,
        clock: Arc<dyn Clock>,
        cfg: RepositoryConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                providers,
                clock,
                cfg,
                cache: Mutex::new(HashMap::new()),
            }),
            flights: Coalescer::new(),
        }
    }

    /// 正在等待 `track` 那次查詢的呼叫者數；沒有進行中的查詢時為 0（測試用）。
    pub fn in_flight_waiters(&self, track: &TrackIdentity) -> usize {
        self.flights.waiters(&track.cache_key())
    }
}

impl LyricsResolving for LyricsRepository {
    fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Option<LyricsHit>> {
        let key = track.cache_key();
        if let Some(hit) = self.inner.cached(&key) {
            return Box::pin(std::future::ready(hit));
        }
        // 同一首歌的呼叫者共用一次查詢，所以來回快速切歌不會把同一個來源問兩次。
        // 呼叫者 drop 自己的 future 即離開；最後一個離開時查詢被 drop（放棄）。
        let inner = self.inner.clone();
        let run_key = key.clone();
        Box::pin(self.flights.run(key, move || {
            Box::pin(async move { inner.resolve(track, run_key).await })
        }))
    }
}

impl Inner {
    /// 未過期的快取結果：`Some(Some(_))` 命中歌詞、`Some(None)` 命中「查無」、`None` 沒有可用的快取。
    fn cached(&self, key: &str) -> Option<Option<LyricsHit>> {
        let now = self.clock.now();
        match lock(&self.cache).get(key)? {
            CacheEntry::Found { hit, until } if until.is_none_or(|u| now < u) => {
                Some(Some(hit.clone()))
            }
            CacheEntry::Missing { until } if now < *until => Some(None),
            _ => None,
        }
    }

    async fn resolve(&self, track: TrackIdentity, key: String) -> Option<LyricsHit> {
        // 多執行緒競態：呼叫者查快取沒命中之後、本查詢第一次被輪詢之前，另一條執行緒的查詢可能
        // 已經寫好快取並移除了它的表項（快取一律先寫、表項後移除），於是這裡成了第二輪查詢。
        // 開頭重查一次快取即可擋下，不必再問一輪來源。
        if let Some(hit) = self.cached(&key) {
            return hit;
        }
        let mut best: Option<LyricsHit> = None;
        let mut any_failed = false;

        for provider in &self.providers {
            match self.fetch(provider.as_ref(), &track).await {
                FetchOutcome::Failed => any_failed = true,
                FetchOutcome::Empty => {}
                FetchOutcome::Hit(result) => {
                    // 來源答了，但時間軸不可信：等同沒有——不是失敗，也不當保底，網址也一併丟掉。
                    if !result.lyrics.is_plausible(track.duration) {
                        continue;
                    }
                    // 同序保留較前面來源的那筆（連同它自己的網址），由優先序決定。
                    LyricsHit::keep_better(&mut best, result);
                    if best.as_ref().is_some_and(LyricsHit::is_best_possible) {
                        break;
                    }
                }
            }
        }

        // 走到這裡代表查詢沒被放棄（被放棄的 future 已被 drop，不會執行以下任何一行）。
        let now = self.clock.now();
        let failure_expiry = now + self.cfg.failure_ttl.as_secs_f64();
        let (entry, result) = if let Some(hit) = best {
            // 純文字或曲長放寬的結果，在有來源掛掉時只短暫記住，之後可能換成更好的。
            let until = (any_failed && !hit.is_best_possible()).then_some(failure_expiry);
            (
                CacheEntry::Found {
                    hit: hit.clone(),
                    until,
                },
                Some(hit),
            )
        } else {
            let until = if any_failed {
                failure_expiry
            } else {
                now + self.cfg.negative_ttl.as_secs_f64()
            };
            (CacheEntry::Missing { until }, None)
        };
        lock(&self.cache).insert(key, entry);
        result
    }

    /// 讓來源與逾時賽跑；輸家直接 drop，所以不理會取消的來源也卡不住走訪。
    async fn fetch(&self, provider: &dyn LyricsProvider, track: &TrackIdentity) -> FetchOutcome {
        let attempt = with_timeout(
            self.clock.as_ref(),
            self.cfg.provider_timeout,
            provider.fetch(track),
        );
        match attempt.await {
            Ok(Ok(Some(hit))) => FetchOutcome::Hit(hit),
            Ok(Ok(None)) => FetchOutcome::Empty,
            Ok(Err(_)) | Err(_) => FetchOutcome::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    //! 移植 Swift `LyricsRepositoryTests`（30 案，名稱為 Swift 測試名的 snake_case），
    //! 之後是任務簡報點名的行為測試（以正式版預設值 86400／120／5 秒驅動）。
    //!
    //! Swift 的真實時間（`Task.sleep`、`DispatchQueue.asyncAfter`、`waitUntil`）一律換成
    //! `FakeClock` + `LocalSpawner`：`run_until_stalled` 之後的狀態就是「所有能前進的都前進了」。

    use super::*;
    use crate::lyrics::provider::ProviderError;
    use crate::lyrics::{LyricLine, Lyrics, LyricsHit};
    use crate::runtime::{Spawner, TaskHandle};
    use crate::testing::{FakeClock, LocalSpawner};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::task::{Poll, Waker};

    // ------------------------------------------------------------ 測試替身

    /// 對應 Swift `FakeProvider`：固定的結果或錯誤，可選的延遲（以 `FakeClock` 計時），
    /// 可在中途改答案（例如來源恢復）。延遲之後才讀取答案，與 Swift 相同。
    struct FakeProvider {
        id: String,
        response: Mutex<(Option<LyricsHit>, bool)>,
        delay: Duration,
        clock: Arc<FakeClock>,
        calls: AtomicUsize,
    }

    /// 測試用命中：網址固定。
    fn hit(lyrics: Lyrics) -> LyricsHit {
        LyricsHit::new(lyrics, "https://test.example/0")
    }

    /// 帶指定來源網址的命中。
    fn hit_at(lyrics: Lyrics, url: &str) -> LyricsHit {
        LyricsHit::new(lyrics, url)
    }

    /// 方便寫法：命中的網址由來源 id 決定（`https://test.example/<id>`），
    /// 讓只關心歌詞的測試不必每個都寫網址。
    fn hit_for(id: &str, lyrics: Option<Lyrics>) -> Option<LyricsHit> {
        lyrics.map(|l| hit_at(l, &format!("https://test.example/{id}")))
    }

    impl FakeProvider {
        fn new(
            id: &str,
            clock: &Arc<FakeClock>,
            result: Option<LyricsHit>,
            fails: bool,
            delay: Duration,
        ) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                response: Mutex::new((result, fails)),
                delay,
                clock: clock.clone(),
                calls: AtomicUsize::new(0),
            })
        }

        fn result(id: &str, clock: &Arc<FakeClock>, result: Option<Lyrics>) -> Arc<Self> {
            Self::new(id, clock, hit_for(id, result), false, Duration::ZERO)
        }

        /// 明確指定整個命中（含網址）。
        fn result_hit(id: &str, clock: &Arc<FakeClock>, result: Option<LyricsHit>) -> Arc<Self> {
            Self::new(id, clock, result, false, Duration::ZERO)
        }

        fn failing(id: &str, clock: &Arc<FakeClock>) -> Arc<Self> {
            Self::new(id, clock, None, true, Duration::ZERO)
        }

        fn slow(
            id: &str,
            clock: &Arc<FakeClock>,
            result: Option<Lyrics>,
            delay: Duration,
        ) -> Arc<Self> {
            Self::new(id, clock, hit_for(id, result), false, delay)
        }

        /// 永不回應的來源。
        fn hanging(id: &str, clock: &Arc<FakeClock>, result: Option<Lyrics>) -> Arc<Self> {
            Self::new(id, clock, hit_for(id, result), false, Duration::MAX)
        }

        /// 改變之後呼叫的答案。
        fn respond(&self, result: Option<Lyrics>, fails: bool) {
            *self.response.lock().unwrap() = (hit_for(&self.id, result), fails);
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl LyricsProvider for FakeProvider {
        fn id(&self) -> &str {
            &self.id
        }

        fn fetch<'a>(
            &'a self,
            _track: &'a TrackIdentity,
        ) -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if self.delay > Duration::ZERO {
                    self.clock.sleep(self.delay).await;
                }
                let (result, fails) = self.response.lock().unwrap().clone();
                if fails {
                    return Err(ProviderError::Transport("provider failure".into()));
                }
                Ok(result)
            })
        }
    }

    /// 對應 Swift `StubbornProvider`：不理會取消，等滿 `delay` 才回答。
    /// Rust 的取消就是 drop；這裡在 drop 時立旗，證明逾時後它被丟掉而不是被等完。
    struct StubbornProvider {
        result: Option<LyricsHit>,
        delay: Duration,
        clock: Arc<FakeClock>,
        calls: AtomicUsize,
        dropped: Arc<AtomicBool>,
    }

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl LyricsProvider for StubbornProvider {
        fn id(&self) -> &str {
            "stubborn"
        }

        fn fetch<'a>(
            &'a self,
            _track: &'a TrackIdentity,
        ) -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let flag = DropFlag(self.dropped.clone());
            Box::pin(async move {
                let _flag = flag;
                self.clock.sleep(self.delay).await;
                Ok(self.result.clone())
            })
        }
    }

    /// 對應 Swift `GatedProvider`：`open()` 之前不回答。
    struct GatedProvider {
        result: Option<LyricsHit>,
        calls: AtomicUsize,
        gate: Mutex<(bool, Vec<Waker>)>,
    }

    impl GatedProvider {
        fn new(result: Option<Lyrics>) -> Arc<Self> {
            Arc::new(Self {
                result: result.map(hit),
                calls: AtomicUsize::new(0),
                gate: Mutex::new((false, Vec::new())),
            })
        }

        fn open(&self) {
            let wakers = {
                let mut gate = self.gate.lock().unwrap();
                gate.0 = true;
                std::mem::take(&mut gate.1)
            };
            wakers.into_iter().for_each(Waker::wake);
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl LyricsProvider for GatedProvider {
        fn id(&self) -> &str {
            "gated"
        }

        fn fetch<'a>(
            &'a self,
            _track: &'a TrackIdentity,
        ) -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                std::future::poll_fn(|cx| {
                    let mut gate = self.gate.lock().unwrap();
                    if gate.0 {
                        Poll::Ready(())
                    } else {
                        gate.1.push(cx.waker().clone());
                        Poll::Pending
                    }
                })
                .await;
                Ok(self.result.clone())
            })
        }
    }

    // ------------------------------------------------------------ 測試骨架

    /// 一次在背景跑的查詢：結果寫進 `slot`；drop `task` 即取消該呼叫者。
    struct Lookup {
        slot: Arc<Mutex<Option<Option<LyricsHit>>>>,
        task: TaskHandle,
    }

    impl Lookup {
        /// `None`：還沒結束（或已被取消）；`Some(r)`：查詢回傳了 `r`（只看歌詞）。
        fn outcome(&self) -> Option<Option<Lyrics>> {
            let done = self.slot.lock().unwrap().clone()?;
            Some(done.map(|h| h.lyrics))
        }

        fn finished(&self) -> Option<Lyrics> {
            self.outcome().expect("the lookup should have finished")
        }

        fn finished_hit(&self) -> Option<LyricsHit> {
            let done = self.slot.lock().unwrap().clone();
            done.expect("the lookup should have finished")
        }
    }

    struct Harness {
        clock: Arc<FakeClock>,
        spawner: LocalSpawner,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                clock: Arc::new(FakeClock::new(0.0)),
                spawner: LocalSpawner::new(),
            }
        }

        fn repo_with(
            &self,
            providers: Vec<Arc<dyn LyricsProvider>>,
            cfg: RepositoryConfig,
        ) -> Arc<LyricsRepository> {
            Arc::new(LyricsRepository::new(providers, self.clock.clone(), cfg))
        }

        /// Swift 測試沒傳參數時的預設值（與正式版相同）。
        fn repo(&self, providers: Vec<Arc<dyn LyricsProvider>>) -> Arc<LyricsRepository> {
            self.repo_with(providers, RepositoryConfig::default())
        }

        /// 對應 Swift 的 `makeRepo`：negativeTTL 3600、failureTTL 60。
        fn make_repo(
            &self,
            providers: Vec<Arc<dyn LyricsProvider>>,
            timeout: Duration,
        ) -> Arc<LyricsRepository> {
            self.repo_with(
                providers,
                RepositoryConfig {
                    negative_ttl: Duration::from_secs(3_600),
                    failure_ttl: Duration::from_secs(60),
                    provider_timeout: timeout,
                },
            )
        }

        /// 開始一次查詢（尚未執行）。
        fn start(&self, repo: &Arc<LyricsRepository>, track: TrackIdentity) -> Lookup {
            let slot = Arc::new(Mutex::new(None));
            let (r, s) = (repo.clone(), slot.clone());
            let task = self.spawner.spawn(Box::pin(async move {
                let result = r.lyrics(track).await;
                *s.lock().unwrap() = Some(result);
            }));
            Lookup { slot, task }
        }

        fn run(&self) {
            self.spawner.run_until_stalled();
        }

        /// 推進時間並讓所有工作跑到停滯。
        fn advance(&self, d: Duration) {
            self.clock.advance(d);
            self.run();
        }

        /// 查詢並在不推進時間的情況下跑完（來源都立即回答時）。
        fn ask(&self, repo: &Arc<LyricsRepository>, track: TrackIdentity) -> Option<Lyrics> {
            self.ask_hit(repo, track).map(|h| h.lyrics)
        }

        /// 同 `ask`，但連來源網址一起回傳。
        fn ask_hit(&self, repo: &Arc<LyricsRepository>, track: TrackIdentity) -> Option<LyricsHit> {
            let lookup = self.start(repo, track);
            self.run();
            lookup.finished_hit()
        }
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn millis(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn identity(title: &str, duration: f64) -> TrackIdentity {
        TrackIdentity {
            title: title.into(),
            artist: "A".into(),
            album: None,
            duration: Some(duration),
        }
    }

    fn track() -> TrackIdentity {
        identity("T", 200.0)
    }

    fn line(time: f64, text: &str) -> LyricLine {
        LyricLine {
            time,
            text: text.into(),
        }
    }

    fn synced() -> Lyrics {
        Lyrics::Synced(vec![line(0.0, "line"), line(10.0, "next")])
    }

    /// 只有一行時間行通常是「純音樂」佔位，可信度把關會擋下。
    fn implausible() -> Lyrics {
        Lyrics::Synced(vec![line(0.0, "純音樂，請欣賞")])
    }

    fn plain(text: &str) -> Lyrics {
        Lyrics::Plain(text.into())
    }

    fn dynp<P: LyricsProvider + 'static>(p: &Arc<P>) -> Arc<dyn LyricsProvider> {
        p.clone()
    }

    // ------------------------------------------------------------ 移植的 30 案

    /// 整個歌詞設計的基礎：帶時間軸的結果勝過來源優先序，較前來源的純文字不得停止走訪。
    #[test]
    fn synced_result_beats_higher_priority_plain() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(plain("words")));
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        assert_eq!(h.ask(&repo, track()), Some(synced()));
    }

    #[test]
    fn each_provider_is_queried_at_most_once() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(plain("words")));
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        h.ask(&repo, track());

        assert_eq!(first.call_count(), 1);
        assert_eq!(second.call_count(), 1);
    }

    #[test]
    fn stops_at_first_synced_result() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(synced()));
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        h.ask(&repo, track());

        assert_eq!(first.call_count(), 1);
        assert_eq!(
            second.call_count(),
            0,
            "No source after the first synced hit should be asked"
        );
    }

    #[test]
    fn falls_back_to_plain_keeping_earlier_provider() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(plain("first words")));
        let second = FakeProvider::result("second", &h.clock, Some(plain("second words")));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        let result = h.ask(&repo, track());

        assert_eq!(result, Some(plain("first words")));
        assert_eq!(second.call_count(), 1, "A plain hit must not stop the walk");
    }

    #[test]
    fn failing_provider_does_not_block_later_ones() {
        let h = Harness::new();
        let first = FakeProvider::failing("first", &h.clock);
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        let result = h.ask(&repo, track());

        assert_eq!(result, Some(synced()));
        assert_eq!(first.call_count(), 1);
    }

    #[test]
    fn returns_nil_when_nobody_has_anything() {
        let h = Harness::new();
        let only = FakeProvider::result("only", &h.clock, None);
        let repo = h.repo(vec![dynp(&only)]);

        assert_eq!(h.ask(&repo, track()), None);
    }

    #[test]
    fn positive_result_is_cached() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        let first = h.ask(&repo, track());
        let second = h.ask(&repo, track());

        assert_eq!(first, Some(synced()));
        assert_eq!(second, Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    #[test]
    fn plain_fallback_is_cached() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(plain("words")));
        let repo = h.repo(vec![dynp(&provider)]);

        h.ask(&repo, track());
        let second = h.ask(&repo, track());

        assert_eq!(second, Some(plain("words")));
        assert_eq!(provider.call_count(), 1);
    }

    #[test]
    fn negative_result_is_cached_within_ttl() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, None);
        let repo = h.repo_with(
            vec![dynp(&provider)],
            RepositoryConfig {
                negative_ttl: secs(86_400),
                ..RepositoryConfig::default()
            },
        );

        h.ask(&repo, track());
        let second = h.ask(&repo, track());

        assert_eq!(second, None);
        assert_eq!(
            provider.call_count(),
            1,
            "A miss is cached so switching back does not re-query every source"
        );
    }

    #[test]
    fn negative_cache_expires() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, None);
        let repo = h.repo_with(
            vec![dynp(&provider)],
            RepositoryConfig {
                negative_ttl: secs(60),
                ..RepositoryConfig::default()
            },
        );

        h.ask(&repo, track());
        h.advance(secs(59));
        h.ask(&repo, track());
        assert_eq!(provider.call_count(), 1, "Still inside the TTL");

        h.advance(secs(2));
        h.ask(&repo, track());
        assert_eq!(
            provider.call_count(),
            2,
            "Past the TTL the sources are asked again"
        );
    }

    /// 快取鍵來自正規化後的識別，所以同一首歌帶 feat. 後綴、長度略有不同時不得重查。
    #[test]
    fn cache_is_keyed_on_normalized_identity() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        h.ask(&repo, track());
        let result = h.ask(&repo, identity("T (feat. X)", 203.0));

        assert_eq!(result, Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    #[test]
    fn provider_timeout_counts_as_miss() {
        let h = Harness::new();
        let slow = FakeProvider::slow("slow", &h.clock, Some(synced()), secs(5));
        let second = FakeProvider::result("second", &h.clock, Some(plain("words")));
        let repo = h.repo_with(
            vec![dynp(&slow), dynp(&second)],
            RepositoryConfig {
                provider_timeout: millis(50),
                ..RepositoryConfig::default()
            },
        );

        let lookup = h.start(&repo, track());
        h.run();
        assert_eq!(lookup.outcome(), None);
        h.advance(millis(50));

        assert_eq!(lookup.finished(), Some(plain("words")));
        assert_eq!(slow.call_count(), 1);
        assert_eq!(
            second.call_count(),
            1,
            "A timed-out source must not block later ones"
        );
    }

    #[test]
    fn fast_provider_is_not_cut_off_by_timeout() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), millis(10));
        let repo = h.repo_with(
            vec![dynp(&provider)],
            RepositoryConfig {
                provider_timeout: secs(2),
                ..RepositoryConfig::default()
            },
        );

        let lookup = h.start(&repo, track());
        h.run();
        h.advance(millis(10));

        assert_eq!(lookup.finished(), Some(synced()));
    }

    #[test]
    fn concurrent_requests_for_same_track_query_providers_once() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), millis(100));
        let repo = h.repo(vec![dynp(&provider)]);

        let a = h.start(&repo, track());
        let b = h.start(&repo, identity("T (feat. X)", 201.0));
        h.run();
        h.advance(millis(100));

        assert_eq!(a.finished(), Some(synced()));
        assert_eq!(b.finished(), Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    /// 取消呼叫者即放棄查詢；被放棄的查詢不得被記成「這首歌沒有歌詞」。
    #[test]
    fn cancelled_lookup_is_not_cached_as_missing() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), millis(200));
        let repo = h.repo(vec![dynp(&provider)]);

        let cancelled = h.start(&repo, track());
        h.run();
        assert_eq!(provider.call_count(), 1);
        let Lookup { slot, task } = cancelled;
        drop(task);
        h.run();
        assert_eq!(
            *slot.lock().unwrap(),
            None,
            "the abandoned caller gets nothing"
        );
        assert_eq!(repo.in_flight_waiters(&track()), 0);
        assert_eq!(
            h.clock.pending_sleeps(),
            0,
            "the abandoned lookup was dropped"
        );

        let retry = h.start(&repo, track());
        h.run();
        h.advance(millis(200));

        assert_eq!(retry.finished(), Some(synced()));
        assert_eq!(
            provider.call_count(),
            2,
            "The retry must query again rather than hit a cached miss"
        );
    }

    #[test]
    fn cancelled_caller_does_not_cancel_lookup_shared_with_another_caller() {
        let h = Harness::new();
        let provider = GatedProvider::new(Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        let survivor = h.start(&repo, track());
        h.run();
        assert_eq!(
            provider.call_count(),
            1,
            "the first caller reached the provider"
        );
        let joiner = h.start(&repo, track());
        h.run();
        assert_eq!(
            repo.in_flight_waiters(&track()),
            2,
            "the second caller joined"
        );

        drop(joiner.task);
        h.run();
        assert_eq!(
            repo.in_flight_waiters(&track()),
            1,
            "the cancelled caller left"
        );
        provider.open();
        h.run();

        assert_eq!(survivor.finished(), Some(synced()));
        assert_eq!(*joiner.slot.lock().unwrap(), None);
        assert_eq!(provider.call_count(), 1);
    }

    // ---- 不理會取消的來源

    #[test]
    fn provider_ignoring_cancellation_does_not_stall_the_walk() {
        let h = Harness::new();
        let stubborn = Arc::new(StubbornProvider {
            result: Some(hit(synced())),
            delay: secs(1),
            clock: h.clock.clone(),
            calls: AtomicUsize::new(0),
            dropped: Arc::new(AtomicBool::new(false)),
        });
        let second = FakeProvider::result("second", &h.clock, Some(plain("words")));
        let repo = h.repo_with(
            vec![dynp(&stubborn), dynp(&second)],
            RepositoryConfig {
                provider_timeout: millis(50),
                ..RepositoryConfig::default()
            },
        );

        let lookup = h.start(&repo, track());
        h.run();
        h.advance(millis(50));

        // Swift 量牆上時間 < 0.5 秒；這裡時間只走了 50 毫秒（遠早於來源的 1 秒）結果就出來了。
        assert_eq!(
            lookup.finished(),
            Some(plain("words")),
            "The walk must move on at the timeout, not when the provider finishes"
        );
        assert_eq!(stubborn.calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.call_count(), 1);
        assert!(
            stubborn.dropped.load(Ordering::SeqCst),
            "the loser was dropped"
        );
    }

    #[test]
    fn infinite_timeout_does_not_trap() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), millis(10));
        let repo = h.repo_with(
            vec![dynp(&provider)],
            RepositoryConfig {
                provider_timeout: Duration::MAX,
                ..RepositoryConfig::default()
            },
        );

        let lookup = h.start(&repo, track());
        h.run();
        h.advance(millis(10));

        assert_eq!(lookup.finished(), Some(synced()));
    }

    // ---- 失敗 vs. 沒有

    #[test]
    fn clean_miss_is_remembered_for_negative_ttl_not_failure_ttl() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, None);
        let repo = h.make_repo(vec![dynp(&provider)], secs(5));

        h.ask(&repo, track());
        h.advance(secs(3_599));
        h.ask(&repo, track());
        assert_eq!(provider.call_count(), 1);

        h.advance(secs(2));
        h.ask(&repo, track());
        assert_eq!(provider.call_count(), 2);
    }

    #[test]
    fn miss_with_a_failing_provider_is_remembered_only_for_failure_ttl() {
        let h = Harness::new();
        let provider = FakeProvider::failing("p", &h.clock);
        let repo = h.make_repo(vec![dynp(&provider)], secs(5));

        assert_eq!(h.ask(&repo, track()), None);
        h.advance(secs(59));
        h.ask(&repo, track());
        assert_eq!(
            provider.call_count(),
            1,
            "Inside failureTTL the failure is still remembered"
        );

        h.advance(secs(2));
        h.ask(&repo, track());
        assert_eq!(
            provider.call_count(),
            2,
            "Past failureTTL the source is asked again, long before negativeTTL"
        );
    }

    #[test]
    fn mixed_failed_and_clean_miss_counts_as_failed() {
        let h = Harness::new();
        let failing = FakeProvider::failing("failing", &h.clock);
        let empty = FakeProvider::result("empty", &h.clock, None);
        let repo = h.make_repo(vec![dynp(&failing), dynp(&empty)], secs(5));

        h.ask(&repo, track());
        h.advance(secs(61));
        h.ask(&repo, track());

        assert_eq!(failing.call_count(), 2);
        assert_eq!(empty.call_count(), 2);
    }

    #[test]
    fn timeout_counts_as_failure_for_caching() {
        let h = Harness::new();
        let slow = FakeProvider::slow("slow", &h.clock, Some(synced()), secs(5));
        let repo = h.make_repo(vec![dynp(&slow)], millis(50));

        let first = h.start(&repo, track());
        h.run();
        h.advance(millis(50));
        assert_eq!(first.finished(), None);
        h.advance(secs(61));
        let second = h.start(&repo, track());
        h.run();
        h.advance(millis(50));

        assert_eq!(second.finished(), None);
        assert_eq!(slow.call_count(), 2);
    }

    #[test]
    fn plain_fallback_found_while_a_provider_failed_is_retried_after_failure_ttl() {
        let h = Harness::new();
        let flaky = FakeProvider::failing("flaky", &h.clock);
        let plain_source = FakeProvider::result("plain", &h.clock, Some(plain("words")));
        let repo = h.make_repo(vec![dynp(&flaky), dynp(&plain_source)], secs(5));

        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
        h.advance(secs(59));
        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
        assert_eq!(
            flaky.call_count(),
            1,
            "Inside failureTTL the plain fallback is served from cache"
        );

        flaky.respond(Some(synced()), false);
        h.advance(secs(2));
        let retried = h.ask(&repo, track());

        assert_eq!(
            retried,
            Some(synced()),
            "The retry finds the synced lyrics the failed source holds"
        );
        assert_eq!(flaky.call_count(), 2);
    }

    #[test]
    fn plain_fallback_with_all_providers_clean_stays_cached() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(plain("words")));
        let repo = h.make_repo(vec![dynp(&provider)], secs(5));

        h.ask(&repo, track());
        h.advance(secs(1_000_000_000));
        let result = h.ask(&repo, track());

        assert_eq!(result, Some(plain("words")));
        assert_eq!(provider.call_count(), 1);
    }

    #[test]
    fn synced_result_stays_cached_even_if_an_earlier_provider_failed() {
        let h = Harness::new();
        let flaky = FakeProvider::failing("flaky", &h.clock);
        let good = FakeProvider::result("good", &h.clock, Some(synced()));
        let repo = h.make_repo(vec![dynp(&flaky), dynp(&good)], secs(5));

        h.ask(&repo, track());
        h.advance(secs(1_000_000_000));
        let result = h.ask(&repo, track());

        assert_eq!(result, Some(synced()));
        assert_eq!(flaky.call_count(), 1);
        assert_eq!(good.call_count(), 1);
    }

    // ---- 可信度把關

    #[test]
    fn implausible_synced_result_is_skipped_and_walk_continues() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(implausible()));
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        let result = h.ask(&repo, track());

        assert_eq!(result, Some(synced()));
        assert_eq!(first.call_count(), 1);
        assert_eq!(second.call_count(), 1);
    }

    #[test]
    fn implausible_synced_does_not_become_fallback() {
        let h = Harness::new();
        let only = FakeProvider::result("only", &h.clock, Some(implausible()));
        let repo = h.repo(vec![dynp(&only)]);

        assert_eq!(h.ask(&repo, track()), None);
    }

    #[test]
    fn implausible_synced_then_later_plain_returns_plain() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(implausible()));
        let second = FakeProvider::result("second", &h.clock, Some(plain("words")));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
    }

    #[test]
    fn implausible_synced_counts_as_clean_answer_for_caching() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(implausible()));
        let repo = h.make_repo(vec![dynp(&provider)], secs(5));

        assert_eq!(h.ask(&repo, track()), None);
        h.advance(secs(3_599));
        h.ask(&repo, track());
        assert_eq!(
            provider.call_count(),
            1,
            "Remembered for negativeTTL (3600), not failureTTL (60)"
        );

        h.advance(secs(2));
        h.ask(&repo, track());
        assert_eq!(provider.call_count(), 2);
    }

    #[test]
    fn plausibility_uses_track_duration() {
        // 最後一行在 150 秒：放得進 200 秒的曲子，放不進 100 秒的。
        let lyrics = Lyrics::Synced(vec![line(5.0, "a"), line(150.0, "b")]);
        let h = Harness::new();

        let long_repo = h.repo(vec![dynp(&FakeProvider::result(
            "p",
            &h.clock,
            Some(lyrics.clone()),
        ))]);
        let long_enough = h.ask(&long_repo, identity("T", 200.0));
        let short_repo = h.repo(vec![dynp(&FakeProvider::result(
            "p",
            &h.clock,
            Some(lyrics.clone()),
        ))]);
        let too_short = h.ask(&short_repo, identity("T", 100.0));

        assert_eq!(long_enough, Some(lyrics));
        assert_eq!(too_short, None);
    }

    // ------------------------------------------------------------ 簡報點名的行為（正式版預設值）

    /// 高順位只有純文字、低順位有同步 → 用同步。
    #[test]
    fn synced_result_outranks_provider_priority() {
        let h = Harness::new();
        let high = FakeProvider::result("high", &h.clock, Some(plain("high words")));
        let low = FakeProvider::result("low", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&high), dynp(&low)]);

        assert_eq!(h.ask(&repo, track()), Some(synced()));
        assert_eq!((high.call_count(), low.call_count()), (1, 1));
    }

    /// 純文字只在沒有任何可信的同步歌詞時才回傳，而且是最高順位的那一份。
    #[test]
    fn plain_is_only_a_fallback() {
        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(plain("first")));
        let second = FakeProvider::result("second", &h.clock, Some(plain("second")));
        let third = FakeProvider::result("third", &h.clock, Some(implausible()));
        let repo = h.repo(vec![dynp(&first), dynp(&second), dynp(&third)]);
        assert_eq!(h.ask(&repo, track()), Some(plain("first")));
        assert_eq!(third.call_count(), 1, "plain never stops the walk");

        let h = Harness::new();
        let first = FakeProvider::result("first", &h.clock, Some(plain("first")));
        let last = FakeProvider::result("last", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&first), dynp(&last)]);
        assert_eq!(h.ask(&repo, track()), Some(synced()));
    }

    /// 不可信的同步結果等同「乾淨的沒有」：不是失敗（快取 24 小時而非 120 秒），也不當保底。
    #[test]
    fn implausible_synced_counts_as_empty_not_failure() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(implausible()));
        let repo = h.repo(vec![dynp(&provider)]);

        assert_eq!(h.ask(&repo, track()), None);
        h.advance(secs(121));
        assert_eq!(h.ask(&repo, track()), None);
        assert_eq!(
            provider.call_count(),
            1,
            "not a failure: outlives failure_ttl"
        );
        h.advance(secs(86_400));
        h.ask(&repo, track());
        assert_eq!(provider.call_count(), 2, "but expires with negative_ttl");
    }

    #[test]
    fn success_is_cached_forever() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        h.ask(&repo, track());
        h.advance(secs(10 * 365 * 86_400));

        assert_eq!(h.ask(&repo, track()), Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    #[test]
    fn all_clean_misses_are_cached_for_24_hours() {
        let h = Harness::new();
        let a = FakeProvider::result("a", &h.clock, None);
        let b = FakeProvider::result("b", &h.clock, None);
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);

        h.ask(&repo, track());
        h.advance(secs(86_399));
        h.ask(&repo, track());
        assert_eq!((a.call_count(), b.call_count()), (1, 1));

        h.advance(secs(1));
        h.ask(&repo, track());
        assert_eq!((a.call_count(), b.call_count()), (2, 2));
    }

    #[test]
    fn any_failure_caches_only_120_seconds() {
        let h = Harness::new();
        let empty = FakeProvider::result("empty", &h.clock, None);
        let failing = FakeProvider::failing("failing", &h.clock);
        let repo = h.repo(vec![dynp(&empty), dynp(&failing)]);

        h.ask(&repo, track());
        h.advance(secs(119));
        h.ask(&repo, track());
        assert_eq!(failing.call_count(), 1);

        h.advance(secs(1));
        h.ask(&repo, track());
        assert_eq!((empty.call_count(), failing.call_count()), (2, 2));
    }

    #[test]
    fn plain_fallback_found_while_a_source_failed_expires_after_120_seconds() {
        let h = Harness::new();
        let failing = FakeProvider::failing("failing", &h.clock);
        let plain_source = FakeProvider::result("plain", &h.clock, Some(plain("words")));
        let repo = h.repo(vec![dynp(&failing), dynp(&plain_source)]);

        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
        h.advance(secs(119));
        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
        assert_eq!(plain_source.call_count(), 1);

        h.advance(secs(1));
        assert_eq!(h.ask(&repo, track()), Some(plain("words")));
        assert_eq!(plain_source.call_count(), 2);
    }

    /// 第一個來源永不回應：`advance(5s)` 後仍走到第二個來源。
    #[test]
    fn provider_timeout_is_a_miss_and_does_not_stall_the_walk() {
        let h = Harness::new();
        let hanging = FakeProvider::hanging("hanging", &h.clock, Some(synced()));
        let second = FakeProvider::result("second", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&hanging), dynp(&second)]);

        let lookup = h.start(&repo, track());
        h.run();
        h.advance(millis(4_999));
        assert_eq!(lookup.outcome(), None);
        assert_eq!(second.call_count(), 0);

        h.advance(millis(1));
        assert_eq!(lookup.finished(), Some(synced()));
        assert_eq!(second.call_count(), 1);
        assert_eq!(h.clock.pending_sleeps(), 0, "the hanging fetch was dropped");
    }

    #[test]
    fn concurrent_callers_share_one_lookup() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), secs(1));
        let repo = h.repo(vec![dynp(&provider)]);

        let callers: Vec<Lookup> = (0..3).map(|_| h.start(&repo, track())).collect();
        h.run();
        assert_eq!(repo.in_flight_waiters(&track()), 3);
        h.advance(secs(1));

        for c in &callers {
            assert_eq!(c.finished(), Some(synced()));
        }
        assert_eq!(provider.call_count(), 1);
        assert_eq!(repo.in_flight_waiters(&track()), 0);
    }

    #[test]
    fn one_cancelled_waiter_does_not_abandon_the_lookup_for_the_others() {
        let h = Harness::new();
        let provider = FakeProvider::slow("p", &h.clock, Some(synced()), secs(1));
        let repo = h.repo(vec![dynp(&provider)]);

        let a = h.start(&repo, track());
        let b = h.start(&repo, track());
        let c = h.start(&repo, track());
        h.run();
        drop(b.task);
        h.run();
        assert_eq!(repo.in_flight_waiters(&track()), 2);
        h.advance(secs(1));

        assert_eq!(a.finished(), Some(synced()));
        assert_eq!(c.finished(), Some(synced()));
        assert_eq!(*b.slot.lock().unwrap(), None);
        assert_eq!(provider.call_count(), 1);

        // 查詢跑完了，所以結果已入快取。
        assert_eq!(h.ask(&repo, track()), Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    /// spec 第 8 節的擴充點：新來源插在最前面，其他程式碼不必動。
    #[test]
    fn inserting_a_provider_at_the_front_needs_no_other_change() {
        let base = |h: &Harness| {
            (
                FakeProvider::result("plain", &h.clock, Some(plain("words"))),
                FakeProvider::result("synced", &h.clock, Some(synced())),
            )
        };

        // 新來源有同步歌詞：它的結果勝出，後面的來源都不必問。
        let h = Harness::new();
        let (a, b) = base(&h);
        let newcomer_lyrics = Lyrics::Synced(vec![line(1.0, "new"), line(2.0, "source")]);
        let newcomer = FakeProvider::result("new", &h.clock, Some(newcomer_lyrics.clone()));
        let repo = h.repo(vec![dynp(&newcomer), dynp(&a), dynp(&b)]);
        assert_eq!(h.ask(&repo, track()), Some(newcomer_lyrics));
        assert_eq!((a.call_count(), b.call_count()), (0, 0));

        // 新來源沒有：走訪照舊落到原本的來源。
        let h = Harness::new();
        let (a, b) = base(&h);
        let newcomer = FakeProvider::result("new", &h.clock, None);
        let repo = h.repo(vec![dynp(&newcomer), dynp(&a), dynp(&b)]);
        assert_eq!(h.ask(&repo, track()), Some(synced()));
        assert_eq!(
            (newcomer.call_count(), a.call_count(), b.call_count()),
            (1, 1, 1)
        );

        // 新來源只有純文字：它成為保底，但原本的同步歌詞仍然勝出。
        let h = Harness::new();
        let (_, b) = base(&h);
        let newcomer = FakeProvider::result("new", &h.clock, Some(plain("new words")));
        let repo = h.repo(vec![dynp(&newcomer), dynp(&b)]);
        assert_eq!(h.ask(&repo, track()), Some(synced()));
    }

    /// 多執行緒競態：呼叫者查快取沒命中、登記了查詢，但在查詢第一次被輪詢之前，另一條執行緒的查詢
    /// 剛好寫好快取並移除了它的表項。被合併的查詢開頭必須重查快取，不得再問一輪來源。
    /// 以手動排出該交錯：呼叫者的 future 已建立（快取當時沒命中）、尚未輪詢，快取才被填入。
    #[test]
    fn coalesced_run_rechecks_the_cache_before_walking_providers() {
        use futures_util::FutureExt;

        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(plain("from provider")));
        let repo = h.repo(vec![dynp(&provider)]);

        let caller = repo.lyrics(track());
        assert_eq!(repo.in_flight_waiters(&track()), 1);
        lock(&repo.inner.cache).insert(
            track().cache_key(),
            CacheEntry::Found {
                hit: hit(synced()),
                until: None,
            },
        );

        assert_eq!(
            caller.now_or_never().map(|r| r.map(|h| h.lyrics)),
            Some(Some(synced())),
            "the run must answer from the cache another run just wrote"
        );
        assert_eq!(provider.call_count(), 0, "no duplicate lookup");
        assert_eq!(repo.in_flight_waiters(&track()), 0);
    }

    /// 建立後從未輪詢就 drop：不留下等待者，也不碰任何來源。
    #[test]
    fn dropping_an_unpolled_lookup_leaves_no_waiter_and_calls_no_provider() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        let caller = repo.lyrics(track());
        assert_eq!(repo.in_flight_waiters(&track()), 1);
        drop(caller);

        assert_eq!(repo.in_flight_waiters(&track()), 0);
        assert_eq!(provider.call_count(), 0);
        // 沒有留下快取：之後的查詢照常問來源。
        assert_eq!(h.ask(&repo, track()), Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    /// 快取鍵不含曲長：同一首歌回報不同長度（或沒有長度）時直接命中快取。
    #[test]
    fn cache_key_ignores_duration() {
        let h = Harness::new();
        let provider = FakeProvider::result("p", &h.clock, Some(synced()));
        let repo = h.repo(vec![dynp(&provider)]);

        h.ask(&repo, identity("T", 200.0));
        assert_eq!(h.ask(&repo, identity("T", 260.0)), Some(synced()));
        let unknown = TrackIdentity {
            duration: None,
            ..track()
        };
        assert_eq!(h.ask(&repo, unknown), Some(synced()));
        assert_eq!(provider.call_count(), 1);
    }

    // ------------------------------------------------------------ 來源網址（LyricsHit）

    #[test]
    fn a_hit_carries_the_providers_url() {
        let h = Harness::new();
        let provider =
            FakeProvider::result_hit("a", &h.clock, Some(hit_at(synced(), "https://a.example/1")));
        let repo = h.repo(vec![dynp(&provider)]);

        let got = h.ask_hit(&repo, track()).expect("a hit");

        assert_eq!(got.lyrics, synced());
        assert_eq!(got.url, "https://a.example/1");
        assert!(got.url.starts_with("https://"));
    }

    #[test]
    fn cached_hit_still_carries_its_url() {
        let h = Harness::new();
        let synced_src =
            FakeProvider::result_hit("s", &h.clock, Some(hit_at(synced(), "https://s.example/9")));
        let repo = h.repo(vec![dynp(&synced_src)]);
        let first = h.ask_hit(&repo, track()).expect("a hit");
        let again = h.ask_hit(&repo, track()).expect("a cached hit");
        assert_eq!(synced_src.call_count(), 1, "the second answer is cached");
        assert_eq!(first, again);
        assert_eq!(again.url, "https://s.example/9");

        // 純文字保底也一樣：從快取回來的仍帶來源網址。
        let plain_src = FakeProvider::result_hit(
            "p",
            &h.clock,
            Some(hit_at(plain("words"), "https://p.example/2")),
        );
        let repo = h.repo(vec![dynp(&plain_src)]);
        h.ask_hit(&repo, track());
        let again = h.ask_hit(&repo, track()).expect("a cached plain hit");
        assert_eq!(plain_src.call_count(), 1);
        assert_eq!(again.url, "https://p.example/2");
    }

    #[test]
    fn plain_fallback_carries_the_fallback_providers_url_not_a_later_one() {
        // A 純文字、B 沒有：用 A 的網址。
        let h = Harness::new();
        let a = FakeProvider::result_hit(
            "a",
            &h.clock,
            Some(hit_at(plain("a words"), "https://a.example/1")),
        );
        let b = FakeProvider::result("b", &h.clock, None);
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);
        let got = h.ask_hit(&repo, track()).expect("fallback");
        assert_eq!(got.lyrics, plain("a words"));
        assert_eq!(got.url, "https://a.example/1");

        // A、B 都純文字：保底是第一個，網址也是第一個的。
        let h = Harness::new();
        let a = FakeProvider::result_hit(
            "a",
            &h.clock,
            Some(hit_at(plain("a words"), "https://a.example/1")),
        );
        let b = FakeProvider::result_hit(
            "b",
            &h.clock,
            Some(hit_at(plain("b words"), "https://b.example/2")),
        );
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);
        let got = h.ask_hit(&repo, track()).expect("fallback");
        assert_eq!(got.url, "https://a.example/1");

        // A 純文字、B 帶時間軸：整個換成 B 的 hit。
        let h = Harness::new();
        let a = FakeProvider::result_hit(
            "a",
            &h.clock,
            Some(hit_at(plain("a words"), "https://a.example/1")),
        );
        let b =
            FakeProvider::result_hit("b", &h.clock, Some(hit_at(synced(), "https://b.example/2")));
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);
        let got = h.ask_hit(&repo, track()).expect("synced wins");
        assert_eq!(got, hit_at(synced(), "https://b.example/2"));
    }

    #[test]
    fn implausible_synced_result_does_not_leak_its_url() {
        let h = Harness::new();
        let a = FakeProvider::result_hit(
            "a",
            &h.clock,
            Some(hit_at(implausible(), "https://a.example/leak")),
        );
        let b = FakeProvider::result("b", &h.clock, None);
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);
        assert_eq!(h.ask_hit(&repo, track()), None);

        // 後面有純文字：用它自己的網址，不是不可信那個的。
        let h = Harness::new();
        let a = FakeProvider::result_hit(
            "a",
            &h.clock,
            Some(hit_at(implausible(), "https://a.example/leak")),
        );
        let b = FakeProvider::result_hit(
            "b",
            &h.clock,
            Some(hit_at(plain("words"), "https://b.example/2")),
        );
        let repo = h.repo(vec![dynp(&a), dynp(&b)]);
        let got = h.ask_hit(&repo, track()).expect("plain fallback");
        assert_eq!(got.url, "https://b.example/2");
    }

    // ------------------------------------------------------------ 曲長放寬的命中（Rust 版新增）

    use crate::lyrics::identity::DurationFit;

    fn loose(lyrics: Lyrics, url: &str) -> Option<LyricsHit> {
        Some(hit_at(lyrics, url).with_fit(DurationFit::Loose))
    }

    #[test]
    fn a_loose_duration_hit_does_not_stop_the_walk_before_a_close_one() {
        let h = Harness::new();
        let first = FakeProvider::result_hit("a", &h.clock, loose(synced(), "https://a.example/1"));
        let second =
            FakeProvider::result_hit("b", &h.clock, Some(hit_at(synced(), "https://b.example/2")));
        let repo = h.repo(vec![dynp(&first), dynp(&second)]);

        let got = h.ask_hit(&repo, track()).expect("a hit");

        assert_eq!(got.url, "https://b.example/2");
    }

    #[test]
    fn a_loose_synced_hit_beats_plain_but_the_earlier_loose_one_wins() {
        let h = Harness::new();
        let words = FakeProvider::result_hit(
            "p",
            &h.clock,
            Some(hit_at(plain("words"), "https://p.example/0")),
        );
        let first = FakeProvider::result_hit("a", &h.clock, loose(synced(), "https://a.example/1"));
        let second =
            FakeProvider::result_hit("b", &h.clock, loose(synced(), "https://b.example/2"));
        let repo = h.repo(vec![dynp(&words), dynp(&first), dynp(&second)]);

        let got = h.ask_hit(&repo, track()).expect("a hit");

        assert_eq!(got.url, "https://a.example/1");
        assert_eq!(second.call_count(), 1);
    }

    /// 另一個來源失敗時，放寬的命中只短暫記住，之後可能換成吻合的版本。
    #[test]
    fn a_loose_hit_found_while_a_provider_failed_is_retried_later() {
        let h = Harness::new();
        let flaky = FakeProvider::failing("flaky", &h.clock);
        let other = FakeProvider::result_hit("a", &h.clock, loose(synced(), "https://a.example/1"));
        let repo = h.make_repo(vec![dynp(&flaky), dynp(&other)], secs(5));
        assert!(h.ask_hit(&repo, track()).unwrap().loose_duration);

        flaky.respond(Some(synced()), false);
        h.advance(secs(61));

        assert!(!h.ask_hit(&repo, track()).unwrap().loose_duration);
    }
}
