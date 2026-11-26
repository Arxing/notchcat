//! 可在執行時抽換的歌詞解析器，以及依設定組歌詞提供者清單的函式。
//!
//! [`SwitchableResolver`] 本身實作 [`LyricsResolving`]，`MusicController` 只持有它一份；選單切換歌詞來源時，
//! app 呼叫 [`build_repository`] 依新設定組出新的 `LyricsRepository`，再 [`SwitchableResolver::swap`] 換上去。
//!
//! # 行為與已知限制
//!
//! - 提供者順序固定為 `[LRCLIB, NetEase]`，再依開關過濾；啟用的來源同時查，順序只決定同序版本誰在前。
//!   兩個都關 → 沒有任何提供者的 repository，查詢直接回空清單，不發任何網路請求。
//! - 切換來源會**重建** repository，所以**快取從空的開始**（舊快取不會帶過去）。
//! - **目前正在播放的曲目不會重新查詢**，要等下一次換曲：`MusicController` 沒有「重新整理歌詞」的入口。
//!   已經顯示的歌詞（來自舊來源）會一直顯示到換曲為止。
//! - 抽換前就已經發出的查詢，由發出它的那個解析器做完（結果仍會被呼叫者收到）；
//!   [`SwitchableResolver::lyrics`] 在**呼叫當下**就選定解析器，不是在 future 第一次被輪詢時。
//! - 內部以 `Mutex` 保護 `Arc`；鎖只在複製或替換 `Arc` 的瞬間持有，**從不跨過 `await`**。

use std::sync::{Arc, Mutex};

use musicat_core::http::HttpClient;
use musicat_core::lyrics::lrclib::LrclibProvider;
use musicat_core::lyrics::netease::NetEaseProvider;
use musicat_core::lyrics::provider::LyricsProvider;
use musicat_core::lyrics::repository::{LyricsRepository, LyricsResolving, RepositoryConfig};
use musicat_core::lyrics::{LyricsHit, TrackIdentity};
use musicat_core::runtime::{BoxFuture, Clock};

use crate::config::Sources;

/// 鎖中毒（別的執行緒在持鎖時 panic）時照樣取值：裡面只是一個 `Arc`，不會處於半更新的狀態。
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 委派給可抽換的 `Arc<dyn LyricsResolving>`（見模組說明）。
pub struct SwitchableResolver {
    current: Mutex<Arc<dyn LyricsResolving>>,
}

impl SwitchableResolver {
    pub fn new(initial: Arc<dyn LyricsResolving>) -> Self {
        Self {
            current: Mutex::new(initial),
        }
    }

    /// 換上新的解析器。之後的查詢用新的；已經發出的查詢仍由舊的完成。
    pub fn swap(&self, new: Arc<dyn LyricsResolving>) {
        let old = std::mem::replace(&mut *lock(&self.current), new);
        // 舊解析器在鎖外釋放（它的 drop 可能很重，不該在持鎖時跑）。
        drop(old);
    }
}

impl LyricsResolving for SwitchableResolver {
    fn lyrics(&self, track: TrackIdentity) -> BoxFuture<'_, Vec<LyricsHit>> {
        let resolver = lock(&self.current).clone();
        // future 自己擁有那一份 Arc：之後被 swap 掉也不影響它。
        Box::pin(async move { resolver.lyrics(track).await })
    }
}

/// 依開關組出歌詞解析器：提供者順序固定為 [LRCLIB, NetEase]，關掉的略過。
/// 全部關掉時是沒有提供者的 repository（查詢回空清單，不發請求）。
pub fn build_repository(
    sources: &Sources,
    http: Arc<dyn HttpClient>,
    clock: Arc<dyn Clock>,
) -> Arc<dyn LyricsResolving> {
    let mut providers: Vec<Arc<dyn LyricsProvider>> = Vec::new();
    if sources.lrclib {
        providers.push(Arc::new(LrclibProvider::new(http.clone())));
    }
    if sources.netease {
        providers.push(Arc::new(NetEaseProvider::new(http)));
    }
    Arc::new(LyricsRepository::new(
        providers,
        clock,
        RepositoryConfig::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_channel::oneshot;
    use musicat_core::lyrics::Lyrics;
    use musicat_core::testing::{FakeClock, FakeHttp};
    use std::task::{Context, Poll, Waker};

    fn track() -> TrackIdentity {
        TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: None,
            duration: Some(200.0),
        }
    }

    fn hosts(http: &FakeHttp) -> Vec<String> {
        http.requests()
            .iter()
            .map(|r| {
                r.url
                    .split("//")
                    .nth(1)
                    .unwrap()
                    .split('/')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    fn lookup_hit(resolver: &dyn LyricsResolving) -> Option<LyricsHit> {
        futures_executor::block_on(resolver.lyrics(track()))
            .into_iter()
            .next()
    }

    fn lookup(resolver: &dyn LyricsResolving) -> Option<Lyrics> {
        lookup_hit(resolver).map(|hit| hit.lyrics)
    }

    fn first_lyrics(versions: Vec<LyricsHit>) -> Option<Lyrics> {
        versions.into_iter().next().map(|hit| hit.lyrics)
    }

    fn repo(sources: Sources, http: &Arc<FakeHttp>) -> Arc<dyn LyricsResolving> {
        build_repository(&sources, http.clone(), Arc::new(FakeClock::new(1_000.0)))
    }

    /// 回傳固定標記的解析器；`gate` 有值時要等它放行才回覆。
    struct Stub {
        label: &'static str,
        gate: Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl Stub {
        fn fixed(label: &'static str) -> Arc<dyn LyricsResolving> {
            Arc::new(Self {
                label,
                gate: Mutex::new(None),
            })
        }

        fn gated(label: &'static str) -> (Arc<dyn LyricsResolving>, oneshot::Sender<()>) {
            let (tx, rx) = oneshot::channel();
            let stub = Arc::new(Self {
                label,
                gate: Mutex::new(Some(rx)),
            });
            (stub, tx)
        }
    }

    impl LyricsResolving for Stub {
        fn lyrics(&self, _track: TrackIdentity) -> BoxFuture<'_, Vec<LyricsHit>> {
            let gate = lock(&self.gate).take();
            let label = self.label;
            Box::pin(async move {
                if let Some(rx) = gate {
                    let _ = rx.await;
                }
                vec![LyricsHit::new(
                    Lyrics::Plain(label.into()),
                    format!("https://test.example/{label}"),
                )]
            })
        }
    }

    fn plain(label: &str) -> Option<Lyrics> {
        Some(Lyrics::Plain(label.into()))
    }

    #[test]
    fn providers_are_queried_in_the_fixed_order_lrclib_then_netease() {
        let http = Arc::new(FakeHttp::new()); // 沒排程的 URL 都是傳輸錯誤：兩個來源都會被問到
        assert_eq!(lookup(repo(Sources::default(), &http).as_ref()), None);
        let hosts = hosts(&http);
        let first_lrclib = hosts.iter().position(|h| h == "lrclib.net").unwrap();
        let first_netease = hosts.iter().position(|h| h == "music.163.com").unwrap();
        assert_eq!(first_lrclib, 0, "{hosts:?}");
        assert!(first_lrclib < first_netease, "{hosts:?}");
    }

    #[test]
    fn only_lrclib_enabled_never_contacts_netease() {
        let http = Arc::new(FakeHttp::new());
        let sources = Sources {
            lrclib: true,
            netease: false,
        };
        lookup(repo(sources, &http).as_ref());
        let hosts = hosts(&http);
        assert!(!hosts.is_empty());
        assert!(hosts.iter().all(|h| h == "lrclib.net"), "{hosts:?}");
    }

    #[test]
    fn only_netease_enabled_never_contacts_lrclib() {
        let http = Arc::new(FakeHttp::new());
        let sources = Sources {
            lrclib: false,
            netease: true,
        };
        lookup(repo(sources, &http).as_ref());
        let hosts = hosts(&http);
        assert!(!hosts.is_empty());
        assert!(hosts.iter().all(|h| h == "music.163.com"), "{hosts:?}");
    }

    #[test]
    fn both_sources_off_returns_nothing_without_any_http_call() {
        let http = Arc::new(FakeHttp::new());
        let sources = Sources {
            lrclib: false,
            netease: false,
        };
        let resolver = repo(sources, &http);
        assert_eq!(lookup(resolver.as_ref()), None);
        assert_eq!(lookup(resolver.as_ref()), None);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn delegates_to_the_initial_resolver() {
        let sw = SwitchableResolver::new(Stub::fixed("first"));
        assert_eq!(lookup(&sw), plain("first"));
    }

    #[test]
    fn swap_takes_effect_for_subsequent_lookups() {
        let sw = SwitchableResolver::new(Stub::fixed("old"));
        assert_eq!(lookup(&sw), plain("old"));
        sw.swap(Stub::fixed("new"));
        assert_eq!(lookup(&sw), plain("new"));
        assert_eq!(lookup(&sw), plain("new"));
    }

    #[test]
    fn swap_changes_the_url_source_for_later_lookups() {
        let sw = SwitchableResolver::new(Stub::fixed("old"));
        assert_eq!(
            lookup_hit(&sw).map(|h| h.url),
            Some("https://test.example/old".to_string())
        );
        sw.swap(Stub::fixed("new"));
        let hit = lookup_hit(&sw).expect("a hit from the new resolver");
        assert_eq!(hit.url, "https://test.example/new");
        assert_eq!(hit.lyrics, Lyrics::Plain("new".into()));
    }

    #[test]
    fn an_in_flight_lookup_completes_with_the_old_resolver() {
        let (old, release) = Stub::gated("old");
        let sw = SwitchableResolver::new(old);

        let mut in_flight = sw.lyrics(track());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(in_flight.as_mut().poll(&mut cx), Poll::Pending));

        sw.swap(Stub::fixed("new"));
        release.send(()).unwrap();

        assert_eq!(
            first_lyrics(futures_executor::block_on(in_flight)),
            plain("old")
        );
        assert_eq!(lookup(&sw), plain("new"));
    }

    #[test]
    fn the_resolver_is_chosen_when_lookup_is_called_not_when_first_polled() {
        let sw = SwitchableResolver::new(Stub::fixed("old"));
        let not_yet_polled = sw.lyrics(track());
        sw.swap(Stub::fixed("new"));
        assert_eq!(
            first_lyrics(futures_executor::block_on(not_yet_polled)),
            plain("old")
        );
    }

    #[test]
    fn swapping_in_a_rebuilt_repository_starts_with_an_empty_cache() {
        let http = Arc::new(FakeHttp::new());
        let sw = SwitchableResolver::new(repo(Sources::default(), &http));
        lookup(&sw);
        let after_first = http.requests().len();
        lookup(&sw);
        // 同一個 repository：失敗回覆有 120 秒快取，第二次不再發請求。
        assert_eq!(http.requests().len(), after_first);
        sw.swap(repo(Sources::default(), &http));
        lookup(&sw);
        assert!(
            http.requests().len() > after_first,
            "new repository must ask again"
        );
    }

    #[test]
    fn the_resolver_and_its_futures_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_send<T: Send>(_: &T) {}
        assert_send_sync::<SwitchableResolver>();
        let sw = SwitchableResolver::new(Stub::fixed("x"));
        let fut = sw.lyrics(track());
        assert_send(&fut);
    }

    #[test]
    fn swapping_from_another_thread_while_lookups_run_is_safe() {
        let sw = Arc::new(SwitchableResolver::new(Stub::fixed("a")));
        let worker = {
            let sw = sw.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    assert!(lookup(sw.as_ref()).is_some());
                }
            })
        };
        for i in 0..200 {
            sw.swap(Stub::fixed(if i % 2 == 0 { "a" } else { "b" }));
        }
        worker.join().unwrap();
    }
}
