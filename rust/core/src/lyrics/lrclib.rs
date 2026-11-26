//! LRCLIB：免費、免金鑰、社群維護的 LRC 資料庫（移植自 Swift `LRCLIBProvider`）。
//!
//! 先精確查詢、再搜尋：精確查詢可能只因為系統回報的標題少了資料庫帶有的
//! `(feat. …)` 後綴而落空。精確端點回 404 是乾淨的「沒有」，接著改搜尋；
//! 其他任何失敗都回 `Err`。

use std::sync::Arc;

use serde::{Deserialize, Deserializer};

use super::provider::{
    build_url, candidate_title, decode_array, decode_object, is_success, request, send,
    success_body, LyricsProvider, ProviderError,
};
use super::{parse_lrc, Lyrics, LyricsHit, TrackIdentity};
use crate::http::HttpClient;
use crate::runtime::BoxFuture;

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

    /// 搜尋結果中第一筆通過 `decode` 檢查、且歌名與查詢相符的紀錄。
    #[cfg(test)]
    fn decode_search(data: &[u8], track: &TrackIdentity) -> Option<Lyrics> {
        let payloads = decode_array::<Payload>(data).ok()?;
        Self::select_candidate(&payloads, track).map(|hit| hit.lyrics)
    }

    /// 精確查詢 → 搜尋 → 簡體搜尋。拿到時間軸且曲長吻合的就停；否則取其中最好的一筆。
    async fn fetch_lyrics(
        &self,
        track: &TrackIdentity,
    ) -> Result<Option<LyricsHit>, ProviderError> {
        let mut best = None;
        let exact_url = Self::exact_url(track);
        let exact = request(exact_url.clone(), &[]);
        let (status, body) = send(&self.http, exact).await?;
        if is_success(status) {
            let payload = decode_object::<Payload>(&body)?;
            if let Some(hit) = Self::payload_hit(&payload, track, &exact_url) {
                if !hit.loose_duration {
                    return Ok(Some(hit));
                }
                best = Some(hit);
            }
        } else if status != 404 {
            return Err(ProviderError::BadStatus(status));
        }

        if let Some(hit) = self.search(track).await? {
            LyricsHit::keep_better(&mut best, hit);
        }
        if best.as_ref().is_some_and(LyricsHit::is_best_possible) {
            return Ok(best);
        }

        if let Some(simplified) = track.simplified_chinese() {
            match self.search(&simplified).await {
                Ok(Some(hit)) => LyricsHit::keep_better(&mut best, hit),
                Ok(None) => {}
                // 手上沒有任何結果時，失敗要照實回報（只短暫快取）；有結果就用它。
                Err(error) if best.is_none() => return Err(error),
                Err(_) => {}
            }
        }
        Ok(best)
    }

    async fn search(&self, track: &TrackIdentity) -> Result<Option<LyricsHit>, ProviderError> {
        let search = request(Self::search_url(track), &[]);
        let found = success_body(&self.http, search).await?;
        let payloads = decode_array::<Payload>(&found)?;
        Ok(Self::select_candidate(&payloads, track))
    }

    /// 搜尋是模糊的，可能回來另一首歌、混音或翻唱；不像精確查詢（伺服器自己比對名稱），
    /// 每個候選都必須帶有與查詢寬鬆相符的歌名。寧可沒有歌詞，也不要錯的歌詞。
    /// 通過的候選取優先序最高者（時間軸、曲長吻合），同序取排在前面的。
    ///
    /// 命中的網址是候選自己的紀錄頁；候選沒有可用編號時退回搜尋請求的位址。
    fn select_candidate(payloads: &[Payload], track: &TrackIdentity) -> Option<LyricsHit> {
        let wanted = track.normalized().title;
        let search_url = Self::search_url(track);
        let mut best = None;
        for payload in payloads {
            let Some(name) = payload.track_name.as_deref().filter(|n| !n.is_empty()) else {
                continue;
            };
            if !TrackIdentity::loosely_matches(&candidate_title(name), &wanted) {
                continue;
            }
            if let Some(hit) = Self::payload_hit(payload, track, &search_url) {
                LyricsHit::keep_better(&mut best, hit);
            }
        }
        best
    }

    /// 一筆紀錄的命中；曲長差超過放寬的容忍（雙方都回報正的長度時才比）或沒有歌詞時為 `None`。
    fn payload_hit(
        payload: &Payload,
        track: &TrackIdentity,
        request_url: &str,
    ) -> Option<LyricsHit> {
        let fit = track.duration_fit(payload.duration)?;
        let lyrics = Self::payload_lyrics(payload)?;
        let url = Self::record_url(payload.id, request_url);
        Some(LyricsHit::new(lyrics, url).with_fit(fit))
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
    ) -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>> {
        Box::pin(self.fetch_lyrics(track))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpError;
    use crate::lyrics::provider::test_support::*;
    use crate::lyrics::LyricLine;
    use crate::testing::FakeHttp;

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

        let result = block_on(provider.fetch(&query)).unwrap();

        assert_eq!(result.map(|h| h.lyrics), Some(synced(1.0, "hit")));
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
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
        assert_eq!(request.timeout, std::time::Duration::from_secs(5));
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
        }

        let _ = block_on(provider.fetch(&a)).unwrap();
        let _ = block_on(provider.fetch(&b)).unwrap();

        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        for request in requests {
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

        let result = block_on(provider.fetch(&t)).unwrap();

        assert_eq!(
            result.map(|h| h.lyrics),
            Some(synced(2.0, "found by search"))
        );
        let requests = http.requests();
        let paths: Vec<&str> = requests.iter().map(|r| path(&r.url)).collect();
        assert_eq!(paths, ["/api/get", "/api/search"]);
        let q = query_values(&requests[1].url);
        assert_eq!(q.len(), 1);
        assert_eq!(q["q"], "T A");
        assert_eq!(header(&requests[1], "User-Agent"), Some("MusiCat"));
        assert_eq!(requests[1].timeout, std::time::Duration::from_secs(5));
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

        let result = block_on(provider.fetch(&t)).unwrap();

        assert_eq!(result.map(|h| h.lyrics), Some(synced(2.0, "right song")));
    }

    #[test]
    fn lrclib_not_found_everywhere_returns_nil_without_throwing() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(LrclibProvider::search_url(&t), 200, "[]");

        assert_eq!(block_on(provider.fetch(&t)), Ok(None));
    }

    #[test]
    fn lrclib_server_error_on_exact_lookup_throws() {
        let (http, provider) = setup();
        let t = track();
        http.respond(LrclibProvider::exact_url(&t), 503, "");

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::BadStatus(503))
        );
        assert_eq!(http.requests().len(), 1);
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

        assert_eq!(
            block_on(provider.fetch(&t)),
            Err(ProviderError::Transport("notConnectedToInternet".into()))
        );
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
        let hit = block_on(provider.fetch(&t)).unwrap().unwrap();
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
        block_on(provider_http.1.fetch(&t)).unwrap().unwrap()
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
        let hit = block_on(provider.fetch(&t)).unwrap().unwrap();
        assert_eq!(hit.url, "https://lrclib.net/api/get/42");

        let (http, provider) = setup();
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            format!(r#"[{{"id":0,"trackName":"T","duration":200,"syncedLyrics":"{TWO_LINES}"}}]"#),
        );
        let hit = block_on(provider.fetch(&t)).unwrap().unwrap();
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

    // MARK: Simplified Chinese retry and loose duration (Rust only)

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
        block_on(provider.fetch(&mv_track()))
            .unwrap()
            .map(|h| (h.lyrics, h.loose_duration))
    }

    #[test]
    fn lrclib_retries_in_simplified_chinese_when_the_search_misses() {
        let (http, provider) = mv_setup("", &record(9, "爸 我回来了", 230.0, "found"));

        let hit = block_on(provider.fetch(&mv_track())).unwrap().unwrap();

        assert_eq!(hit.lyrics, synced(1.0, "found"));
        assert!(!hit.loose_duration);
        assert_eq!(hit.url, "https://lrclib.net/api/get/9");
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(query_values(&requests[2].url)["q"], "爸 我回来了 周杰伦");
    }

    #[test]
    fn lrclib_does_not_retry_after_a_close_synced_hit() {
        let (http, provider) = mv_setup(&record(1, "爸 我回來了", 230.0, "original"), "");

        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "original"), false)));
        assert_eq!(http.requests().len(), 2);
    }

    #[test]
    fn lrclib_prefers_a_close_simplified_record_over_a_loose_original_one() {
        let (_, provider) = mv_setup(
            &record(1, "爸 我回來了", 236.0, "loose"),
            &record(2, "爸 我回来了", 230.0, "close"),
        );

        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "close"), false)));
    }

    #[test]
    fn lrclib_uses_a_loose_record_only_when_nothing_is_closer() {
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
    fn lrclib_prefers_a_close_synced_record_over_an_earlier_plain_one() {
        let plain = r#"{"id":1,"trackName":"爸 我回來了","duration":230,"plainLyrics":"words"}"#;
        let (_, provider) = mv_setup(plain, &record(2, "爸 我回来了", 230.0, "synced"));
        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "synced"), false)));
    }

    #[test]
    fn lrclib_failed_retry_keeps_what_was_already_found() {
        let (http, provider) = mv_setup(&record(1, "爸 我回來了", 236.0, "loose"), "");
        let retry = mv_track().simplified_chinese().unwrap();
        http.respond(LrclibProvider::search_url(&retry), 500, "");
        assert_eq!(fetch_mv(&provider), Some((synced(1.0, "loose"), true)));

        // 什麼都沒找到時，重試失敗就是失敗（不能被當成「查無」快取一整天）。
        let (http, provider) = mv_setup("", "");
        http.respond(LrclibProvider::search_url(&retry), 500, "");
        assert_eq!(
            block_on(provider.fetch(&mv_track())),
            Err(ProviderError::BadStatus(500))
        );
    }
}
