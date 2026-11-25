//! 歌詞提供者介面與共用的請求管線（移植自 Swift `LyricsProvider`／`LyricsHTTP`）。
//!
//! 提供者必須區分「失敗」（回傳 `Err`）與「乾淨地沒有」（回傳 `Ok(None)`）：
//! `LyricsRepository` 對兩者的快取策略截然不同。

use std::sync::Arc;
use std::time::Duration;

use super::LyricsHit;
use crate::http::{HttpClient, HttpError, HttpRequest};
use crate::lyrics::identity::TrackIdentity;
use crate::runtime::BoxFuture;

/// 請求失敗的方式，與「服務回答說沒有」區分開。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    BadStatus(u16),
    BadResponse,
    Transport(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::BadStatus(code) => write!(f, "bad status {code}"),
            ProviderError::BadResponse => f.write_str("bad response"),
            ProviderError::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// 一個歌詞來源。
///
/// 回傳 `Err` 表示請求失敗（傳輸錯誤、非 2xx、2xx 但看不懂的本體）；
/// `Ok(None)` 只用於服務乾淨地回答「沒有」。兩者的快取策略不同，不可混用。
/// 命中的 `LyricsHit` 帶著這筆歌詞在來源服務上的 `https://` 網址。
pub trait LyricsProvider: Send + Sync {
    fn id(&self) -> &str;
    fn fetch<'a>(
        &'a self,
        track: &'a TrackIdentity,
    ) -> BoxFuture<'a, Result<Option<LyricsHit>, ProviderError>>;
}

/// 固定、不帶任何使用者資料。
pub const USER_AGENT: &str = "MusiCat";

/// 2xx 回應本體的上限。超過視為失敗而非命中，避免惡意或出錯的服務吃光記憶體。
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// 每個來源的請求逾時；逾時視同失敗。
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// 以嚴格百分比編碼組出 URL：只保留 `A-Za-z0-9-._~`，其餘（包含 `+`、`&`、空白與非 ASCII）
/// 逐位元組編成大寫十六進位。通用的 query 編碼會放過 `+` 與 `&`，
/// 而伺服器把前者讀成空白、後者讀成分隔符。
pub fn build_url(base: &str, query: &[(&str, &str)]) -> String {
    let mut url = String::from(base);
    for (i, (name, value)) in query.iter().enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        encode_into(&mut url, name);
        url.push('=');
        encode_into(&mut url, value);
    }
    url
}

fn encode_into(out: &mut String, text: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
}

/// 帶固定 User-Agent 與 5 秒逾時的 GET 請求。
pub(crate) fn request(url: String, extra_headers: &[(&str, &str)]) -> HttpRequest {
    let mut headers = vec![("User-Agent".to_string(), USER_AGENT.to_string())];
    headers.extend(
        extra_headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string())),
    );
    HttpRequest {
        url,
        headers,
        timeout: REQUEST_TIMEOUT,
    }
}

fn map_http_error(error: HttpError) -> ProviderError {
    match error {
        HttpError::Transport(msg) => ProviderError::Transport(msg),
        // 本體過大與「2xx 但看不懂」同類：服務答了，但答案不能用。
        HttpError::TooLarge => ProviderError::BadResponse,
    }
}

/// 傳輸錯誤（含逾時）原樣往上傳。2xx 的本體超過 `MAX_BODY_BYTES` 即使用戶端沒擋住也視為失敗；
/// 非 2xx 的本體不會被讀取，所以不檢查大小。
pub(crate) async fn send(
    http: &Arc<dyn HttpClient>,
    request: HttpRequest,
) -> Result<(u16, Vec<u8>), ProviderError> {
    let response = http.get(request).await.map_err(map_http_error)?;
    if is_success(response.status) && response.body.len() > MAX_BODY_BYTES {
        return Err(ProviderError::BadResponse);
    }
    Ok((response.status, response.body))
}

pub(crate) fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// 2xx 回應的本體；其他狀態都是失敗。
pub(crate) async fn success_body(
    http: &Arc<dyn HttpClient>,
    request: HttpRequest,
) -> Result<Vec<u8>, ProviderError> {
    let (status, body) = send(http, request).await?;
    if !is_success(status) {
        return Err(ProviderError::BadStatus(status));
    }
    Ok(body)
}

/// 解碼頂層為 JSON 物件的本體；不是物件、或 JSON 壞掉一律為 `BadResponse`。
pub(crate) fn decode_object<T: serde::de::DeserializeOwned>(
    body: &[u8],
) -> Result<T, ProviderError> {
    decode_json(body, b'{')
}

/// 解碼頂層為 JSON 陣列的本體。
pub(crate) fn decode_array<T: serde::de::DeserializeOwned>(
    body: &[u8],
) -> Result<Vec<T>, ProviderError> {
    decode_json(body, b'[')
}

/// 候選歌名在與查詢比對前先經過與查詢相同的正規化（去掉 `(feat. …)` 等後綴）。
pub(crate) fn candidate_title(name: &str) -> String {
    TrackIdentity {
        title: name.to_string(),
        artist: String::new(),
        album: None,
        duration: None,
    }
    .normalized()
    .title
}

/// serde 的 derive 會把 JSON 陣列當成位置式欄位接受，Swift 的解碼器不會；
/// 先確認頂層形狀（容忍 UTF-8 BOM 與前導空白）來保持同樣的嚴格度。
fn decode_json<T: serde::de::DeserializeOwned>(body: &[u8], open: u8) -> Result<T, ProviderError> {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    let first = body.iter().find(|b| !b.is_ascii_whitespace());
    if first != Some(&open) {
        return Err(ProviderError::BadResponse);
    }
    serde_json::from_slice(body).map_err(|_| ProviderError::BadResponse)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::task::{Context, Poll};

    use futures_util::task::noop_waker;

    /// FakeHttp 的回應都是立即就緒，所以不需要真的執行器。
    pub fn block_on<F: Future>(f: F) -> F::Output {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        for _ in 0..1000 {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
        panic!("future never completed: it is waiting on something that never arrives");
    }

    fn percent_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    /// 解碼後的查詢參數（對應 Swift 測試的 `queryValues`）。
    pub fn query_values(url: &str) -> BTreeMap<String, String> {
        let Some((_, query)) = url.split_once('?') else {
            return BTreeMap::new();
        };
        query
            .split('&')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (percent_decode(k), percent_decode(v))
            })
            .collect()
    }

    pub fn host(url: &str) -> &str {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        rest.split(['/', '?']).next().unwrap()
    }

    pub fn path(url: &str) -> &str {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let rest = rest.split('?').next().unwrap();
        rest.find('/').map_or("", |i| &rest[i..])
    }

    pub fn header<'a>(req: &'a crate::http::HttpRequest, name: &str) -> Option<&'a str> {
        req.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::lyrics::lrclib::LrclibProvider;
    use crate::lyrics::netease::NetEaseProvider;
    use crate::testing::FakeHttp;

    fn track() -> TrackIdentity {
        TrackIdentity {
            title: "T".into(),
            artist: "A".into(),
            album: Some("Al".into()),
            duration: Some(200.0),
        }
    }

    #[test]
    fn build_url_keeps_only_unreserved_characters() {
        let url = build_url("https://x.test/p", &[("a", "AZaz09-._~")]);
        assert_eq!(url, "https://x.test/p?a=AZaz09-._~");
    }

    #[test]
    fn build_url_encodes_plus_ampersand_space_and_equals() {
        let url = build_url("https://x.test/p", &[("q", "a+b&c d=e")]);
        assert_eq!(url, "https://x.test/p?q=a%2Bb%26c%20d%3De");
    }

    #[test]
    fn build_url_encodes_non_ascii_as_uppercase_utf8_hex() {
        let url = build_url("https://x.test/p", &[("q", "晴é")]);
        assert_eq!(url, "https://x.test/p?q=%E6%99%B4%C3%A9");
    }

    #[test]
    fn build_url_encodes_names_and_joins_pairs_in_order() {
        let url = build_url("https://x.test/p", &[("a b", "1"), ("c", "2/3?4#5%")]);
        assert_eq!(url, "https://x.test/p?a%20b=1&c=2%2F3%3F4%235%25");
    }

    #[test]
    fn build_url_without_query_is_the_base() {
        assert_eq!(build_url("https://x.test/p", &[]), "https://x.test/p");
    }

    #[test]
    fn user_agent_is_the_bare_app_name() {
        assert_eq!(USER_AGENT, "MusiCat");
        let req = request("https://x.test".into(), &[("Referer", "r")]);
        assert_eq!(req.url, "https://x.test");
        assert_eq!(req.timeout, Duration::from_secs(5));
        assert_eq!(header(&req, "User-Agent"), Some("MusiCat"));
        assert_eq!(header(&req, "Referer"), Some("r"));
        assert_eq!(req.headers.len(), 2);
    }

    #[test]
    fn provider_ids_match_the_swift_ids() {
        let http: Arc<dyn HttpClient> = Arc::new(FakeHttp::new());
        assert_eq!(LrclibProvider::new(http.clone()).id(), "net.lrclib");
        assert_eq!(NetEaseProvider::new(http).id(), "com.music163");
    }

    #[test]
    fn fetch_futures_are_send_and_borrow_provider_and_track() {
        fn assert_send<T: Send>(_: &T) {}
        let http: Arc<dyn HttpClient> = Arc::new(FakeHttp::new());
        let providers: Vec<Box<dyn LyricsProvider>> = vec![
            Box::new(LrclibProvider::new(http.clone())),
            Box::new(NetEaseProvider::new(http)),
        ];
        let t = track();
        for p in &providers {
            let fut = p.fetch(&t);
            assert_send(&fut);
        }
    }

    fn padded(json: &str, total: usize) -> Vec<u8> {
        let mut body = json.as_bytes().to_vec();
        body.resize(total, b' ');
        body
    }

    const LRCLIB_HIT: &str = r#"{"duration":200,"syncedLyrics":"[00:01.00]hit"}"#;

    #[test]
    fn oversized_body_is_a_failure_not_a_hit() {
        let t = track();

        // 用戶端回報 TooLarge：兩個提供者都是失敗。
        let http = Arc::new(FakeHttp::new());
        http.fail(LrclibProvider::exact_url(&t), HttpError::TooLarge);
        let lrclib = LrclibProvider::new(http.clone());
        assert!(block_on(lrclib.fetch(&t)).is_err());

        let http = Arc::new(FakeHttp::new());
        http.fail(NetEaseProvider::search_url(&t), HttpError::TooLarge);
        let netease = NetEaseProvider::new(http.clone());
        assert!(block_on(netease.fetch(&t)).is_err());

        // 用戶端沒擋住：合法 JSON 後面接滿空白，本體超過上限；解碼得出命中，但仍須失敗。
        let http = Arc::new(FakeHttp::new());
        http.respond(
            LrclibProvider::exact_url(&t),
            200,
            padded(LRCLIB_HIT, MAX_BODY_BYTES + 1),
        );
        let lrclib = LrclibProvider::new(http.clone());
        assert_eq!(block_on(lrclib.fetch(&t)), Err(ProviderError::BadResponse));
        assert_eq!(http.requests().len(), 1, "不得退回搜尋");

        let http = Arc::new(FakeHttp::new());
        http.respond(LrclibProvider::exact_url(&t), 404, "{}");
        http.respond(
            LrclibProvider::search_url(&t),
            200,
            padded(
                r#"[{"trackName":"T","duration":200,"syncedLyrics":"[00:01.00]hit"}]"#,
                MAX_BODY_BYTES + 1,
            ),
        );
        let lrclib = LrclibProvider::new(http);
        assert_eq!(block_on(lrclib.fetch(&t)), Err(ProviderError::BadResponse));

        let http = Arc::new(FakeHttp::new());
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            padded(
                r#"{"code":200,"result":{"songs":[{"id":1,"name":"T","duration":200000}]}}"#,
                MAX_BODY_BYTES + 1,
            ),
        );
        let netease = NetEaseProvider::new(http.clone());
        assert_eq!(block_on(netease.fetch(&t)), Err(ProviderError::BadResponse));
        assert_eq!(http.requests().len(), 1, "不得繼續查歌詞");

        let http = Arc::new(FakeHttp::new());
        http.respond(
            NetEaseProvider::search_url(&t),
            200,
            r#"{"code":200,"result":{"songs":[{"id":1,"name":"T","duration":200000}]}}"#,
        );
        http.respond(
            NetEaseProvider::lyric_url(1),
            200,
            padded(
                r#"{"lrc":{"lyric":"[00:01.00]a\n[00:02.00]b"}}"#,
                MAX_BODY_BYTES + 1,
            ),
        );
        let netease = NetEaseProvider::new(http);
        assert_eq!(block_on(netease.fetch(&t)), Err(ProviderError::BadResponse));
    }

    #[test]
    fn body_exactly_at_the_limit_is_still_accepted() {
        let t = track();
        let http = Arc::new(FakeHttp::new());
        http.respond(
            LrclibProvider::exact_url(&t),
            200,
            padded(LRCLIB_HIT, MAX_BODY_BYTES),
        );
        let lrclib = LrclibProvider::new(http);
        let got = block_on(lrclib.fetch(&t)).unwrap();
        assert!(got.is_some());
    }

    #[test]
    fn oversized_error_body_does_not_hide_the_status() {
        let t = track();
        let http = Arc::new(FakeHttp::new());
        http.respond(
            LrclibProvider::exact_url(&t),
            503,
            vec![b'x'; MAX_BODY_BYTES + 1],
        );
        let lrclib = LrclibProvider::new(http);
        assert_eq!(
            block_on(lrclib.fetch(&t)),
            Err(ProviderError::BadStatus(503))
        );
    }

    #[test]
    fn provider_error_messages_name_the_failure() {
        assert_eq!(ProviderError::BadStatus(503).to_string(), "bad status 503");
        assert_eq!(ProviderError::BadResponse.to_string(), "bad response");
        assert_eq!(
            ProviderError::Transport("boom".into()).to_string(),
            "transport error: boom"
        );
    }

    #[test]
    fn decode_object_rejects_non_objects_that_serde_would_accept_as_sequences() {
        #[derive(serde::Deserialize, Debug)]
        struct S {
            #[allow(dead_code)]
            a: Option<i64>,
        }
        assert!(decode_object::<S>(b"[1]").is_err());
        assert!(decode_object::<S>(b"null").is_err());
        assert!(decode_object::<S>(b"{}").is_ok());
        assert!(
            decode_object::<S>(b"\xEF\xBB\xBF {\"a\":1}").is_ok(),
            "UTF-8 BOM 與前導空白可容忍"
        );
    }

    /// Swift 版的 `testLiveLookupSmoke`：手動對真實服務跑，預設不執行。
    /// core 沒有真實的 `HttpClient`（那在 app 的 ureq 實作），所以這裡只留一個佔位，
    /// 實機冒煙測試走 app crate 的 `MUSICAT_LIVE_TESTS=1`。
    #[test]
    #[ignore = "needs a real HttpClient (lives in the app crate); set MUSICAT_LIVE_TESTS=1 there"]
    fn live_lookup_smoke() {
        eprintln!("skipped: musicat-core has no real HttpClient");
    }
}
