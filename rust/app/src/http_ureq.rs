//! 以 ureq（rustls）實作 core 的 [`HttpClient`]。
//!
//! 每次 `get` 在一條 `std::thread` 上跑阻塞的 ureq 呼叫，結果經 oneshot 交回；因此回傳的 future
//! 是 `Send`、不綁任何執行器，drop 它不會阻塞（執行緒在背景跑完，受請求逾時限制）。
//!
//! 語意（core 的 lyrics 管線依賴這些）：
//! - 非 2xx 是正常回應（`http_status_as_error(false)`）：LRCLIB 的 404 必須落到搜尋，不是傳輸錯誤。
//! - 本體最多讀 [`MAX_BODY_BYTES`]。2xx 超過 → `TooLarge`；非 2xx 超過 → 保留狀態碼、本體為空
//!   （巨大的 404 仍是 404，不能變成失敗）。
//! - 傳輸錯誤只帶固定的短描述，不含 URL：query 裡是歌名，不進錯誤字串或日誌。

use std::io::Read;

use futures_channel::oneshot;
use musicat_core::http::{HttpClient, HttpError, HttpRequest, HttpResponse};
use musicat_core::lyrics::provider::MAX_BODY_BYTES;
use musicat_core::runtime::BoxFuture;

/// 真實網路的 [`HttpClient`]。無狀態；每次請求建一個獨立的 ureq agent（沒有連線池，請求很少）。
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqHttp;

impl UreqHttp {
    pub fn new() -> Self {
        Self
    }
}

impl HttpClient for UreqHttp {
    fn get(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, HttpError>> {
        let (tx, rx) = oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("musicat-http".into())
            .spawn(move || {
                // 接收端若已被 drop（呼叫者放棄），結果直接丟掉。
                let _ = tx.send(blocking_get(&req));
            });
        Box::pin(async move {
            if spawned.is_err() {
                return Err(HttpError::Transport("could not start worker thread".into()));
            }
            rx.await
                .unwrap_or_else(|_| Err(HttpError::Transport("worker thread failed".into())))
        })
    }
}

fn blocking_get(req: &HttpRequest) -> Result<HttpResponse, HttpError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(req.timeout))
        .http_status_as_error(false)
        .build()
        .into();

    let mut request = agent.get(&req.url);
    for (name, value) in &req.headers {
        request = request.header(name, value);
    }
    let mut response = request.call().map_err(|e| transport_error(&e))?;

    let status = response.status().as_u16();
    let is_success = (200..300).contains(&status);

    // 多讀 1 位元組以分辨「剛好等於上限」與「超過上限」。
    let mut body = Vec::new();
    let read = response
        .body_mut()
        .as_reader()
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut body);
    if let Err(e) = read {
        return Err(HttpError::Transport(describe_io(&e).into()));
    }
    if body.len() > MAX_BODY_BYTES {
        return if is_success {
            Err(HttpError::TooLarge)
        } else {
            Ok(HttpResponse {
                status,
                body: Vec::new(),
            })
        };
    }
    Ok(HttpResponse { status, body })
}

/// 固定字串，不用 `Display`：ureq 的錯誤文字可能帶 URL 或主機名稱。
fn transport_error(e: &ureq::Error) -> HttpError {
    let msg = match e {
        ureq::Error::Timeout(_) => "timed out",
        ureq::Error::HostNotFound => "host not found",
        ureq::Error::ConnectionFailed => "connection failed",
        ureq::Error::Io(io) => describe_io(io),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS error",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "redirect failed",
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => "invalid request",
        _ => "request failed",
    };
    HttpError::Transport(msg.into())
}

fn describe_io(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "timed out",
        std::io::ErrorKind::ConnectionRefused => "connection refused",
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted => {
            "connection reset"
        }
        _ => "I/O error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicat_core::lyrics::provider::{MAX_BODY_BYTES, USER_AGENT};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    /// 讀到請求標頭結束（空行）為止，回傳整段標頭文字。
    fn read_request(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => buf.push(byte[0]),
                _ => break,
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// 每個連線回覆 `status` 與 `body`；回傳 base URL 與收到的請求標頭通道。
    fn serve(status: &'static str, body: Vec<u8>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = tx.send(read_request(&mut stream));
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // 客戶端可能讀到上限就關連線，寫入失敗是預期的。
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (url, rx)
    }

    /// 接受連線後永遠不回覆。
    fn serve_silent() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                held.push(stream);
            }
        });
        url
    }

    fn req(url: String, timeout: Duration) -> HttpRequest {
        HttpRequest {
            url,
            headers: vec![
                ("User-Agent".into(), USER_AGENT.into()),
                ("X-Test".into(), "hello".into()),
            ],
            timeout,
        }
    }

    fn get(url: String) -> Result<HttpResponse, HttpError> {
        futures_executor::block_on(UreqHttp::new().get(req(url, Duration::from_secs(5))))
    }

    #[test]
    fn returns_the_body_of_a_200() {
        let (base, _rx) = serve("200 OK", b"hello world".to_vec());
        let resp = get(format!("{base}/api")).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hello world");
    }

    #[test]
    fn a_404_with_a_body_is_a_normal_response_not_an_error() {
        let (base, _rx) = serve("404 Not Found", b"{\"message\":\"nope\"}".to_vec());
        let resp = get(format!("{base}/api/get")).unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"{\"message\":\"nope\"}");
    }

    #[test]
    fn a_2xx_body_over_the_cap_is_too_large() {
        let (base, _rx) = serve("200 OK", vec![b'x'; MAX_BODY_BYTES + 10]);
        assert_eq!(get(format!("{base}/big")), Err(HttpError::TooLarge));
    }

    #[test]
    fn a_2xx_body_exactly_at_the_cap_is_returned() {
        let (base, _rx) = serve("200 OK", vec![b'x'; MAX_BODY_BYTES]);
        let resp = get(format!("{base}/exact")).unwrap();
        assert_eq!(resp.body.len(), MAX_BODY_BYTES);
    }

    #[test]
    fn a_non_2xx_body_over_the_cap_keeps_its_status_with_an_empty_body() {
        let (base, _rx) = serve("404 Not Found", vec![b'x'; MAX_BODY_BYTES + 10]);
        let resp = get(format!("{base}/big404")).unwrap();
        assert_eq!(resp.status, 404);
        assert!(resp.body.is_empty());
    }

    #[test]
    fn a_server_that_never_replies_times_out_as_a_transport_error() {
        let url = serve_silent();
        let start = Instant::now();
        let result = futures_executor::block_on(
            UreqHttp::new().get(req(format!("{url}/hang"), Duration::from_millis(200))),
        );
        assert!(matches!(result, Err(HttpError::Transport(_))), "{result:?}");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn request_headers_including_user_agent_reach_the_server() {
        let (base, rx) = serve("200 OK", b"ok".to_vec());
        get(format!("{base}/h")).unwrap();
        let head = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .to_lowercase();
        assert!(head.starts_with("get /h http/1.1"), "{head}");
        assert!(head.contains("user-agent: musicat\r\n"), "{head}");
        assert!(head.contains("x-test: hello\r\n"), "{head}");
    }

    #[test]
    fn transport_error_messages_do_not_contain_the_query() {
        // 綁定後立刻放掉，得到一個沒人聽的埠：連線被拒。
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url =
            format!("http://127.0.0.1:{port}/api/get?track_name=SecretSong&artist_name=Hidden");
        let Err(HttpError::Transport(msg)) = get(url) else {
            panic!("expected a transport error");
        };
        assert!(!msg.is_empty());
        for leak in ["SecretSong", "Hidden", "track_name", "?", "/api/get"] {
            assert!(!msg.contains(leak), "{msg:?} leaks {leak:?}");
        }
    }

    #[test]
    fn the_returned_future_is_send_and_dropping_it_does_not_block() {
        fn assert_send<T: Send>(_: &T) {}
        let url = serve_silent();
        let client = UreqHttp::new();
        let fut = client.get(req(format!("{url}/hang"), Duration::from_secs(5)));
        assert_send(&fut);
        let start = Instant::now();
        drop(fut);
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
