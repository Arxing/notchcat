//! helper 的線上格式：每行一個 JSON 物件，加上把管線位元組切成行的緩衝。
//! 移植自 Swift `HelperMessageParser.swift` 的 `HelperMessageParser`／`HelperLineBuffer`
//! 與 `PerlHelperLauncher.swift` 的 `CappedHelperLineBuffer`。
//!
//! 全部是純 Rust、沒有 OS API，所以在所有目標上都編譯（也都跑測試）。

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use musicat_core::types::{ArtworkRef, PositionAnchor, TrackSnapshot};
use serde_json::Value;

pub const SOURCE_ID: &str = "mediaremote";

/// helper 的一行解碼結果。
#[derive(Clone, PartialEq, Debug)]
pub enum HelperMessage {
    None,
    Track(TrackSnapshot),
}

/// 單行長度上限的預設值（不含換行）。
pub const DEFAULT_MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 解碼 helper 的一行：一個 JSON 物件。
///
/// ```text
/// {"none":true}
/// {"title":String,"artist":String?,"album":String?,"duration":Number?,
///  "elapsed":Number,"timestamp":Number,"rate":Number,
///  "artworkId":String?,"artworkData":String?}
/// ```
///
/// `timestamp` 是 Unix 秒；`artworkData` 是 base64 圖片位元組，helper 沒把圖片附在這則訊息時缺席。
///
/// 回傳 `None` 表示這不是合法訊息（與 `Some(HelperMessage::None)`「沒有在播」不同），包括：
/// 沒有標題或標題為空、`duration`／`elapsed`／`timestamp`／`rate` 存在但不是有限的 JSON 數字。
/// 整行以 `serde_json::Value` 解析，所以過深的巢狀（超過 serde_json 的遞迴上限 128）會回 `None`，不會堆疊溢位。
pub fn parse_line(line: &str) -> Option<HelperMessage> {
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;

    if object.get("none") == Some(&Value::Bool(true)) {
        return Some(HelperMessage::None);
    }

    let title = match object.get("title") {
        Some(Value::String(title)) if !title.is_empty() => title.clone(),
        _ => return None,
    };

    // 欄位存在就必須是有限數字，否則整行拒絕；欄位不存在才是「沒有」。
    let number = |key: &str| -> Result<Option<f64>, ()> {
        match object.get(key) {
            None => Ok(None),
            Some(Value::Number(n)) => match n.as_f64() {
                Some(f) if f.is_finite() => Ok(Some(f)),
                _ => Err(()),
            },
            Some(_) => Err(()),
        }
    };
    let duration = number("duration").ok()?;
    let elapsed = number("elapsed").ok()?;
    let timestamp = number("timestamp").ok()?;
    let rate = number("rate").ok()?;

    let position = match (elapsed, timestamp, rate) {
        (Some(elapsed), Some(timestamp), Some(rate)) => Some(PositionAnchor {
            elapsed,
            timestamp,
            rate,
        }),
        _ => None,
    };

    let artwork = non_empty(object.get("artworkId")).map(|id| ArtworkRef {
        id,
        data: match object.get("artworkData") {
            Some(Value::String(encoded)) => STANDARD.decode(encoded).ok().map(Arc::<[u8]>::from),
            _ => None,
        },
    });

    Some(HelperMessage::Track(TrackSnapshot {
        title,
        artist: non_empty(object.get("artist")),
        album: non_empty(object.get("album")),
        duration: duration.filter(|d| *d > 0.0),
        is_playing: rate.unwrap_or(0.0) > 0.0,
        position,
        artwork,
        source_id: SOURCE_ID.to_string(),
    }))
}

fn non_empty(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// 累積從 helper 管線讀到的位元組，只吐出完整的行。
///
/// 在位元組層級以 `\n` 切行，所以被切在兩次讀取之間的多位元組字元會被接回來。
/// 不是合法 UTF-8 的行整行丟棄，不影響前後的行；空行略過。
///
/// 單行上限（不含換行）：超過上限的行會被丟棄到它的換行為止（丟棄狀態跨 `push` 延續），
/// 所以行為異常的 helper 不能讓暫存無限成長。恰好等於上限的行保留，多一個位元組就丟。
/// 合併自 Swift 的 `HelperLineBuffer` 與 `CappedHelperLineBuffer`（語意相同）。
pub struct LineBuffer {
    max_line_bytes: usize,
    /// 目前還沒收到換行的那一行的前半段。丟棄中時一定是空的。
    pending: Vec<u8>,
    discarding: bool,
}

impl LineBuffer {
    /// 單行上限 4 MiB。
    pub fn new() -> Self {
        Self::with_max(DEFAULT_MAX_LINE_BYTES)
    }

    pub fn with_max(max_line_bytes: usize) -> Self {
        Self {
            max_line_bytes,
            pending: Vec::new(),
            discarding: false,
        }
    }

    /// 餵入新讀到的位元組，回傳因此完成的行（不含換行）。每個位元組只被掃描、複製一次。
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let newline = rest.iter().position(|&b| b == b'\n');
            let segment = &rest[..newline.unwrap_or(rest.len())];
            if self.discarding {
                // 丟棄中：這一段不留。
            } else if self.pending.len() + segment.len() > self.max_line_bytes {
                // 釋放記憶體，而不只是 clear。
                self.pending = Vec::new();
                self.discarding = true;
            } else if newline.is_some() {
                // 這一行完成。暫存為空時直接從輸入切片轉換，省一次複製。
                let line = if self.pending.is_empty() {
                    std::str::from_utf8(segment).ok().map(str::to_owned)
                } else {
                    self.pending.extend_from_slice(segment);
                    String::from_utf8(std::mem::take(&mut self.pending)).ok()
                };
                if let Some(line) = line.filter(|l| !l.is_empty()) {
                    lines.push(line);
                }
            } else {
                self.pending.extend_from_slice(segment);
            }
            match newline {
                Some(index) => {
                    self.discarding = false;
                    self.pending.clear();
                    rest = &rest[index + 1..];
                }
                None => break,
            }
        }
        lines
    }
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn track(line: &str) -> TrackSnapshot {
        match parse_line(line) {
            Some(HelperMessage::Track(snapshot)) => snapshot,
            other => panic!("expected a track for: {line} (got {other:?})"),
        }
    }

    fn b64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    // ---- HelperMessageParserTests（逐案移植）----

    #[test]
    fn parses_full_track() {
        let artwork = [0xFFu8, 0xD8, 0xFF, 0xE0, 0x01, 0x02];
        let line = format!(
            r#"{{"title":"蘇裔非 - 抱我!  ft. 莊岱諺","artist":"蘇裔非","album":"","duration":213.081,"elapsed":0,"timestamp":1793750400.5,"rate":1,"artworkId":"7C1F2A52-4D41-4E0B-9B6B-0E2C3D4F5A6B","artworkData":"{}"}}"#,
            b64(&artwork)
        );
        let snapshot = track(&line);
        assert_eq!(snapshot.title, "蘇裔非 - 抱我!  ft. 莊岱諺");
        assert_eq!(snapshot.artist.as_deref(), Some("蘇裔非"));
        assert_eq!(snapshot.album, None);
        assert_eq!(snapshot.duration, Some(213.081));
        assert!(snapshot.is_playing);
        assert_eq!(snapshot.source_id, "mediaremote");
        assert_eq!(
            snapshot.position,
            Some(PositionAnchor {
                elapsed: 0.0,
                timestamp: 1793750400.5,
                rate: 1.0
            })
        );
        let art = snapshot.artwork.expect("artwork");
        assert_eq!(art.id, "7C1F2A52-4D41-4E0B-9B6B-0E2C3D4F5A6B");
        assert_eq!(art.data.as_deref(), Some(&artwork[..]));
    }

    #[test]
    fn parses_none() {
        assert_eq!(parse_line(r#"{"none":true}"#), Some(HelperMessage::None));
    }

    #[test]
    fn artwork_data_absent_keeps_identifier() {
        let snapshot =
            track(r#"{"title":"T","elapsed":1,"timestamp":100,"rate":1,"artworkId":"abc"}"#);
        let art = snapshot.artwork.expect("artwork");
        assert_eq!(art.id, "abc");
        assert_eq!(art.data, None);
    }

    #[test]
    fn rate_zero_is_paused() {
        let snapshot = track(r#"{"title":"T","elapsed":42,"timestamp":100,"rate":0}"#);
        assert!(!snapshot.is_playing);
        assert_eq!(snapshot.position.map(|p| p.rate), Some(0.0));
        assert_eq!(snapshot.position.map(|p| p.elapsed), Some(42.0));
    }

    #[test]
    fn missing_title_yields_nil() {
        assert_eq!(
            parse_line(r#"{"artist":"A","elapsed":1,"timestamp":100,"rate":1}"#),
            None
        );
    }

    #[test]
    fn empty_title_yields_nil() {
        assert_eq!(
            parse_line(r#"{"title":"","elapsed":1,"timestamp":100,"rate":1}"#),
            None
        );
    }

    #[test]
    fn garbage_yields_nil() {
        for line in ["hello", "{", "", "[]"] {
            assert_eq!(parse_line(line), None, "line: {line}");
        }
    }

    #[test]
    fn missing_optional_fields_are_fine() {
        let snapshot = track(r#"{"title":"T","elapsed":5,"timestamp":200,"rate":1}"#);
        assert_eq!(snapshot.title, "T");
        assert_eq!(snapshot.artist, None);
        assert_eq!(snapshot.album, None);
        assert_eq!(snapshot.duration, None);
        assert_eq!(snapshot.artwork, None);
        assert_eq!(
            snapshot.position,
            Some(PositionAnchor {
                elapsed: 5.0,
                timestamp: 200.0,
                rate: 1.0
            })
        );
    }

    #[test]
    fn empty_artist_is_nil() {
        let snapshot = track(r#"{"title":"T","artist":"","elapsed":5,"timestamp":200,"rate":1}"#);
        assert_eq!(snapshot.artist, None);
    }

    #[test]
    fn non_finite_numbers_yield_nil() {
        // 1e999 超出 f64 範圍：serde_json 直接拒絕該行（Swift 版是解成無限大再拒絕），結果同為 nil。
        for key in ["elapsed", "timestamp", "rate", "duration"] {
            let mut fields = [
                ("elapsed", "1"),
                ("timestamp", "100"),
                ("rate", "1"),
                ("duration", "200"),
            ];
            for field in fields.iter_mut() {
                if field.0 == key {
                    field.1 = "1e999";
                }
            }
            let body = fields
                .iter()
                .map(|(k, v)| format!("\"{k}\":{v}"))
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(
                parse_line(&format!("{{\"title\":\"T\",{body}}}")),
                None,
                "key: {key}"
            );
        }
    }

    #[test]
    fn non_positive_duration_is_absent() {
        for duration in ["0", "-3"] {
            let snapshot = track(&format!(
                "{{\"title\":\"T\",\"duration\":{duration},\"elapsed\":1,\"timestamp\":100,\"rate\":1}}"
            ));
            assert_eq!(snapshot.duration, None, "duration: {duration}");
            assert_eq!(snapshot.title, "T");
        }
    }

    // ---- HelperLineBuffer ----

    #[test]
    fn line_buffer_splits_across_reads() {
        let mut buffer = LineBuffer::new();
        assert_eq!(buffer.push(br#"{"none":"#), Vec::<String>::new());
        assert_eq!(buffer.push(b"true}\n"), vec![r#"{"none":true}"#]);
    }

    #[test]
    fn line_buffer_handles_multiple_lines_in_one_read() {
        let mut buffer = LineBuffer::new();
        assert_eq!(buffer.push(b"one\ntwo\nthr"), vec!["one", "two"]);
        assert_eq!(buffer.push(b"ee\n"), vec!["three"]);
    }

    #[test]
    fn line_buffer_drops_invalid_utf8_line() {
        let mut buffer = LineBuffer::new();
        let mut data = b"good\n".to_vec();
        data.extend_from_slice(&[0x62, 0xFF, 0xFE, 0x0A]);
        data.extend_from_slice(br#"{"none":true}"#);
        data.push(0x0A);
        let lines = buffer.push(&data);
        assert_eq!(lines, vec!["good", r#"{"none":true}"#]);
        assert_eq!(
            lines.last().and_then(|l| parse_line(l)),
            Some(HelperMessage::None)
        );
    }

    #[test]
    fn line_buffer_keeps_multibyte_character_split_across_reads() {
        let mut buffer = LineBuffer::new();
        let bytes = "蘇\n".as_bytes();
        assert_eq!(buffer.push(&bytes[..2]), Vec::<String>::new());
        assert_eq!(buffer.push(&bytes[2..]), vec!["蘇"]);
    }

    // ---- CappedHelperLineBuffer（MediaRemoteSourceTests 內的 4 案）----

    #[test]
    fn capped_buffer_passes_lines_within_cap() {
        let mut buffer = LineBuffer::with_max(8);
        assert_eq!(buffer.push(b"abc"), Vec::<String>::new());
        assert_eq!(buffer.push(b"de\n12345678\nx"), vec!["abcde", "12345678"]);
        assert_eq!(buffer.push(b"y\n"), vec!["xy"]);
    }

    #[test]
    fn capped_buffer_discards_oversized_unterminated_line_until_newline() {
        let mut buffer = LineBuffer::with_max(8);
        assert_eq!(buffer.push(b"12345"), Vec::<String>::new());
        assert_eq!(buffer.push(b"6789"), Vec::<String>::new());
        assert_eq!(buffer.push(b"still dropped"), Vec::<String>::new());
        assert_eq!(buffer.push(b"tail\nok\n"), vec!["ok"]);
    }

    #[test]
    fn capped_buffer_discards_oversized_line_within_one_chunk() {
        let mut buffer = LineBuffer::with_max(4);
        assert_eq!(buffer.push(b"a\ntoolong\nb\n"), vec!["a", "b"]);
    }

    #[test]
    fn capped_buffer_default_cap_is_four_mib() {
        // 預設上限 4 MiB：恰好 4 MiB 的一行通過，多一個位元組就丟掉，且不影響下一行。
        let cap = 4 * 1024 * 1024;
        let mut buffer = LineBuffer::new();
        let mut exact = vec![b'a'; cap];
        exact.push(b'\n');
        let lines = buffer.push(&exact);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), cap);
        let mut over = vec![b'a'; cap + 1];
        over.extend_from_slice(b"\nnext\n");
        assert_eq!(buffer.push(&over), vec!["next"]);
    }

    // ---- Review Focus 3 ----

    #[test]
    fn line_split_across_pushes_is_reassembled() {
        let line = r#"{"title":"蘇裔非 🎵","elapsed":1,"timestamp":100,"rate":1}"#;
        let mut wire = line.as_bytes().to_vec();
        wire.push(b'\n');
        // 每個切點都試一次（包含切在多位元組字元中間），以及逐位元組餵入。
        for split in 0..=wire.len() {
            let mut buffer = LineBuffer::new();
            let mut got = buffer.push(&wire[..split]);
            got.extend(buffer.push(&wire[split..]));
            assert_eq!(got, vec![line], "split at {split}");
        }
        let mut buffer = LineBuffer::new();
        let mut got = Vec::new();
        for byte in &wire {
            got.extend(buffer.push(std::slice::from_ref(byte)));
        }
        assert_eq!(got, vec![line]);
        assert!(matches!(parse_line(&got[0]), Some(HelperMessage::Track(_))));
    }

    #[test]
    fn invalid_utf8_line_is_dropped_without_affecting_neighbours() {
        let mut buffer = LineBuffer::new();
        let mut data = b"before\n".to_vec();
        data.extend_from_slice(&[0xC3, 0x28, 0xE8, 0x98]); // 非法序列與被截斷的三位元組字元
        data.push(b'\n');
        data.extend_from_slice(b"after\n");
        assert_eq!(buffer.push(&data), vec!["before", "after"]);
        // 非法行被切在兩次 push 之間時也一樣。
        let mut buffer = LineBuffer::new();
        let mut got = buffer.push(&[0xFF, 0xFE]);
        got.extend(buffer.push(b"\nok\n"));
        assert_eq!(got, vec!["ok"]);
    }

    #[test]
    fn line_over_the_cap_is_discarded_through_its_newline() {
        // 恰好等於上限：保留。
        let mut buffer = LineBuffer::with_max(5);
        assert_eq!(buffer.push(b"12345\n"), vec!["12345"]);
        // 上限加一：丟棄，直到換行為止，換行之後恢復正常。
        assert_eq!(buffer.push(b"123456\nok\n"), vec!["ok"]);
        // 丟棄狀態跨越多次 push 延續到換行（包含剛好在換行前後切開）。
        let mut buffer = LineBuffer::with_max(5);
        assert_eq!(buffer.push(b"123"), Vec::<String>::new());
        assert_eq!(buffer.push(b"456"), Vec::<String>::new()); // 超過上限，進入丟棄
        assert_eq!(buffer.push(b"7"), Vec::<String>::new());
        assert_eq!(buffer.push(b"89"), Vec::<String>::new());
        assert_eq!(buffer.push(b"\n"), Vec::<String>::new()); // 換行只結束丟棄，不產生行
        assert_eq!(buffer.push(b"ok\n"), vec!["ok"]);
        // 分兩次湊到恰好上限：保留。
        let mut buffer = LineBuffer::with_max(5);
        assert_eq!(buffer.push(b"123"), Vec::<String>::new());
        assert_eq!(buffer.push(b"45\n"), vec!["12345"]);
        // 上限以位元組計，不是字元。
        let mut buffer = LineBuffer::with_max(3);
        assert_eq!(buffer.push("蘇\n".as_bytes()), vec!["蘇"]);
        assert_eq!(buffer.push("蘇a\n".as_bytes()), Vec::<String>::new());
        // 被丟棄的行不會讓後面緊接的合法行消失。
        assert_eq!(buffer.push(b"abcd\nabc\n"), vec!["abc"]);
    }

    #[test]
    fn empty_lines_are_skipped() {
        let mut buffer = LineBuffer::new();
        assert_eq!(buffer.push(b"\n\na\n\n\nb\n\n"), vec!["a", "b"]);
        assert_eq!(buffer.push(b"\n"), Vec::<String>::new());
        assert_eq!(buffer.push(b"c"), Vec::<String>::new());
        assert_eq!(buffer.push(b"\n\n"), vec!["c"]);
        assert_eq!(buffer.push(b""), Vec::<String>::new());
        // 空的 push 不影響進行中的行。
        let mut buffer = LineBuffer::new();
        assert_eq!(buffer.push(b"ab"), Vec::<String>::new());
        assert_eq!(buffer.push(b""), Vec::<String>::new());
        assert_eq!(buffer.push(b"c\n"), vec!["abc"]);
    }

    #[test]
    fn garbage_json_returns_none() {
        for line in [
            "not json at all",
            "{{{{",
            "}",
            "null",
            "true",
            "\u{0}",
            "{'title':'T'}",
            "{\"title\":\"T\",}",
            "\u{FEFF}{\"none\":true}x",
            "{\"none\":true} trailing",
        ] {
            assert_eq!(parse_line(line), None, "line: {line:?}");
        }
    }

    // ---- 其他邊界 ----

    #[test]
    fn truncated_json_returns_none() {
        let full = r#"{"title":"蘇裔非","artist":"A","elapsed":1,"timestamp":100,"rate":1}"#;
        // 每個真前綴（在字元邊界上）都不是合法訊息。
        for end in 0..full.len() {
            if !full.is_char_boundary(end) {
                continue;
            }
            assert_eq!(
                parse_line(&full[..end]),
                None,
                "prefix {end}: {}",
                &full[..end]
            );
        }
        assert!(parse_line(full).is_some());
    }

    #[test]
    fn non_object_json_returns_none() {
        for line in [
            "[]",
            r#"[{"none":true}]"#,
            r#"[{"title":"T","elapsed":1,"timestamp":100,"rate":1}]"#,
            "42",
            "1.5",
            r#""title""#,
            "null",
            "true",
            "false",
        ] {
            assert_eq!(parse_line(line), None, "line: {line}");
        }
    }

    #[test]
    fn none_requires_literal_true() {
        for line in [
            r#"{"none":false}"#,
            r#"{"none":1}"#,
            r#"{"none":"true"}"#,
            r#"{"none":null}"#,
            "{}",
        ] {
            assert_eq!(parse_line(line), None, "line: {line}");
        }
    }

    #[test]
    fn wrongly_typed_fields_are_handled() {
        // title 不是字串 → 整行拒絕。
        for title in ["1", "1.5", "true", "null", "[]", "{}", r#"["T"]"#] {
            let line = format!(r#"{{"title":{title},"elapsed":1,"timestamp":100,"rate":1}}"#);
            assert_eq!(parse_line(&line), None, "title: {title}");
        }
        // 數值欄位存在但不是數字（字串、null、布林、陣列、物件）→ 整行拒絕。
        for key in ["elapsed", "timestamp", "rate", "duration"] {
            for bad in [r#""1""#, "null", "true", "[]", "{}"] {
                let mut fields = vec![("elapsed", "1"), ("timestamp", "100"), ("rate", "1")];
                fields.retain(|(k, _)| *k != key);
                fields.push((key, bad));
                let body = fields
                    .iter()
                    .map(|(k, v)| format!("\"{k}\":{v}"))
                    .collect::<Vec<_>>()
                    .join(",");
                assert_eq!(
                    parse_line(&format!("{{\"title\":\"T\",{body}}}")),
                    None,
                    "{key}: {bad}"
                );
            }
        }
        // artist／album／artworkId 型別錯誤 → 視同缺值，不影響整行。
        let snapshot = track(
            r#"{"title":"T","artist":1,"album":["x"],"artworkId":5,"artworkData":"AAAA","elapsed":1,"timestamp":100,"rate":1}"#,
        );
        assert_eq!(snapshot.artist, None);
        assert_eq!(snapshot.album, None);
        assert_eq!(snapshot.artwork, None);
    }

    #[test]
    fn position_needs_elapsed_timestamp_and_rate() {
        for line in [
            r#"{"title":"T","timestamp":100,"rate":1}"#,
            r#"{"title":"T","elapsed":1,"rate":1}"#,
            r#"{"title":"T","elapsed":1,"timestamp":100}"#,
            r#"{"title":"T"}"#,
        ] {
            assert_eq!(track(line).position, None, "line: {line}");
        }
        // rate 缺值 → 不在播放。
        assert!(!track(r#"{"title":"T","elapsed":1,"timestamp":100}"#).is_playing);
        // rate 為負 → 不在播放，錨點仍保留。
        let snapshot = track(r#"{"title":"T","elapsed":1,"timestamp":100,"rate":-1}"#);
        assert!(!snapshot.is_playing);
        assert_eq!(snapshot.position.map(|p| p.rate), Some(-1.0));
        // 沒有位置也能 is_playing 以外的欄位照常。
        assert_eq!(track(r#"{"title":"T","duration":5}"#).duration, Some(5.0));
    }

    #[test]
    fn huge_number_strings_do_not_crash() {
        // 位數很多但仍在 f64 範圍內 → 照常解析。
        let digits = "9".repeat(300);
        let line = format!(r#"{{"title":"T","elapsed":{digits},"timestamp":100,"rate":1}}"#);
        let snapshot = track(&line);
        assert!(snapshot.position.unwrap().elapsed.is_finite());
        // 超出 f64 範圍（無論是整數、小數或指數寫法）→ nil。
        let huge = "9".repeat(400);
        for value in [
            huge.clone(),
            format!("{huge}.5"),
            "1e400".into(),
            "-1e400".into(),
        ] {
            let line = format!(r#"{{"title":"T","elapsed":{value},"timestamp":100,"rate":1}}"#);
            assert_eq!(parse_line(&line), None, "value: {value:.20}");
        }
        // 非常長的小數、很小的指數 → 解成 0 或極小值，不崩潰。
        let long_fraction = format!("0.{}1", "0".repeat(100_000));
        let line =
            format!(r#"{{"title":"T","elapsed":{long_fraction},"timestamp":1e-400,"rate":1}}"#);
        let _ = parse_line(&line);
        // JSON 不允許的數字寫法 → nil。
        for value in [
            "NaN",
            "Infinity",
            "-Infinity",
            "0x10",
            "01",
            "+1",
            ".5",
            "1.",
        ] {
            let line = format!(r#"{{"title":"T","elapsed":{value},"timestamp":100,"rate":1}}"#);
            assert_eq!(parse_line(&line), None, "value: {value}");
        }
    }

    #[test]
    fn deeply_nested_json_returns_none_without_crashing() {
        for depth in [200usize, 10_000, 200_000] {
            let arrays = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(parse_line(&arrays), None, "arrays depth {depth}");
            let objects = format!("{}1{}", r#"{"a":"#.repeat(depth), "}".repeat(depth));
            assert_eq!(parse_line(&objects), None, "objects depth {depth}");
            // 合法欄位之外夾帶一個過深的值：整行仍拒絕，不崩潰。
            let line = format!(
                r#"{{"title":"T","elapsed":1,"timestamp":100,"rate":1,"x":{}{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            );
            assert_eq!(parse_line(&line), None, "nested value depth {depth}");
            // 只有開頭沒有結尾（被截斷的深層巢狀）。
            assert_eq!(parse_line(&"[".repeat(depth)), None);
        }
        // 在遞迴上限之內的巢狀額外欄位無害。
        let snapshot = track(&format!(
            r#"{{"title":"T","x":{}{},"elapsed":1,"timestamp":100,"rate":1}}"#,
            "[".repeat(50),
            "]".repeat(50)
        ));
        assert_eq!(snapshot.title, "T");
    }

    #[test]
    fn invalid_base64_artwork_data_keeps_identifier_without_bytes() {
        for data in ["!!!not base64!!!", "AAA", "AA=A", "A A A A"] {
            let line = format!(
                r#"{{"title":"T","elapsed":1,"timestamp":100,"rate":1,"artworkId":"id","artworkData":"{data}"}}"#
            );
            let snapshot = track(&line);
            assert_eq!(
                snapshot.artwork,
                Some(ArtworkRef {
                    id: "id".into(),
                    data: None
                }),
                "data: {data}"
            );
        }
        // 空的 artworkId → 沒有封面，即使有資料。
        let snapshot = track(
            r#"{"title":"T","elapsed":1,"timestamp":100,"rate":1,"artworkId":"","artworkData":"AAAA"}"#,
        );
        assert_eq!(snapshot.artwork, None);
    }

    #[test]
    fn large_valid_looking_line_just_under_the_cap_parses_in_linear_time() {
        let cap = 4 * 1024 * 1024;
        let prefix = r#"{"title":"T","elapsed":1,"timestamp":100,"rate":1,"artist":""#;
        let suffix = r#""}"#;
        let filler = cap - prefix.len() - suffix.len();
        let mut wire = Vec::with_capacity(cap + 1);
        wire.extend_from_slice(prefix.as_bytes());
        wire.resize(prefix.len() + filler, b'a');
        wire.extend_from_slice(suffix.as_bytes());
        assert_eq!(wire.len(), cap);
        wire.push(b'\n');

        let started = Instant::now();
        let mut buffer = LineBuffer::new();
        let mut lines = Vec::new();
        // 以 4 KiB 一塊餵入：緩衝區不能因為反覆重掃或重新配置而變成二次方。
        for chunk in wire.chunks(4096) {
            lines.extend(buffer.push(chunk));
        }
        assert_eq!(lines.len(), 1);
        let snapshot = track(&lines[0]);
        assert_eq!(snapshot.artist.map(|a| a.len()), Some(filler));
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );

        // 同樣大小再多一個位元組：被丟棄，後面的行仍可用。
        let mut over = vec![b'a'; cap + 1];
        over.extend_from_slice(b"\n{\"none\":true}\n");
        let mut buffer = LineBuffer::new();
        let mut lines = Vec::new();
        for chunk in over.chunks(4096) {
            lines.extend(buffer.push(chunk));
        }
        assert_eq!(lines, vec![r#"{"none":true}"#]);
    }

    #[test]
    fn many_small_lines_in_one_push_are_all_returned() {
        let mut data = Vec::new();
        for i in 0..10_000 {
            data.extend_from_slice(format!("line{i}\n").as_bytes());
        }
        let lines = LineBuffer::new().push(&data);
        assert_eq!(lines.len(), 10_000);
        assert_eq!(lines[9_999], "line9999");
    }
}
