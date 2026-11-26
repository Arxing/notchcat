//! LRCLIB：免費、免金鑰、社群維護的 LRC 資料庫（移植自 Swift `LRCLIBProvider`）。
//!
//! 精確查詢、原文搜尋、簡體搜尋（歌名或演出者轉簡體後有差異時）同時發出，回傳所有合格的紀錄。
//! 只靠精確查詢不夠：它可能只因為系統回報的標題少了資料庫帶有的 `(feat. …)` 後綴而落空，
//! 而且同一首歌常有好幾筆時間軸不同的紀錄。精確端點回 404 是乾淨的「沒有」；
//! 其他非 2xx 或看不懂的回應算那個請求失敗（見 [`Candidates`]）。
//!
//! 搜尋的紀錄會標記演出者是否和曲目相符（`LyricsHit::artist_matches`）；精確查詢的演出者由服務端比對，
//! 一律算相符。要不要丟掉不相符的由 `LyricsRepository` 決定。

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{join, join_all};
use serde::{Deserialize, Deserializer};

use super::provider::{
    build_url, candidate_title, decode_array, decode_object, is_success, request, send,
    success_body, Candidates, LyricsProvider, ProviderError,
};
use super::{parse_lrc, Lyrics, LyricsHit, TrackIdentity};
use crate::http::HttpClient;
use crate::runtime::BoxFuture;

/// 每個請求的逾時；逾時視同該請求失敗。各請求同時發出，只要比來源上限
/// （`RepositoryConfig::provider_timeout`）短，一個卡住的請求就不會連累其他請求已到手的紀錄。
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);

pub struct LrclibProvider {
    http: Arc<dyn HttpClient>,
}

/// 缺欄位、`null`、多餘欄位都容忍；型別不對（例如 `duration` 是字串）視為壞回應。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    /// 紀錄編號，只用來組來源網址；怪異的型別或值一律視為沒有，絕不讓整筆回應變成壞回應。
    #[serde(default, deserialize_with = "lenient_positive_id")]
    id: Option<i64>,
    track_name: Option<String>,
    artist_name: Option<String>,
    duration: Option<f64>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

/// 正整數才是 `Some`（也接受小數部分為零的浮點，如 `12.0`）；缺漏、`null`、0、負數、字串、
/// `7.5`、超出 `i64` 的值、布林、陣列、物件一律是 `None`，而且不是錯誤。
fn lenient_positive_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let serde_json::Value::Number(n) = value else {
        return Ok(None);
    };
    let id = n.as_i64().or_else(|| {
        n.as_f64()
            // `i64::MAX as f64` 是 2^63：嚴格小於它的整數值浮點才轉得進 `i64`。
            .filter(|f| f.fract() == 0.0 && *f >= 1.0 && *f < i64::MAX as f64)
            .map(|f| f as i64)
    });
    Ok(id.filter(|id| *id > 0))
}

impl LrclibProvider {
    pub fn new(http: Arc<dyn HttpClient>) -> Self {
        Self { http }
    }

    pub(crate) fn exact_url(track: &TrackIdentity) -> String {
        let n = track.normalized();
        let duration;
        let mut query: Vec<(&str, &str)> =
            vec![("track_name", &n.title), ("artist_name", &n.artist)];
        if let Some(album) = n.album.as_deref().filter(|a| !a.is_empty()) {
            query.push(("album_name", album));
        }
        if let Some(d) = n.duration.filter(|d| *d > 0.0) {
            duration = (d.round() as i64).to_string();
            query.push(("duration", &duration));
        }
        build_url("https://lrclib.net/api/get", &query)
    }

    pub(crate) fn search_url(track: &TrackIdentity) -> String {
        let n = track.normalized();
        let q = format!("{} {}", n.title, n.artist);
        build_url("https://lrclib.net/api/search", &[("q", &q)])
    }

    /// 有紀錄編號時指向該紀錄；否則退回產生這筆命中的請求位址。
    fn record_url(id: Option<i64>, request_url: &str) -> String {
        match id {
            Some(id) => format!("https://lrclib.net/api/get/{id}"),
            None => request_url.to_string(),
        }
    }

    /// 單筆紀錄的歌詞；沒有歌詞或長度不符時為 `None`（壞掉的本體也是 `None`）。
    #[cfg(test)]
    fn decode(data: &[u8], track: &TrackIdentity) -> Option<Lyrics> {
        let payload = decode_object::<Payload>(data).ok()?;
        Self::payload_hit(&payload, track, &Self::exact_url(track)).map(|hit| hit.lyrics)
    }

    /// 搜尋結果的候選裡最好的一筆。
    #[cfg(test)]
    fn decode_search(data: &[u8], track: &TrackIdentity) -> Option<Lyrics> {
        let payloads = decode_array::<Payload>(data).ok()?;
        let hits = Self::search_candidates(&payloads, track, track);
        super::provider::test_support::best_of(hits).map(|hit| hit.lyrics)
    }

    /// 精確查詢、原文搜尋與簡體搜尋同時發出，收齊所有合格的紀錄。
    async fn fetch_candidates(&self, track: &TrackIdentity) -> Result<Candidates, ProviderError> {
        let simplified = track.simplified_chinese();
        let queries = std::iter::once(track).chain(simplified.as_ref());
        let (exact, searches) = join(
            self.exact(track),
            join_all(queries.map(|query| self.search(query, track))),
        )
        .await;
        Candidates::collect(std::iter::once(exact).chain(searches))
    }

    /// 精確查詢的候選（至多一筆）。
    async fn exact(&self, track: &TrackIdentity) -> Result<Vec<LyricsHit>, ProviderError> {
        let url = Self::exact_url(track);
        let exact = request(url.clone(), &[], REQUEST_TIMEOUT);
        let (status, body) = send(&self.http, exact).await?;
        if status == 404 {
            return Ok(Vec::new());
        }
        if !is_success(status) {
            return Err(ProviderError::BadStatus(status));
        }
        let payload = decode_object::<Payload>(&body)?;
        Ok(Self::payload_hit(&payload, track, &url)
            .into_iter()
            .collect())
    }

    /// 以 `query`（原本的曲目或它的簡體形式）搜尋；`track` 是原本的曲目。
    async fn search(
        &self,
        query: &TrackIdentity,
        track: &TrackIdentity,
    ) -> Result<Vec<LyricsHit>, ProviderError> {
        let search = request(Self::search_url(query), &[], REQUEST_TIMEOUT);
        let found = success_body(&self.http, search).await?;
        let payloads = decode_array::<Payload>(&found)?;
        Ok(Self::search_candidates(&payloads, query, track))
    }

    /// 搜尋是模糊的，可能回來另一首歌、混音或翻唱；不像精確查詢（伺服器自己比對名稱），
    /// 每個候選都必須帶有與查詢寬鬆相符的歌名。寧可沒有歌詞，也不要錯的歌詞。
    ///
    /// 命中的網址是候選自己的紀錄頁；候選沒有可用編號時退回搜尋請求的位址。
    /// 歌名相符不代表是同一首歌，所以每筆命中另外標記演出者是否相符；比的是原本曲目 `track` 的演出者，
    /// 簡體查詢 `query` 可能已經拿掉英文名。
    fn search_candidates(
        payloads: &[Payload],
        query: &TrackIdentity,
        track: &TrackIdentity,
    ) -> Vec<LyricsHit> {
        let wanted = query.normalized().title;
        let search_url = Self::search_url(query);
        payloads
            .iter()
            .filter(|payload| {
                payload.track_name.as_deref().is_some_and(|name| {
                    !name.is_empty()
                        && TrackIdentity::loosely_matches(&candidate_title(name), &wanted)
                })
            })
            .filter_map(|payload| {
                let mut hit = Self::payload_hit(payload, query, &search_url)?;
                hit.artist_matches = track.matches_artist(payload.artist_name.as_deref());
                Some(hit)
            })
            .collect()
    }

    /// 一筆紀錄的命中；曲長差超過放寬的容忍（雙方都回報正的長度時才比）或沒有歌詞時為 `None`。
    fn payload_hit(
        payload: &Payload,
        track: &TrackIdentity,
        request_url: &str,
    ) -> Option<LyricsHit> {
        let lyrics = Self::payload_lyrics(payload)?;
        let url = Self::record_url(payload.id, request_url);
        LyricsHit::new(lyrics, url).matched(track, payload.duration)
    }

    /// 帶時間軸的優先於純文字。
    fn payload_lyrics(payload: &Payload) -> Option<Lyrics> {
        if let Some(parsed) = payload.synced_lyrics.as_deref().and_then(parse_lrc) {
            if parsed.is_synced() {
                return Some(parsed);
            }
        }
        match payload.plain_lyrics.as_deref() {
            Some(plain) if !plain.trim_matches(char::is_whitespace).is_empty() => {
                Some(Lyrics::Plain(plain.to_string()))
            }
            _ => None,
        }
    }
}

impl LyricsProvider for LrclibProvider {
    fn id(&self) -> &str {
        "net.lrclib"
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

    /// `Candidates` 裡最好的一筆。
    fn fetch_best(
        provider: &LrclibProvider,
        track: &TrackIdentity,
    ) -> Result<Option<LyricsHit>, ProviderError> {
        block_on(provider.fetch(track)).map(|found| best_of(found.hits))
    }

    fn track() -> TrackIdentity {
        TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: Some("Al".into()),
            duration: Some(200.0),
        }
    }

    fn unknown_length() -> TrackIdentity {
        TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: None,
            duration: None,
        }
    }

    fn synced(time: f64, text: &str) -> Lyrics {
        Lyrics::Synced(vec![LyricLine {
            time,
            text: text.into(),
        }])
    }

    fn setup() -> (Arc<FakeHttp>, LrclibProvider) {
        let http = Arc::new(FakeHttp::new());
        let provider = LrclibProvider::new(http.clone());
        (http, provider)
    }

    // MARK: LRCLIB decoding

    #[test]
    fn lrclib_prefers_synced_field_over_plain() {
        let json = br#"{"trackName":"T","artistName":"A","duration":200,
         "plainLyrics":"words","syncedLyrics":"[00:01.00]line"}"#;
        assert_eq!(
            LrclibProvider::decode(json, &track()),
            Some(synced(1.0, "line"))
        );
    }

    #[test]
    fn lrclib_falls_back_to_plain_when_synced_absent() {
        let json = br#"{"trackName":"T","artistName":"A","duration":200,"plainLyrics":"words"}"#;
        assert_eq!(
            LrclibProvider::decode(json, &track()),
            Some(Lyrics::Plain("words".into()))
        );
    }

    #[test]
    fn lrclib_rejects_mismatched_duration() {
        let json =
            br#"{"trackName":"T","artistName":"A","duration":400,"syncedLyrics":"[00:01.00]line"}"#;
        assert_eq!(LrclibProvider::decode(json, &track()), None);
    }

    #[test]
    fn lrclib_accepts_duration_within_tolerance() {
        let json =
            br#"{"trackName":"T","artistName":"A","duration":203,"syncedLyrics":"[00:01.00]line"}"#;
        assert!(LrclibProvider::decode(json, &track()).is_some());
    }

    #[test]
    fn lrclib_accepts_result_when_track_has_no_duration() {
        let json =
            br#"{"trackName":"T","artistName":"A","duration":400,"syncedLyrics":"[00:01.00]line"}"#;
        assert!(LrclibProvider::decode(json, &unknown_length()).is_some());
    }

    #[test]
    fn lrclib_search_picks_first_valid_candidate() {
        let json = br#"[{"trackName":"T","duration":400,"syncedLyrics":"[00:01.00]wrong length"},
         {"trackName":"T","duration":200,"syncedLyrics":"","plainLyrics":"  "},
         {"trackName":"T","duration":201,"syncedLyrics":"[00:01.00]second"},
         {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]third"}]"#;
        assert_eq!(
            LrclibProvider::decode_search(json, &track()),
            Some(synced(1.0, "second"))
        );
    }

    #[test]
    fn lrclib_search_rejects_candidate_with_wrong_title() {
        let json =
            br#"[{"trackName":"Another Song","duration":200,"syncedLyrics":"[00:01.00]wrong"}]"#;
        assert_eq!(LrclibProvider::decode_search(json, &track()), None);
    }

    #[test]
    fn lrclib_search_rejects_wrong_title_when_track_has_no_duration() {
        let json =
            br#"[{"trackName":"Another Song","duration":200,"syncedLyrics":"[00:01.00]wrong"}]"#;
        assert_eq!(LrclibProvider::decode_search(json, &unknown_length()), None);
    }

    #[test]
    fn lrclib_search_rejects_candidate_without_track_name() {
        let json = br#"[{"duration":200,"syncedLyrics":"[00:01.00]no name"},
         {"trackName":"","duration":200,"syncedLyrics":"[00:01.00]empty name"}]"#;
        assert_eq!(LrclibProvider::decode_search(json, &track()), None);
    }

    #[test]
    fn lrclib_search_accepts_simplified_variant_of_traditional_title() {
        let query = TrackIdentity {
            title: "後來".into(),
            artist: "劉若英".into(),
            album: None,
            duration: None,
        };
        let json = r#"[{"trackName":"后来","duration":250,"syncedLyrics":"[00:01.00]line"}]"#;
        assert_eq!(
            LrclibProvider::decode_search(json.as_bytes(), &query),
            Some(synced(1.0, "line"))
        );
    }

    #[test]
    fn lrclib_search_accepts_candidate_title_with_featuring_suffix() {
        let json =
            br#"[{"trackName":"T (feat. X)","duration":200,"syncedLyrics":"[00:01.00]line"}]"#;
        assert!(LrclibProvider::decode_search(json, &track()).is_some());
    }

    #[test]
    fn lrclib_search_skips_earlier_wrong_title_and_picks_first_matching_title() {
        let json = br#"[{"trackName":"T (Remix Of Something Else)","duration":200,"syncedLyrics":"[00:01.00]remix"},
         {"trackName":"Other","duration":200,"syncedLyrics":"[00:01.00]other"},
         {"trackName":"t","duration":200,"syncedLyrics":"[00:01.00]right"},
         {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]later"}]"#;
        assert_eq!(
            LrclibProvider::decode_search(json, &track()),
            Some(synced(1.0, "right"))
        );
    }

    #[test]
    fn lrclib_search_with_no_valid_candidate_yields_nil() {
        assert_eq!(LrclibProvider::decode_search(b"[]", &track()), None);
    }

    // MARK: LRCLIB requests and error semantics

    #[test]
    fn lrclib_exact_lookup_sends_normalized_parameters_and_user_agent() {
        let (http, provider) = setup();
        let query = TrackIdentity {
            title: "Song (feat. X)".into(),
            artist: "A & B".into(),
            album: Some("Al".into()),
            duration: Some(200.0),
        };
        http.respond(
            LrclibProvider::exact_url(&query),
            200,
            r#"{"duration":200,"syncedLyrics":"[00:01.00]hit"}"#,
        );
        http.respond(LrclibProvider::search_url(&query), 200, "[]");

        let result = fetch_best(&provider, &query).unwrap();

        assert_eq!(result.map(|h| h.lyrics), Some(synced(1.0, "hit")));
        let requests = http.requests();
        assert_eq!(requests.len(), 2, "精確查詢與一次搜尋");
        let request = &requests[0];
        assert_eq!(host(&request.url), "lrclib.net");
        assert_eq!(path(&request.url), "/api/get");
        let expected: std::collections::BTreeMap<String, String> = [
            ("track_name", "Song"),
            ("artist_name", "A"),
            ("album_name", "Al"),
            ("duration", "200"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(query_values(&request.url), expected);
        assert_eq!(header(request, "User-Agent"), Some("MusiCat"));
        assert_eq!(request.timeout, Duration::from_secs(4));
    }

    #[test]
    fn lrclib_omits_duration_and_album_when_unknown() {
        let (http, provider) = setup();
        let a = unknown_length();
        let b = TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: Some(String::new()),
            duration: Some(0.0),
        };
        for q in [&a, &b] {
            http.respond(
                LrclibProvider::exact_url(q),
                200,
                r#"{"syncedLyrics":"[00:01.00]hit"}"#,
            );
            http.respond(LrclibProvider::search_url(q), 200, "[]");
        }

        let _ = block_on(provider.fetch(&a)).unwrap();
        let _ = block_on(provider.fetch(&b)).unwrap();

        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        let exact: Vec<_> = requests
            .iter()
            .filter(|r| path(&r.url) == "/api/get")
            .collect();
        assert_eq!(exact.len(), 2);
        for request in exact {
            let values = query_values(&request.url);
            assert_eq!(values.len(), 2, "{}", request.url);
            assert_eq!(values["track_name"], "T");
            assert_eq!(values["artist_name"], "A");
        }
    }

    #[test]
    fn lrclib_encodes_cjk_and_reserved_characters_in_query() {
        let (http, provider) = setup();
        let query = TrackIdentity {
            title: "晴天 + Rock&Roll".into(),
            artist: "周杰倫".into(),
            album: None,
            duration: None,
        };
        http.respond(
            LrclibProvider::exact_url(&query),
            200,
            r#"{"syncedLyrics":"[00:01.00]hit"}"#,
        );

        let _ = block_on(provider.fetch(&query)).unwrap();

        let url = http.requests()[0].url.clone();
        assert!(
            url.contains("track_name=%E6%99%B4%E5%A4%A9%20%2B%20Rock%26Roll"),
            "{url}"
        );
        assert!(
            url.contains("artist_name=%E5%91%A8%E6%9D%B0%E5%80%AB"),
            "{url}"
        );
        assert_eq!(query_values(&url)["track_name"], "晴天 + Rock&Roll");
    }

    #[test]
    fn lrclib_exact_404_falls_through_to_search() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, r#"{"code":404}"#);
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            r#"[{"trackName":"T","duration":200,"syncedLyrics":"[00:02.00]found by search"}]"#,
        );

        let found = block_on(provider.fetch(&t)).unwrap();

        assert!(found.complete, "404 是乾淨的「沒有」，不算失敗");
        assert_eq!(
            best_of(found.hits).map(|h| h.lyrics),
            Some(synced(2.0, "found by search"))
        );
        let requests = http.requests();
        let paths: Vec<&str> = requests.iter().map(|r| path(&r.url)).collect();
        assert_eq!(paths, ["/api/get", "/api/search"]);
        let q = query_values(&requests[1].url);
        assert_eq!(q.len(), 1);
        assert_eq!(q["q"], "T A");
        assert_eq!(header(&requests[1], "User-Agent"), Some("MusiCat"));
        assert_eq!(requests[1].timeout, Duration::from_secs(4));
    }

    #[test]
    fn lrclib_exact_hit_rejected_by_duration_also_falls_through_to_search() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            LrclibProvider::exact_url(&t),
            200,
            r#"{"duration":400,"syncedLyrics":"[00:01.00]wrong song"}"#,
        );
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            r#"[{"trackName":"T","duration":199,"syncedLyrics":"[00:02.00]right song"}]"#,
        );

        let result = fetch_best(&provider, &t).unwrap();

        assert_eq!(result.map(|h| h.lyrics), Some(synced(2.0, "right song")));
    }

    #[test]
    fn lrclib_not_found_everywhere_returns_nil_without_throwing() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(LrclibProvider::search_url(&t), 200, "[]");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Ok(Candidates {
                hits: Vec::new(),
                complete: true
            })
        );
    }

    #[test]
    fn lrclib_server_error_on_exact_lookup_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 503, "");
        http.respond(LrclibProvider::search_url(&t), 200, "[]");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(503))
        );
        assert_eq!(http.requests().len(), 2);
    }

    #[test]
    fn lrclib_server_error_on_search_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(LrclibProvider::search_url(&t), 500, "");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(500))
        );
    }

    #[test]
    fn lrclib_garbled_body_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 200, "<html>gateway</html>");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn lrclib_transport_error_propagates() {
        let (http, provider) = setup();
        let t = track();
        http.fail(
            LrclibProvider::exact_url(&t),
            HttpError::Transport("notConnectedToInternet".into()),
        );
        http.respond(LrclibProvider::search_url(&t), 200, "[]");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::Transport("notConnectedToInternet".into()))
        );
        assert_eq!(http.requests().len(), 2);
    }

    // MARK: extras for the Rust port (lenient and strict JSON handling)

    #[test]
    fn lrclib_tolerates_null_extra_and_float_fields() {
        let json =
            br#"{"id":1,"trackName":null,"artistName":null,"albumName":"x","instrumental":false,
         "duration":200.4,"plainLyrics":null,"syncedLyrics":"[00:01.00]line"}"#;
        assert_eq!(
            LrclibProvider::decode(json, &track()),
            Some(synced(1.0, "line"))
        );
    }

    #[test]
    fn lrclib_wrong_field_types_are_a_bad_response_not_a_miss() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            LrclibProvider::exact_url(&t),
            200,
            r#"{"duration":"200","syncedLyrics":"[00:01.00]line"}"#,
        );
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );

        // 搜尋結果中任何一筆壞掉，整個回應都算壞（與 Swift 解碼整個陣列一致）。
        let (http, provider) = setup();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            r#"[{"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]ok"},{"trackName":5}]"#,
        );
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn lrclib_search_response_that_is_an_object_is_a_bad_response() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            r#"{"message":"rate limited"}"#,
        );
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn lrclib_exact_2xx_other_than_200_is_decoded_too() {
        let (http, provider) = setup();
        let t = track();
        http.respond(
            LrclibProvider::exact_url(&t),
            201,
            r#"{"duration":200,"syncedLyrics":"[00:01.00]hit"}"#,
        );
        let hit = fetch_best(&provider, &t).unwrap().unwrap();
        assert_eq!(hit.lyrics, synced(1.0, "hit"));
    }

    #[test]
    fn lrclib_exact_redirect_status_is_a_failure() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 301, "");
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(301))
        );
    }

    // MARK: source URL (LyricsHit)

    const TWO_LINES: &str = "[00:01.00]a\\n[00:02.00]b";

    fn exact_hit(provider_http: &(Arc<FakeHttp>, LrclibProvider), body: &str) -> LyricsHit {
        let t = track();
        provider_http
            .0
            .respond(LrclibProvider::exact_url(&t), 200, body);
        fetch_best(&provider_http.1, &t).unwrap().unwrap()
    }

    #[test]
    fn lrclib_exact_hit_uses_the_record_id_url() {
        let ctx = setup();
        let hit = exact_hit(
            &ctx,
            &format!(r#"{{"id":3396226,"duration":200,"syncedLyrics":"{TWO_LINES}"}}"#),
        );
        assert_eq!(hit.url, "https://lrclib.net/api/get/3396226");
        assert!(hit.lyrics.is_synced());
        assert_eq!(hit.duration_gap, Some(0.0));
    }

    #[test]
    fn lrclib_hit_without_a_usable_id_falls_back_to_the_request_url() {
        let t = track();
        for id in [
            None,
            Some("null"),
            Some("0"),
            Some("-5"),
            Some(r#""abc""#),
            Some(r#""42""#),
            Some("7.5"),
            Some("1e30"),
            Some("-1e30"),
            Some("18446744073709551615"),
            Some("true"),
            Some("[1]"),
            Some(r#"{"a":1}"#),
        ] {
            let field = id.map_or(String::new(), |v| format!(r#""id":{v},"#));
            let ctx = setup();
            let hit = exact_hit(
                &ctx,
                &format!(r#"{{{field}"duration":200,"syncedLyrics":"{TWO_LINES}"}}"#),
            );
            assert_eq!(hit.url, LrclibProvider::exact_url(&t), "id = {id:?}");
        }
    }

    #[test]
    fn lrclib_integral_float_id_is_accepted_but_other_fields_stay_strict() {
        let ctx = setup();
        let hit = exact_hit(
            &ctx,
            &format!(r#"{{"id":12.0,"duration":200,"syncedLyrics":"{TWO_LINES}"}}"#),
        );
        assert_eq!(hit.url, "https://lrclib.net/api/get/12");

        // 寬鬆的只有 id；其他欄位型別不對仍是壞回應。
        let (http, provider) = setup();
        let t = track();
        http.respond(
            LrclibProvider::exact_url(&t),
            200,
            r#"{"id":"x","duration":"200","syncedLyrics":"[00:01.00]a"}"#,
        );
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadResponse)
        );
    }

    #[test]
    fn lrclib_search_fallback_hit_uses_the_candidate_id_or_the_search_url() {
        let t = track();
        let (http, provider) = setup();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            format!(
                r#"[{{"id":7,"trackName":"Other","duration":200,"syncedLyrics":"{TWO_LINES}"}},
                   {{"id":42,"trackName":"T","duration":200,"syncedLyrics":"{TWO_LINES}"}}]"#
            ),
        );
        let hit = fetch_best(&provider, &t).unwrap().unwrap();
        assert_eq!(hit.url, "https://lrclib.net/api/get/42");

        let (http, provider) = setup();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            format!(r#"[{{"id":0,"trackName":"T","duration":200,"syncedLyrics":"{TWO_LINES}"}}]"#),
        );
        let hit = fetch_best(&provider, &t).unwrap().unwrap();
        assert_eq!(hit.url, LrclibProvider::search_url(&t));
    }

    #[test]
    fn lrclib_every_hit_url_is_https() {
        let t = track();
        let ctx = setup();
        let hit = exact_hit(
            &ctx,
            &format!(r#"{{"duration":200,"syncedLyrics":"{TWO_LINES}"}}"#),
        );
        assert!(hit.url.starts_with("https://"));
        assert!(
            LrclibProvider::record_url(Some(1), &LrclibProvider::search_url(&t))
                .starts_with("https://")
        );
        assert!(
            LrclibProvider::record_url(None, &LrclibProvider::search_url(&t))
                .starts_with("https://")
        );
    }

    // MARK: both scripts and loose duration (Rust only)

    fn mv_track() -> TrackIdentity {
        TrackIdentity {
            title: "爸 我回來了".into(),
            artist: "周杰倫 Jay Chou".into(),
            album: None,
            duration: Some(229.6),
        }
    }

    fn record(id: i64, title: &str, duration: f64, text: &str) -> String {
        format!(
            r#"{{"id":{id},"trackName":"{title}","duration":{duration},"syncedLyrics":"[00:01.00]{text}"}}"#
        )
    }

    /// 精確查詢落空，原文與簡體的搜尋各回指定的紀錄。
    fn mv_setup(original: &str, simplified: &str) -> (Arc<FakeHttp>, LrclibProvider) {
        let (http, provider) = setup();
        let t = mv_track();
        let retry = t.simplified_chinese().expect("traditional query");
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(LrclibProvider::search_url(&t), 200, format!("[{original}]"));
        http.respond(
            LrclibProvider::search_url(&retry),
            200,
            format!("[{simplified}]"),
        );
        (http, provider)
    }

    fn fetch_mv(provider: &LrclibProvider) -> Option<(Lyrics, bool)> {
        fetch_best(provider, &mv_track())
            .unwrap()
            .map(|h| (h.lyrics, h.loose_duration))
    }

    #[test]
    fn lrclib_searches_both_scripts_and_finds_a_simplified_only_record() {
        let (http, provider) = mv_setup("", &record(9, "爸 我回来了", 230.0, "found"));

        let hit = fetch_best(&provider, &mv_track()).unwrap().unwrap();

        assert_eq!(hit.lyrics, synced(1.0, "found"));
        assert!(!hit.loose_duration);
        assert_eq!(hit.url, "https://lrclib.net/api/get/9");
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(query_values(&requests[2].url)["q"], "爸 我回来了 周杰伦");
    }

    #[test]
    fn lrclib_both_scripts_a_close_simplified_record_beats_a_loose_original_one() {
        let (_, provider) = mv_setup(
            &record(1, "爸 我回來了", 236.0, "loose"),
            &record(2, "爸 我回来了", 230.0, "close"),
        );

        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "close"), false)));
    }

    #[test]
    fn lrclib_both_scripts_a_loose_record_is_best_only_when_nothing_is_closer() {
        let (_, provider) = mv_setup(&record(1, "爸 我回來了", 236.0, "loose"), "");
        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "loose"), true)));

        // 同一份搜尋結果裡，吻合的勝過排在前面的放寬者。
        let both = format!(
            "{},{}",
            record(1, "爸 我回來了", 236.0, "loose"),
            record(2, "爸 我回來了", 231.0, "close")
        );
        let (_, provider) = mv_setup(&both, "");
        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "close"), false)));

        // 差超過 10 秒仍然不收。
        let (_, provider) = mv_setup(&record(1, "爸 我回來了", 240.0, "far"), "");
        assert_eq!(fetch_mv(&provider), None);
    }

    #[test]
    fn lrclib_both_scripts_a_close_synced_record_beats_an_earlier_plain_one() {
        let plain = r#"{"id":1,"trackName":"爸 我回來了","duration":230,"plainLyrics":"words"}"#;
        let (_, provider) = mv_setup(plain, &record(2, "爸 我回来了", 230.0, "synced"));
        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "synced"), false)));
    }

    // MARK: every eligible candidate (Rust only)

    #[test]
    fn lrclib_collects_every_eligible_record_from_all_three_requests() {
        let (http, provider) = setup();
        let t = mv_track();
        let retry = t.simplified_chinese().expect("traditional query");
        let one = record(1, "爸 我回來了", 230.0, "one");
        http.respond(LrclibProvider::exact_url(&t), 200, one.clone());
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            format!(
                "[{one},{},{},{}]",
                record(4, "別首歌", 230.0, "wrong title"),
                record(5, "爸 我回來了", 250.0, "twenty seconds off"),
                record(2, "爸 我回來了", 236.0, "two"),
            ),
        );
        http.respond(
            LrclibProvider::search_url(&retry),
            200,
            format!("[{}]", record(3, "爸 我回来了", 230.0, "three")),
        );

        let found = block_on(provider.fetch(&t)).unwrap();

        assert_eq!(
            urls(&found.hits),
            [
                "https://lrclib.net/api/get/1",
                "https://lrclib.net/api/get/2",
                "https://lrclib.net/api/get/3",
            ]
        );
        assert!(found.complete);
        assert!(found.hits[1].loose_duration, "放寬的紀錄也是候選");
        assert_eq!(http.requests().len(), 3);
    }

    #[test]
    fn lrclib_partial_failure_returns_what_it_has_as_incomplete() {
        let t = mv_track();
        let retry = t.simplified_chinese().expect("traditional query");
        let original = record(1, "爸 我回來了", 230.0, "found");

        let (http, provider) = mv_setup(&original, "");
        http.respond(LrclibProvider::search_url(&retry), 500, "");
        let found = block_on(provider.fetch(&t)).unwrap();
        assert_eq!(urls(&found.hits), ["https://lrclib.net/api/get/1"]);
        assert!(!found.complete);

        // 排在前面的請求失敗，也不丟掉後面查到的。
        let (http, provider) = mv_setup(&original, "");
        http.respond(LrclibProvider::exact_url(&t), 503, "");
        let found = block_on(provider.fetch(&t)).unwrap();
        assert_eq!(urls(&found.hits), ["https://lrclib.net/api/get/1"]);
        assert!(!found.complete);

        // 什麼都沒找到而且有請求失敗：就是失敗（不能被當成「查無」快取一整天）。
        let (http, provider) = mv_setup("", "");
        http.respond(LrclibProvider::search_url(&retry), 500, "");
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(500))
        );

        // 不只一個失敗時，回報依請求順序的第一個。
        let (http, provider) = mv_setup("", "");
        http.respond(LrclibProvider::exact_url(&t), 503, "");
        http.respond(LrclibProvider::search_url(&t), 502, "");
        http.respond(LrclibProvider::search_url(&retry), 500, "");
        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(503))
        );
    }

    #[test]
    fn lrclib_records_sharing_a_fallback_url_keep_the_best_one() {
        // 沒有可用編號的紀錄網址都是搜尋位址：只留一筆，而且是最好的那筆。
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            r#"[{"trackName":"T","duration":200,"plainLyrics":"words"},
             {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]line"},
             {"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]later"}]"#,
        );

        let found = block_on(provider.fetch(&t)).unwrap();

        assert_eq!(urls(&found.hits), [LrclibProvider::search_url(&t)]);
        assert_eq!(found.hits[0].lyrics, synced(1.0, "line"));
    }

    #[test]
    fn lrclib_sends_all_three_requests_without_waiting_for_each_other() {
        let (http, provider) = setup();
        let t = mv_track();
        let retry = t.simplified_chinese().expect("traditional query");
        let expected = [
            LrclibProvider::exact_url(&t),
            LrclibProvider::search_url(&t),
            LrclibProvider::search_url(&retry),
        ];
        for url in &expected {
            http.hang(url);
        }

        assert!(provider.fetch(&t).now_or_never().is_none());

        let sent: Vec<String> = http.requests().into_iter().map(|r| r.url).collect();
        assert_eq!(sent, expected);
    }

    // MARK: record artist (Rust only)

    fn by_artist(id: i64, artist: &str) -> String {
        format!(
            r#"{{"id":{id},"trackName":"爸 我回來了","artistName":{artist},"duration":230,"syncedLyrics":"[00:01.00]x"}}"#
        )
    }

    fn artist_flags(provider: &LrclibProvider) -> Vec<(String, bool)> {
        let found = block_on(provider.fetch(&mv_track())).unwrap();
        found
            .hits
            .into_iter()
            .map(|h| {
                let id = h.url.rsplit('/').next().unwrap().to_string();
                (id, h.artist_matches)
            })
            .collect()
    }

    fn flags(expected: &[(&str, bool)]) -> Vec<(String, bool)> {
        expected
            .iter()
            .map(|&(id, m)| (id.to_string(), m))
            .collect()
    }

    #[test]
    fn lrclib_search_records_whether_the_record_artist_matches() {
        let original = [
            by_artist(1, r#""恶童ETON""#),
            by_artist(2, r#""周杰伦""#),
            by_artist(3, "null"),
            record(4, "爸 我回來了", 230.0, "no artist field"),
            by_artist(5, r#""""#),
        ]
        .join(",");
        // 簡體搜尋用只剩中文名的查詢，但演出者仍和原本的查詢比：英文名也算相符。
        let (_, provider) = mv_setup(&original, &by_artist(6, r#""Jay Chou""#));

        assert_eq!(
            artist_flags(&provider),
            flags(&[
                ("1", false),
                ("2", true),
                ("3", true),
                ("4", true),
                ("5", true),
                ("6", true),
            ])
        );
    }

    /// 精確查詢的演出者是服務端自己比對的，回來的寫法不管長怎樣都算相符；
    /// 搜尋又回同一筆時也不改變這個結論。
    #[test]
    fn lrclib_exact_hit_counts_as_matching() {
        let same = by_artist(1, r#""Someone Else""#);
        let (http, provider) = mv_setup(&same, "");
        http.respond(LrclibProvider::exact_url(&mv_track()), 200, same.clone());

        assert_eq!(artist_flags(&provider), flags(&[("1", true)]));
    }
}
