//! 在預設瀏覽器開啟歌詞來源網址（公開的 AppKit／Foundation API：`NSWorkspace.openURL:`、`NSURL`）。
//!
//! 網址來自歌詞提供者的回應（LRCLIB、NetEase），屬於不可信輸入，所以開啟之前一律先過 [`validate_open_url`]：
//! 只放行小寫 `https://`，擋掉 `javascript:`、`file:`、`http:` 與自訂 scheme；主機名稱必須非空且不含 `@`
//! （擋掉 `https://好站@壞站/` 這類使用者資訊的偽裝）；整串不含空白或控制字元（避免換行、NUL 夾帶）；長度上限 2048 位元組。
//! 驗證不做修剪或正規化：有任何不合就整串拒絕。
//!
//! 驗證器是純函式，所有平台都編譯、都測試；ObjC 的部分（[`open_url`]）只在 macOS 編譯。
//! 非 ASCII 的主機名稱不在驗證器擋下：我們自己的提供者只會組出 ASCII 主機，其餘由 `NSURL` 判斷，
//! 無法解析成有效 `NSURL` 的字串由 [`open_url`] 回 `false`。
//!
//! 執行緒：[`open_url`] 必須在主執行緒呼叫（AppKit）；不在主執行緒時印一行警告（不含網址）並回 `false`。

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSWorkspace;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSString, NSURL};

/// 允許的網址長度上限（位元組）。
pub const MAX_URL_BYTES: usize = 2048;

/// 唯一允許的 scheme 前綴（小寫，區分大小寫）。
const HTTPS_PREFIX: &str = "https://";

/// 檢查 `url` 能不能交給瀏覽器開啟：通過就回傳同一個字串，否則 `None`。規則見模組文件。
pub fn validate_open_url(url: &str) -> Option<&str> {
    let rest = url.strip_prefix(HTTPS_PREFIX)?;
    if url.len() > MAX_URL_BYTES || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(url)
}

/// 在預設瀏覽器開啟 `url`；回傳是否已交給系統開啟。
///
/// 驗證沒過（[`validate_open_url`]）、不在主執行緒、`NSURL` 無法解析，或系統回報失敗都回 `false`。
/// 不在主執行緒時印一行警告，警告裡不含網址。
#[cfg(target_os = "macos")]
pub fn open_url(url: &str) -> bool {
    let Some(url) = validate_open_url(url) else {
        return false;
    };
    if MainThreadMarker::new().is_none() {
        eprintln!("musicat: open_url() must be called on the main thread; link not opened");
        return false;
    }
    let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&ns_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_https_urls() {
        for url in [
            "https://lrclib.net/api/get/1",
            "https://music.163.com/song?id=7",
            "https://x.com:8443/p#frag",
            "https://lrclib.net/api/get?track_name=%E5%BE%8C%E4%BE%86&artist_name=a%26b",
            "https://x.com",
            "https://x.com?q=1",
            "https://x.com#top",
            // `@` 在路徑或查詢裡不是使用者資訊。
            "https://x.com/a@b",
            "https://x.com?mail=a@b.com",
        ] {
            assert_eq!(validate_open_url(url), Some(url), "{url}");
        }
    }

    #[test]
    fn rejects_other_schemes_and_case_mismatch() {
        for url in [
            "http://x.com",
            "HTTPS://x.com",
            "Https://x.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://x.com",
            "x.com",
            "//x.com",
            "",
        ] {
            assert_eq!(validate_open_url(url), None, "{url}");
        }
    }

    #[test]
    fn rejects_missing_host() {
        for url in ["https://", "https:///path", "https://?q=1", "https://#f"] {
            assert_eq!(validate_open_url(url), None, "{url}");
        }
        // 主機部分是 `:8443`（非空）：驗證器不判斷埠號格式，留給 NSURL。
        assert_eq!(validate_open_url("https://:8443/"), Some("https://:8443/"));
    }

    #[test]
    fn rejects_userinfo() {
        for url in [
            "https://a@b.com",
            "https://a@b.com/",
            "https://user:pw@b.com/",
            "https://@b.com/",
            "https://good.com@evil.com/",
            "https://a.com\\@b.com/",
        ] {
            assert_eq!(validate_open_url(url), None, "{url}");
        }
    }

    #[test]
    fn rejects_whitespace_and_control_characters_anywhere() {
        for url in [
            "https://exa mple.com",
            "https://x.com/a b",
            "https://x.com/?q=a\tb",
            "https://x.com/\n",
            "https://x.com/a\nb",
            "https://x.com/a\rb",
            "https://x.com/a\0b",
            "https://x.com/\u{7f}",
            "https://x.com/\u{a0}",
            "https://x.com/\u{3000}",
            " https://x.com",
            "https://x.com ",
            "https://x.com\n",
        ] {
            assert_eq!(validate_open_url(url), None, "{url:?}");
        }
    }

    #[test]
    fn length_limit_is_2048_bytes_inclusive() {
        let prefix = "https://x.com/";
        let at_limit = format!("{prefix}{}", "a".repeat(MAX_URL_BYTES - prefix.len()));
        assert_eq!(at_limit.len(), 2048);
        assert_eq!(validate_open_url(&at_limit), Some(at_limit.as_str()));
        let over = format!("{at_limit}a");
        assert_eq!(over.len(), 2049);
        assert_eq!(validate_open_url(&over), None);
    }

    #[test]
    fn length_is_counted_in_bytes_not_characters() {
        // 每個「後」是 3 位元組：683 個 = 2049 位元組的路徑部分，超過上限。
        let prefix = "https://x.com/";
        let n = (MAX_URL_BYTES - prefix.len()) / 3 + 1;
        let url = format!("{prefix}{}", "後".repeat(n));
        assert!(url.len() > MAX_URL_BYTES);
        assert_eq!(validate_open_url(&url), None);
    }

    #[test]
    fn odd_escapes_are_left_to_nsurl() {
        for url in ["https://x.com/100%", "https://x.com/%zz", "https://x.com/%"] {
            assert_eq!(validate_open_url(url), Some(url), "{url}");
        }
    }

    #[test]
    fn non_ascii_host_follows_the_same_rules() {
        // 驗證器不另擋非 ASCII 主機（NSURL 會判斷，無效則 open_url 回 false）。
        let ok = "https://例え.jp/path";
        assert_eq!(validate_open_url(ok), Some(ok));
        assert_eq!(validate_open_url("https://例え@x.jp/"), None);
        assert_eq!(validate_open_url("https://例 え.jp/"), None);
    }

    #[test]
    fn does_not_trim() {
        assert_eq!(validate_open_url("https://x.com/ "), None);
        assert_eq!(validate_open_url("\thttps://x.com/"), None);
    }
}
