//! 注入式 HTTP 介面。core 只定義形狀；實作在外殼（例如 app 的 ureq 版本）與 `testing::FakeHttp`。

use std::time::Duration;

use crate::runtime::BoxFuture;

/// 一次 GET 請求。`timeout` 是實作端的傳輸層上限；呼叫端另外用 `with_timeout` 做邏輯逾時。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// 連線、DNS、TLS、傳輸層逾時等。
    Transport(String),
    /// 回應本體超過實作端的上限。
    TooLarge,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Transport(msg) => write!(f, "transport error: {msg}"),
            HttpError::TooLarge => f.write_str("response body too large"),
        }
    }
}

impl std::error::Error for HttpError {}

pub trait HttpClient: Send + Sync {
    /// 回傳的 future drop 即放棄該請求。
    fn get(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, HttpError>>;
}
