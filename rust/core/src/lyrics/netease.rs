//! 網易雲音樂非官方端點（移植自 Swift `NetEaseProvider`）。對華語曲庫的涵蓋比 LRCLIB 好，
//! 代價是沒有文件、隨時可能改版，所以才需要 `LyricsProvider` 這層隔離。
//!
//! 只用原文歌詞（`lrc`）。回應也帶翻譯（`tlyric`），併進來的話，與原文同時間戳的翻譯行會蓋掉原文。

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Deserializer};

use super::identity::DurationFit;
use super::provider::{
    build_url, candidate_title, decode_object, request, success_body, Candidates, LyricsProvider,
    ProviderError,
};
use super::{parse_lrc, Lyrics, LyricsHit, TrackIdentity};
use crate::http::HttpClient;
use crate::runtime::BoxFuture;

/// 搜尋請求的逾時；逾時視同該請求失敗。搜尋與歌詞是先後兩個階段，兩個逾時加起來要比來源上限
/// （`RepositoryConfig::provider_timeout`）短，一個卡住的歌詞請求才不會連累其他首歌已到手的歌詞。
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_millis(2_500);
/// 單一歌詞請求的逾時（各首同時發出）。
pub(crate) const LYRIC_TIMEOUT: Duration = Duration::from_secs(2);

pub struct NetEaseProvider {
    http: Arc<dyn HttpClient>,
}

/// 缺欄位、`null`、多餘欄位都容忍；數字可以是整數或浮點。型別不對視為壞回應。
#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default, deserialize_with = "optional_integer")]
    code: Option<i64>,
    result: Option<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    songs: Option<Vec<Song>>,
}

#[derive(Deserialize)]
struct Song {
    #[serde(deserialize_with = "integer")]
    id: i64,
    name: Option<String>,
    /// 毫秒。
    duration: Option<f64>,
    artists: Option<Vec<Option<Artist>>>,
}

#[derive(Deserialize)]
struct Artist {
    name: Option<String>,
}

/// 搜尋結果裡挑中、要取歌詞的一首歌。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SelectedSong {
    pub id: i64,
    /// 曲長（秒）。
    pub seconds: Option<f64>,
    /// 任一位演出者和曲目相符（見 [`TrackIdentity::matches_artist`]）。
    pub artist_matches: bool,
}

#[derive(Deserialize)]
struct LyricResponse {
    #[serde(default, deserialize_with = "optional_integer")]
    code: Option<i64>,
    lrc: Option<Lrc>,
}

#[derive(Deserialize)]
struct Lrc {
    lyric: Option<String>,
}

/// 接受 `7` 與 `7.0`，拒絕 `7.5`、字串與超出範圍的值。
fn integer<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let n = serde_json::Number::deserialize(d)?;
    n.as_i64()
        .or_else(|| {
            n.as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0)
                .map(|f| f as i64)
        })
        .ok_or_else(|| serde::de::Error::custom("expected an integer"))
}

fn optional_integer<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    struct Wrapper(#[serde(deserialize_with = "integer")] i64);
    Ok(Option::<Wrapper>::deserialize(d)?.map(|w| w.0))
}

const REFERER: [(&str, &str); 1] = [("Referer", "https://music.163.com/")];

/// 一次查詢最多取幾首歌的歌詞。
const MAX_SONGS: usize = 3;

/// 單行「歌詞」的佔位通知。純音樂與未收錄的歌會拿它當歌詞，但它不是歌詞
/// （不帶時間軸的還會被當成純文字保底顯示），所以視為沒有歌詞。
const PLACEHOLDERS: [&str; 4] = ["纯音乐，请欣赏", "纯音乐, 请欣赏", "暂无歌词", "没有歌词"];

impl NetEaseProvider {
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self { http }
    }

    pub(crate) fn search_url(track: &TrackIdentity) -> String {
        let n = track.normalized();
        let s = format!("{} {}", n.title, n.artist);
        build_url(
            "https://music.163.com/api/search/get/web",
            &[("s", &s), ("type", "1"), ("limit", "5")],
        )
    }

    pub(crate) fn lyric_url(song_id: i64) -> String {
        let id = song_id.to_string();
        build_url(
            "https://music.163.com/api/song/lyric",
            &[("id", &id), ("lv", "1"), ("kv", "1"), ("tv", "-1")],
        )
    }

    /// 這首歌在網易雲網站上的頁面。
    pub(crate) fn song_url(song_id: i64) -> String {
        format!("https://music.163.com/song?id={song_id}")
    }

    #[cfg(test)]
    pub(crate) fn select_song_id(
        data: &[u8],
        track: &TrackIdentity,
    ) -> Result<Option<i64>, ProviderError> {
        Ok(Self::select_songs(data, track)?.first().map(|song| song.id))
    }

    /// 歌名相符、曲長在放寬容忍內（或未知）的搜尋結果，最多 [`MAX_SONGS`] 首：演出者相符的排在前面，
    /// 其次曲長吻合的排在放寬的前面，同類保留搜尋順序。歌名不同的結果永遠不收：錯的歌詞比沒有更糟。
    /// 歌名寬鬆比對，因為這個服務會對繁體查詢回簡體歌名。演出者不相符的仍可能入選（播放器回報的
    /// 演出者常是頻道名稱），由 `LyricsRepository` 決定去留。
    ///
    /// 本體不是成功的搜尋回應時回 `Err`。
    pub(crate) fn select_songs(
        data: &[u8],
        track: &TrackIdentity,
    ) -> Result<Vec<SelectedSong>, ProviderError> {
        let response: SearchResponse = decode_object(data)?;
        if response.code.is_some_and(|code| code != 200) {
            return Err(ProviderError::BadResponse);
        }
        // 真正的零結果回應仍帶 `code: 200`。
        if response.code.is_none() && response.result.is_none() {
            return Err(ProviderError::BadResponse);
        }

        let wanted = track.normalized().title;
        let songs = response.result.and_then(|r| r.songs).unwrap_or_default();
        // 每首合格的歌連同它的排序鍵：（演出者不相符，曲長只在放寬容忍內），小的在前。
        let mut eligible = Vec::new();
        for song in songs {
            let Some(name) = song.name else { continue };
            if !TrackIdentity::loosely_matches(&candidate_title(&name), &wanted) {
                continue;
            }
            let seconds = song.duration.map(|ms| ms / 1000.0);
            let Some(fit) = track.duration_fit(seconds) else {
                continue;
            };
            let artists = song.artists.iter().flatten().flatten();
            let artist_matches =
                track.matches_artist(artists.filter_map(|artist| artist.name.as_deref()));
            eligible.push((
                (!artist_matches, fit == DurationFit::Loose),
                SelectedSong {
                    id: song.id,
                    seconds,
                    artist_matches,
                },
            ));
        }
        // 穩定排序：同類維持搜尋順序。
        eligible.sort_by_key(|&(order, _)| order);
        Ok(eligible
            .into_iter()
            .take(MAX_SONGS)
            .map(|(_, song)| song)
            .collect())
    }

    /// 歌詞回應中的歌詞；沒有原文歌詞時為 `None`。
    fn parse_lyric(data: &[u8]) -> Result<Option<Lyrics>, ProviderError> {
        let response: LyricResponse = decode_object(data)?;
        // 被拒絕的回應如 `{"code":-460,"msg":"Cheating"}` 沒有 `lrc`，不能看起來像「這首歌沒歌詞」。
        if response.code.is_some_and(|code| code != 200) {
            return Err(ProviderError::BadResponse);
        }
        let Some(raw) = response.lrc.and_then(|l| l.lyric) else {
            return Ok(None);
        };
        if raw.trim_matches(char::is_whitespace).is_empty() {
            return Ok(None);
        }
        let Some(parsed) = parse_lrc(&raw) else {
            return Ok(None);
        };
        Ok(if is_placeholder(&parsed) {
            None
        } else {
            Some(parsed)
        })
    }

    #[cfg(test)]
    fn decode_lyric(data: &[u8]) -> Option<Lyrics> {
        Self::parse_lyric(data).unwrap_or(None)
    }

    /// 搜尋一次，再同時取各首合格歌曲的歌詞。搜尋失敗就是整個查詢失敗。
    async fn fetch_candidates(&self, track: &TrackIdentity) -> Result<Candidates, ProviderError> {
        let search = request(Self::search_url(track), &REFERER, SEARCH_TIMEOUT);
        let search_body = success_body(&self.http, search).await?;
        let songs = Self::select_songs(&search_body, track)?;
        let lyrics = songs.into_iter().map(|song| self.song_hit(track, song));
        Candidates::collect(join_all(lyrics).await)
    }

    /// 一首歌的命中；沒有原文歌詞時為 `None`。
    async fn song_hit(
        &self,
        track: &TrackIdentity,
        song: SelectedSong,
    ) -> Result<Option<LyricsHit>, ProviderError> {
        let lyric = request(Self::lyric_url(song.id), &REFERER, LYRIC_TIMEOUT);
        let lyric_body = success_body(&self.http, lyric).await?;
        let lyrics = Self::parse_lyric(&lyric_body)?;
        Ok(lyrics.and_then(|lyrics| {
            let mut hit =
                LyricsHit::new(lyrics, Self::song_url(song.id)).matched(track, song.seconds)?;
            hit.artist_matches = song.artist_matches;
            Some(hit)
        }))
    }
}

fn is_placeholder(lyrics: &Lyrics) -> bool {
    let text = match lyrics {
        Lyrics::Synced(lines) => match lines.as_slice() {
            [only] => only.text.as_str(),
            _ => return false,
        },
        Lyrics::Plain(plain) => plain.as_str(),
    };
    let trimmed = text.trim_matches(char::is_whitespace);
    PLACEHOLDERS
        .iter()
        .any(|p| TrackIdentity::loosely_matches(p, trimmed))
}

impl LyricsProvider for NetEaseProvider {
    fn id(&self) -> &str {
        "com.music163"
    }

    fn fetch<'a>(
        &'a self,
        track: &'a TrackIdentity,
    ) -> BoxFuture<'a, Result<Candidates, ProviderError>> {
        Box::pin(self.fetch_candidates(track))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpError;
    use crate::lyrics::provider::test_support::*;
    use crate::lyrics::LyricLine;
    use crate::testing::FakeHttp;
    use futures_util::FutureExt;
    use std::collections::BTreeMap;

    const TWO_LINE_LYRIC: &str = r#"{"lrc":{"lyric":"[00:01.00]a\n[00:02.00]b"}}"#;

    /// `Candidates` 裡最好的一筆。
    fn fetch_best(
        provider: &NetEaseProvider,
        track: &TrackIdentity,
    ) -> Result<Option<LyricsHit>, ProviderError> {
        block_on(provider.fetch(track)).map(|found| best_of(found.hits))
    }

    fn nothing() -> Result<Candidates, ProviderError> {
        Ok(Candidates {
            hits: Vec::new(),
            complete: true,
        })
    }

    fn track() -> TrackIdentity {
        TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: Some("Al".into()),
            duration: Some(200.0),
        }
    }

    fn query_track(title: &str, artist: &str, duration: Option<f64>) -> TrackIdentity {
        TrackIdentity {
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration,
        }
    }

    fn synced1(time: f64, text: &str) -> Lyrics {
        Lyrics::Synced(vec![LyricLine {
            time,
            text: text.into(),
        }])
    }

    fn search_json(songs: &str) -> String {
        format!(r#"{{"code":200,"result":{{"songCount":1,"songs":[{songs}]}}}}"#)
    }

    fn setup() -> (Arc<FakeHttp>, NetEaseProvider) {
        let http = Arc::new(FakeHttp::new());
        let provider = NetEaseProvider::new(http.clone());
        (http, provider)
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // MARK: NetEase decoding

    #[test]
    fn net_ease_decodes_lrc_field() {
        let json = br#"{"lrc":{"lyric":"[00:02.00]hello"}}"#;
        assert_eq!(
            NetEaseProvider::decode_lyric(json),
            Some(synced1(2.0, "hello"))
        );
    }

    #[test]
    fn net_ease_ignores_translation_so_it_cannot_shadow_the_original() {
        let json = r#"{"lrc":{"lyric":"[00:02.00]original"},"tlyric":{"lyric":"[00:02.00]翻譯"}}"#;
        assert_eq!(
            NetEaseProvider::decode_lyric(json.as_bytes()),
            Some(synced1(2.0, "original"))
        );
    }

    #[test]
    fn net_ease_empty_lyric_yields_nil() {
        assert_eq!(
            NetEaseProvider::decode_lyric(br#"{"lrc":{"lyric":""}}"#),
            None
        );
    }

    #[test]
    fn net_ease_missing_lrc_yields_nil() {
        assert_eq!(NetEaseProvider::decode_lyric(b"{}"), None);
        assert_eq!(NetEaseProvider::decode_lyric(br#"{"nolyric":true}"#), None);
    }

    // MARK: NetEase song selection

    #[test]
    fn net_ease_matches_simplified_title_for_traditional_query() {
        let query = query_track("後來", "劉若英", Some(200.0));
        let json = search_json(r#"{"id":7,"name":"后来","duration":200000}"#);
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &query),
            Ok(Some(7))
        );
    }

    #[test]
    fn net_ease_does_not_fall_back_to_first_song_when_no_title_matches() {
        let json = search_json(
            r#"{"id":1,"name":"Completely Different","duration":200000},
        {"id":2,"name":"Another One","duration":200000}"#,
        );
        let wanted = TrackIdentity {
            title: "Wanted".into(),
            ..track()
        };
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &wanted),
            Ok(None)
        );
    }

    #[test]
    fn net_ease_treats_duration_as_milliseconds() {
        let json = search_json(
            r#"{"id":1,"name":"T","duration":400000},
        {"id":2,"name":"T","duration":203000}"#,
        );
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &track()),
            Ok(Some(2))
        );
    }

    #[test]
    fn net_ease_skips_duration_check_when_either_side_has_none() {
        let json = search_json(
            r#"{"id":1,"name":"T","duration":0},{"id":2,"name":"T","duration":900000}"#,
        );
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &track()),
            Ok(Some(1))
        );

        let unknown_length = query_track("T", "A", None);
        let long = search_json(r#"{"id":3,"name":"T","duration":900000}"#);
        assert_eq!(
            NetEaseProvider::select_song_id(long.as_bytes(), &unknown_length),
            Ok(Some(3))
        );
    }

    #[test]
    fn net_ease_selection_compares_title_without_featuring_suffix() {
        let query = query_track("Song (feat. X)", "A", None);
        let json = search_json(r#"{"id":4,"name":"Song","duration":0}"#);
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &query),
            Ok(Some(4))
        );
    }

    #[test]
    fn net_ease_selection_with_no_results_yields_nil() {
        assert_eq!(
            NetEaseProvider::select_song_id(br#"{"code":200,"result":{"songCount":0}}"#, &track()),
            Ok(None)
        );
        assert_eq!(
            NetEaseProvider::select_song_id(br#"{"code":200,"result":{"songs":[]}}"#, &track()),
            Ok(None)
        );
    }

    #[test]
    fn net_ease_selection_rejects_garbled_and_refused_responses() {
        assert!(NetEaseProvider::select_song_id(b"<html/>", &track()).is_err());
        assert!(
            NetEaseProvider::select_song_id(br#"{"code":-460,"msg":"Cheating"}"#, &track())
                .is_err()
        );
    }

    #[test]
    fn net_ease_selection_rejects_response_with_neither_code_nor_result() {
        assert!(NetEaseProvider::select_song_id(b"{}", &track()).is_err());
        assert!(NetEaseProvider::select_song_id(br#"{"msg":"hi"}"#, &track()).is_err());
    }

    #[test]
    fn net_ease_instrumental_placeholder_yields_nil() {
        for placeholder in [
            "纯音乐，请欣赏",
            "纯音乐, 请欣赏",
            "暂无歌词",
            "没有歌词",
            "純音樂，請欣賞",
            "  暫無歌詞 ",
        ] {
            let json = format!(r#"{{"lrc":{{"lyric":"[00:00.00]{placeholder}"}}}}"#);
            assert_eq!(
                NetEaseProvider::decode_lyric(json.as_bytes()),
                None,
                "{placeholder}"
            );
        }
        assert_eq!(
            NetEaseProvider::decode_lyric(r#"{"lrc":{"lyric":"纯音乐，请欣赏"}}"#.as_bytes()),
            None
        );
    }

    #[test]
    fn net_ease_real_single_line_lyric_is_not_dropped() {
        let json = br#"{"lrc":{"lyric":"[00:05.00]Just one real line"}}"#;
        assert_eq!(
            NetEaseProvider::decode_lyric(json),
            Some(synced1(5.0, "Just one real line"))
        );
    }

    #[test]
    fn net_ease_placeholder_on_later_line_of_longer_lyric_is_kept() {
        let json = r#"{"lrc":{"lyric":"[00:01.00]real\n[00:02.00]暂无歌词"}}"#;
        assert_eq!(
            NetEaseProvider::decode_lyric(json.as_bytes()),
            Some(Lyrics::Synced(vec![
                LyricLine {
                    time: 1.0,
                    text: "real".into()
                },
                LyricLine {
                    time: 2.0,
                    text: "暂无歌词".into()
                },
            ]))
        );
    }

    #[test]
    fn net_ease_refused_lyric_response_throws_through_fetch() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":1,"name":"T","duration":200000}"#),
        );
        http.respond(
            NetEaseProvider::lyric_url(1),
            200,
            r#"{"code":-460,"msg":"Cheating"}"#,
        );

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn net_ease_lyric_response_with_code200_is_accepted() {
        let json = br#"{"code":200,"lrc":{"lyric":"[00:02.00]hello"}}"#;
        assert!(NetEaseProvider::decode_lyric(json).is_some());
    }

    // MARK: NetEase requests and error semantics

    #[test]
    fn net_ease_fetch_searches_then_fetches_original_lyric_with_headers() {
        let (http, provider) = setup();
        let query = query_track("晴天", "周杰倫", Some(200.0));
        http.respond(
            NetEaseProvider::search_url(&query),
            200,
            search_json(r#"{"id":555,"name":"晴天","duration":200000}"#),
        );
        http.respond(
            NetEaseProvider::lyric_url(555),
            200,
            r#"{"lrc":{"lyric":"[00:03.00]original"},"tlyric":{"lyric":"[00:03.00]translated"}}"#,
        );

        let result = fetch_best(&provider, &query).unwrap();

        assert_eq!(result.map(|h| h.lyrics), Some(synced1(3.0, "original")));
        let requests = http.requests();
        let hosts: Vec<&str> = requests.iter().map(|r| host(&r.url)).collect();
        assert_eq!(hosts, ["music.163.com", "music.163.com"]);
        let paths: Vec<&str> = requests.iter().map(|r| path(&r.url)).collect();
        assert_eq!(paths, ["/api/search/get/web", "/api/song/lyric"]);
        let first = query_values(&requests[0].url);
        assert_eq!(first["s"], "晴天 周杰倫");
        assert_eq!(first["type"], "1");
        assert_eq!(first["limit"], "5");
        assert!(requests[0].url.contains("%E6%99%B4%E5%A4%A9"));
        assert_eq!(
            query_values(&requests[1].url),
            map(&[("id", "555"), ("lv", "1"), ("kv", "1"), ("tv", "-1")])
        );
        for request in &requests {
            assert_eq!(header(request, "Referer"), Some("https://music.163.com/"));
            assert_eq!(header(request, "User-Agent"), Some("MusiCat"));
        }
        assert_eq!(requests[0].timeout, Duration::from_millis(2_500));
        assert_eq!(requests[1].timeout, Duration::from_secs(2));
    }

    #[test]
    fn net_ease_no_title_match_returns_nil_and_skips_lyric_request() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":1,"name":"Other","duration":200000}"#),
        );

        assert_eq!(block_on(provider.fetch(&t)), nothing());
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn net_ease_song_without_lyric_returns_nil() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":1,"name":"T","duration":200000}"#),
        );
        http.respond(NetEaseProvider::lyric_url(1), 200, r#"{"nolyric":true}"#);

        assert_eq!(block_on(provider.fetch(&t)), nothing());
    }

    #[test]
    fn net_ease_server_error_on_search_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(NetEaseProvider::search_url(&t), 502, "");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(502))
        );
    }

    #[test]
    fn net_ease_server_error_on_lyric_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":1,"name":"T","duration":200000}"#),
        );
        http.respond(NetEaseProvider::lyric_url(1), 500, "");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(500))
        );
    }

    #[test]
    fn net_ease_garbled_lyric_body_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":1,"name":"T","duration":200000}"#),
        );
        http.respond(NetEaseProvider::lyric_url(1), 200, "not json");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn net_ease_transport_error_propagates() {
        let (http, provider) = setup();
        let t = track();
        http.fail(
            NetEaseProvider::search_url(&t),
            HttpError::Transport("timedOut".into()),
        );

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::Transport("timedOut".into()))
        );
    }

    // MARK: extras for the Rust port (lenient and strict JSON handling)

    #[test]
    fn net_ease_tolerates_null_extra_and_float_fields() {
        let json = search_json(
            r#"{"id":9,"name":null,"duration":null},
        {"id":10.0,"name":"T","duration":200000.0,"artists":[{"name":"A"}],"alias":null}"#,
        );
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &track()),
            Ok(Some(10))
        );
        let json = r#"{"code":200.0,"result":null}"#;
        assert_eq!(
            NetEaseProvider::select_song_id(json.as_bytes(), &track()),
            Ok(None)
        );
    }

    #[test]
    fn net_ease_song_without_id_or_with_non_integer_id_is_a_bad_response() {
        for songs in [
            r#"{"name":"T","duration":200000}"#,
            r#"{"id":1.5,"name":"T"}"#,
            r#"{"id":"1","name":"T"}"#,
        ] {
            let json = search_json(songs);
            assert_eq!(
                NetEaseProvider::select_song_id(json.as_bytes(), &track()),
                Err(ProviderError::BadResponse),
                "{songs}"
            );
        }
    }

    #[test]
    fn net_ease_non_200_code_on_lyric_response_is_a_bad_response_even_with_lrc() {
        let json = br#"{"code":404,"lrc":{"lyric":"[00:02.00]hello"}}"#;
        assert_eq!(
            NetEaseProvider::parse_lyric(json),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn netease_hit_url_points_at_the_song_page() {
        let (http, provider) = setup();
        let query = query_track("T", "A", Some(200.0));
        http.respond(
            NetEaseProvider::search_url(&query),
            200,
            search_json(r#"{"id":7,"name":"T","duration":200000}"#),
        );
        http.respond(
            NetEaseProvider::lyric_url(7),
            200,
            r#"{"lrc":{"lyric":"[00:01.00]a\n[00:02.00]b"}}"#,
        );

        let hit = fetch_best(&provider, &query).unwrap().unwrap();

        assert_eq!(hit.url, "https://music.163.com/song?id=7");
        assert_eq!(NetEaseProvider::song_url(7), hit.url);
        assert!(hit.lyrics.is_synced());
    }

    /// 挑中的歌曲編號與曲長（秒）。
    fn picked(json: &str) -> Result<Vec<(i64, Option<f64>)>, ProviderError> {
        NetEaseProvider::select_songs(json.as_bytes(), &track())
            .map(|songs| songs.iter().map(|s| (s.id, s.seconds)).collect())
    }

    #[test]
    fn net_ease_orders_close_songs_ahead_of_loose_ones() {
        let mixed = search_json(
            r#"{"id":1,"name":"T","duration":208000},{"id":2,"name":"T","duration":201000},
            {"id":6,"name":"T","duration":207000},{"id":7,"name":"T","duration":209000}"#,
        );
        assert_eq!(
            picked(&mixed),
            Ok(vec![(2, Some(201.0)), (1, Some(208.0)), (6, Some(207.0))])
        );
        let loose_only = search_json(
            r#"{"id":3,"name":"T","duration":209000},{"id":4,"name":"T","duration":207000}"#,
        );
        assert_eq!(
            picked(&loose_only),
            Ok(vec![(3, Some(209.0)), (4, Some(207.0))])
        );
        let far = search_json(r#"{"id":5,"name":"T","duration":211000}"#);
        assert_eq!(picked(&far), Ok(Vec::new()));
    }

    #[test]
    fn net_ease_marks_a_loose_duration_hit() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(r#"{"id":8,"name":"T","duration":208000}"#),
        );
        http.respond(
            NetEaseProvider::lyric_url(8),
            200,
            r#"{"code":200,"lrc":{"lyric":"[00:01.00]a\n[00:02.00]b"}}"#,
        );

        let hit = fetch_best(&provider, &t).unwrap().unwrap();

        assert!(hit.loose_duration);
        assert_eq!(hit.duration_gap, Some(8.0));
    }

    // MARK: every eligible candidate (Rust only)

    #[test]
    fn net_ease_fetches_up_to_three_eligible_songs_close_ones_first() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(
                r#"{"id":1,"name":"T","duration":208000},{"id":2,"name":"T","duration":200000},
                {"id":3,"name":"T","duration":201000},{"id":4,"name":"T","duration":202000},
                {"id":5,"name":"Other","duration":200000}"#,
            ),
        );
        for id in 1..=5 {
            http.respond(NetEaseProvider::lyric_url(id), 200, TWO_LINE_LYRIC);
        }

        let found = block_on(provider.fetch(&t)).unwrap();

        let lyric_requests: Vec<String> =
            http.requests().into_iter().skip(1).map(|r| r.url).collect();
        assert_eq!(lyric_requests, [2, 3, 4].map(NetEaseProvider::lyric_url));
        assert_eq!(urls(&found.hits), [2, 3, 4].map(NetEaseProvider::song_url));
        assert!(found.complete);
    }

    #[test]
    fn net_ease_a_failed_lyric_request_leaves_the_others() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(
                r#"{"id":1,"name":"T","duration":200000},{"id":2,"name":"T","duration":201000}"#,
            ),
        );
        http.respond(NetEaseProvider::lyric_url(1), 500, "");
        http.respond(NetEaseProvider::lyric_url(2), 200, TWO_LINE_LYRIC);

        let found = block_on(provider.fetch(&t)).unwrap();

        assert_eq!(urls(&found.hits), [NetEaseProvider::song_url(2)]);
        assert!(!found.complete);

        // 其他首歌乾淨地沒有歌詞時，失敗的那個請求讓整次查詢算失敗。
        http.respond(NetEaseProvider::lyric_url(2), 200, r#"{"nolyric":true}"#);
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(500))
        );
    }

    #[test]
    fn net_ease_requests_the_lyrics_without_waiting_for_each_other() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            search_json(
                r#"{"id":1,"name":"T","duration":200000},{"id":2,"name":"T","duration":201000},
                {"id":3,"name":"T","duration":202000}"#,
            ),
        );
        for id in 1..=3 {
            http.hang(NetEaseProvider::lyric_url(id));
        }

        assert!(provider.fetch(&t).now_or_never().is_none());

        assert_eq!(http.requests().len(), 4);
    }

    // MARK: song artists (Rust only)

    /// 搜尋回 `songs`、每首歌都有歌詞；回傳（歌詞請求的歌曲順序，各命中的演出者是否相符）。
    fn fetch_by_artist(songs: &str, ids: &[i64]) -> (Vec<String>, Vec<bool>) {
        let (http, provider) = setup();
        let t = query_track("T", "周杰倫 Jay Chou", Some(200.0));
        http.respond(NetEaseProvider::search_url(&t), 200, search_json(songs));
        for &id in ids {
            http.respond(NetEaseProvider::lyric_url(id), 200, TWO_LINE_LYRIC);
        }
        let found = block_on(provider.fetch(&t)).unwrap();
        (
            http.requests().into_iter().skip(1).map(|r| r.url).collect(),
            found.hits.iter().map(|h| h.artist_matches).collect(),
        )
    }

    fn song_by(id: i64, millis: u32, artists: &str) -> String {
        format!(r#"{{"id":{id},"name":"T","duration":{millis},"artists":{artists}}}"#)
    }

    #[test]
    fn net_ease_takes_artist_matching_songs_first_then_close_before_loose() {
        let other = r#"[{"id":9,"name":"恶童ETON"}]"#;
        let jay = r#"[{"id":6452,"name":"周杰伦"}]"#;
        let songs = [
            song_by(1, 200_000, other),
            song_by(2, 208_000, jay),
            song_by(3, 208_000, other),
            song_by(4, 201_000, jay),
            song_by(5, 202_000, jay),
        ]
        .join(",");

        let (requests, matched) = fetch_by_artist(&songs, &[1, 2, 3, 4, 5]);

        assert_eq!(requests, [4, 5, 2].map(NetEaseProvider::lyric_url));
        assert_eq!(matched, [true, true, true]);

        // 相符的不夠三首時由不相符的補上，一樣是曲長吻合的優先。
        let songs = [
            song_by(1, 208_000, other),
            song_by(2, 200_000, other),
            song_by(3, 208_000, jay),
        ]
        .join(",");
        let (requests, matched) = fetch_by_artist(&songs, &[1, 2, 3]);
        assert_eq!(requests, [3, 2, 1].map(NetEaseProvider::lyric_url));
        assert_eq!(matched, [true, false, false]);
    }

    #[test]
    fn net_ease_matches_any_of_the_song_artists() {
        let songs = [
            song_by(1, 200_000, r#"[{"name":"费玉清"},{"name":"周杰伦"}]"#),
            song_by(2, 200_000, r#"[{"name":"费玉清"},{"name":"陈奕迅"}]"#),
        ]
        .join(",");

        let (_, matched) = fetch_by_artist(&songs, &[1, 2]);

        assert_eq!(matched, [true, false]);
    }

    #[test]
    fn net_ease_song_without_artist_information_counts_as_matching() {
        for artists in [
            "null",
            "[]",
            "[null]",
            "[{}]",
            r#"[{"name":null}]"#,
            r#"[{"name":""}]"#,
        ] {
            let (_, matched) = fetch_by_artist(&song_by(1, 200_000, artists), &[1]);
            assert_eq!(matched, [true], "{artists}");
        }
        let (_, matched) = fetch_by_artist(r#"{"id":1,"name":"T","duration":200000}"#, &[1]);
        assert_eq!(matched, [true]);
    }
}
