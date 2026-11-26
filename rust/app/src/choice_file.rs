//! 歌詞紀錄檔 `lyric-choices.json`：讀寫記住的歌詞版本與時間偏移（[`ChoiceBook`]）。
//!
//! 放在設定檔同一個資料夾（[`choices_path`]；`MUSICAT_CONFIG` 換掉設定檔時跟著換），只由程式寫入。
//! 沒有 OS 專屬程式碼，也不依賴 GPUI。
//!
//! # 格式
//!
//! ```json
//! {"version":1,"songs":[{"song":"<cache_key>","duration":209.9,"url":"https://…",
//!   "offsets":{"https://…":0.5},"lines":[[8.43,"…"]]}]}
//! ```
//!
//! `duration` 是曲長（秒），未知時為 `null`；`offsets` 以版本網址辨認（秒）；`lines` 的每一行是 `[秒, 文字]`。
//!
//! # 讀取規則（[`ChoiceFile::open`] 永遠成功、不修改檔案）
//!
//! - 檔案不存在 → 空紀錄、沒有警告。
//! - 讀不了、不是一般檔案、超過 [`MAX_CHOICE_BYTES`]、不是合法 JSON、版本不是 1 或沒有 `songs` 陣列
//!   → 空紀錄加一則警告。
//! - 單筆壞掉（欄位型別不對、網址不是 `https://`、沒有歌詞行、數字超出範圍）→ 只略過那筆；
//!   警告只帶略過的筆數。
//! - 偏移讀入時限制範圍（[`clamp_offset`]）。
//! - 警告永遠不含歌名、演出者或歌詞（解析器的錯誤訊息會引用原文，所以不轉述）。
//!
//! # 存檔規則
//!
//! - 原子：同目錄寫暫存檔、`sync_all`、再改名覆蓋（同設定檔，見 [`crate::config`]）；會建立缺少的上層資料夾。
//! - 整份讀不了的原檔在第一次存檔時先改名為 `lyric-choices.json.bak`（蓋掉更舊的 `.bak`）；備份失敗就不寫。
//!   備份成功後即使最後一步失敗，之後也不再備份。
//! - 略過的單筆不另外備份：下一次存檔後就不在檔案裡了。
//! - 讀入時會略過的紀錄不寫出，非有限的曲長寫成 `null`，偏移先限制範圍，所以寫出的檔案一定整份讀得回來。
//! - 路徑是資料夾、或內容超過 [`MAX_CHOICE_BYTES`] → 回 `Err`，什麼都不動。

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use musicat_core::lyrics::LyricLine;
use musicat_core::music::choices::{clamp_offset, ChoiceBook, SavedChoice};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::config::{sibling_with_suffix, write_temp};

/// 紀錄檔大小上限（讀與寫相同）；一首歌約幾 KB，正常使用到不了。
pub const MAX_CHOICE_BYTES: u64 = 64 * 1024 * 1024;

const FILE_NAME: &str = "lyric-choices.json";
const FORMAT_VERSION: u64 = 1;

/// 設定檔所在資料夾裡的 `lyric-choices.json`；設定檔路徑沒有上層時放在目前目錄。
pub fn choices_path(config: &Path) -> PathBuf {
    config
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(FILE_NAME)
}

// ------------------------------------------------------------------ 格式

/// 整份檔案。讀入時 `S` 是還沒解碼的原文，讓壞掉的單筆不拖垮整份。
#[derive(Serialize, Deserialize)]
struct Document<S> {
    version: u64,
    songs: Vec<S>,
}

/// 檔案裡的一首歌。
#[derive(Serialize, Deserialize)]
struct Song {
    song: String,
    #[serde(default)]
    duration: Option<f64>,
    url: String,
    #[serde(default)]
    offsets: BTreeMap<String, f64>,
    lines: Vec<(f64, String)>,
}

impl From<Song> for SavedChoice {
    fn from(song: Song) -> Self {
        SavedChoice {
            key: song.song,
            duration: song.duration,
            url: song.url,
            lines: song
                .lines
                .into_iter()
                .map(|(time, text)| LyricLine { time, text })
                .collect(),
            offsets: song.offsets,
        }
    }
}

impl From<SavedChoice> for Song {
    fn from(choice: SavedChoice) -> Self {
        Song {
            song: choice.key,
            duration: choice.duration,
            url: choice.url,
            offsets: choice.offsets,
            lines: choice
                .lines
                .into_iter()
                .map(|line| (line.time, line.text))
                .collect(),
        }
    }
}

/// 檢查並整理一筆紀錄；不能用的回 `None`。讀入與寫出都經過這裡。
///
/// 下游假設網址是 `https://`、歌詞行不是空的；非有限的數字寫成 JSON 會變成 `null`。
fn usable(mut choice: SavedChoice) -> Option<SavedChoice> {
    let ok = choice.url.starts_with("https://")
        && !choice.lines.is_empty()
        && choice.lines.iter().all(|line| line.time.is_finite());
    if !ok {
        return None;
    }
    choice.duration = choice.duration.filter(|d| d.is_finite());
    for offset in choice.offsets.values_mut() {
        *offset = clamp_offset(*offset);
    }
    Some(choice)
}

// ------------------------------------------------------------------ 載入

/// 讀紀錄檔。`Ok` 是紀錄與略過的筆數（檔案不存在就是空的）；`Err` 是整份讀不了的原因（不含內容）。
fn load(path: &Path) -> Result<(ChoiceBook, usize), String> {
    let unreadable = |e: io::Error| format!("無法讀取歌詞紀錄檔（{:?}）", e.kind());
    // 先看 metadata：開 FIFO 之類的特殊檔案會永遠卡住，所以只讀一般檔案。
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((ChoiceBook::default(), 0)),
        Err(e) => return Err(unreadable(e)),
    };
    if !meta.is_file() {
        return Err("歌詞紀錄檔路徑不是一般檔案".into());
    }
    let mut bytes = Vec::new();
    match File::open(path).and_then(|f| f.take(MAX_CHOICE_BYTES + 1).read_to_end(&mut bytes)) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((ChoiceBook::default(), 0)),
        Err(e) => return Err(unreadable(e)),
    }
    if bytes.len() as u64 > MAX_CHOICE_BYTES {
        return Err("歌詞紀錄檔過大，視為損毀".into());
    }
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Result<(ChoiceBook, usize), String> {
    let document: Document<&RawValue> = serde_json::from_slice(bytes)
        .map_err(|_| "歌詞紀錄檔不是有效的 JSON 或格式不符".to_string())?;
    if document.version != FORMAT_VERSION {
        return Err("歌詞紀錄檔的版本不支援".into());
    }
    let total = document.songs.len();
    let songs: Vec<SavedChoice> = document
        .songs
        .into_iter()
        .filter_map(|raw| serde_json::from_str::<Song>(raw.get()).ok())
        .filter_map(|song| usable(song.into()))
        .collect();
    let skipped = total - songs.len();
    Ok((ChoiceBook { songs }, skipped))
}

// ------------------------------------------------------------------ 存檔

fn serialize(book: &ChoiceBook) -> io::Result<String> {
    let document = Document {
        version: FORMAT_VERSION,
        songs: book
            .songs
            .iter()
            .cloned()
            .filter_map(usable)
            .map(Song::from)
            .collect(),
    };
    Ok(serde_json::to_string(&document)?)
}

/// 紀錄檔的位置，加上「損毀的原檔是否還等著備份」。
#[derive(Debug)]
pub struct ChoiceFile {
    path: PathBuf,
    /// 整份讀不了的原檔還沒備份。鎖在整次存檔期間持有，同時存檔也只會備份一次。
    backup_pending: Mutex<bool>,
    /// 改名函式；只有測試會換掉它（讓存檔的最後一步失敗）。
    rename: fn(&Path, &Path) -> io::Result<()>,
}

impl ChoiceFile {
    /// 開紀錄檔（永遠成功、不修改檔案；規則見模組說明）。回傳讀到的紀錄與給呼叫端寫日誌的警告。
    pub fn open(path: PathBuf) -> (ChoiceFile, ChoiceBook, Vec<String>) {
        let (book, warnings, broken) = match load(&path) {
            Ok((book, 0)) => (book, Vec::new(), false),
            Ok((book, skipped)) => {
                let warning = format!("歌詞紀錄檔有 {skipped} 筆紀錄無法使用，已略過");
                (book, vec![warning], false)
            }
            Err(reason) => {
                let warning = format!("{reason}；當作沒有紀錄，第一次儲存時會把原檔備份為 .bak");
                (ChoiceBook::default(), vec![warning], true)
            }
        };
        let file = ChoiceFile {
            path,
            backup_pending: Mutex::new(broken),
            rename: |from, to| fs::rename(from, to),
        };
        (file, book, warnings)
    }

    /// 原子地寫出整份紀錄（規則見模組說明）。
    pub fn save(&self, book: &ChoiceBook) -> io::Result<()> {
        let mut backup_pending = self
            .backup_pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "歌詞紀錄檔路徑是資料夾",
            ));
        }
        let contents = serialize(book)?;
        if contents.len() as u64 > MAX_CHOICE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "歌詞紀錄超過大小上限",
            ));
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }

        // 1. 先把新內容完整寫進同目錄的暫存檔：失敗時原檔原封不動。
        let tmp = write_temp(&self.path, &contents)?;

        // 2. 損毀的原檔先備份（蓋掉更舊的 .bak）。備份失敗就放棄，不能毀掉使用者的檔案。
        if *backup_pending {
            let backup = sibling_with_suffix(&self.path, ".bak").and_then(|bak| {
                match (self.rename)(&self.path, &bak) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()), // 載入後被刪掉了
                    other => other,
                }
            });
            if let Err(e) = backup {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            // 備份這一步已完成：即使下面的改名失敗，之後的重試也不得再備份一次。
            *backup_pending = false;
        }

        // 3. 改名覆蓋。
        (self.rename)(&tmp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use musicat_core::lyrics::{LyricLine, TrackIdentity};
    use musicat_core::music::choices::SavedChoice;

    // ---------------------------------------------------------------- helpers

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "musicat-choices-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("lyric-choices.json")
        }

        fn bak(&self) -> PathBuf {
            self.0.join("lyric-choices.json.bak")
        }

        fn entries(&self) -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn line(time: f64, text: &str) -> LyricLine {
        LyricLine {
            time,
            text: text.into(),
        }
    }

    fn choice(key: &str) -> SavedChoice {
        SavedChoice {
            key: key.into(),
            duration: Some(209.9),
            url: "https://test.example/1".into(),
            lines: vec![line(8.43, "first"), line(12.0, "second")],
            offsets: BTreeMap::from([("https://test.example/1".to_string(), 0.5)]),
        }
    }

    fn book_of(songs: Vec<SavedChoice>) -> ChoiceBook {
        ChoiceBook { songs }
    }

    /// 重新開檔讀回的紀錄；寫出的檔案不該帶任何警告。
    fn reload(path: &Path) -> ChoiceBook {
        let (_, book, warnings) = ChoiceFile::open(path.to_path_buf());
        assert!(warnings.is_empty(), "{warnings:?}");
        book
    }

    const SECRETS: [&str; 3] = ["SECRET-TITLE", "SECRET-ARTIST", "SECRET-LYRIC"];

    fn assert_no_content_leak(warnings: &[String]) {
        for w in warnings {
            for s in SECRETS {
                assert!(!w.contains(s), "warning leaks content: {w}");
            }
        }
    }

    // ---------------------------------------------------------------- 路徑與格式

    #[test]
    fn choices_live_next_to_the_config_file() {
        for (config, want) in [
            (
                "/home/me/.config/musicat/musicat.toml",
                "/home/me/.config/musicat/lyric-choices.json",
            ),
            // `MUSICAT_CONFIG` 換掉的設定檔（任意檔名、相對路徑）。
            ("/tmp/x/other.toml", "/tmp/x/lyric-choices.json"),
            ("rel/dir/c.toml", "rel/dir/lyric-choices.json"),
            ("musicat.toml", "lyric-choices.json"),
        ] {
            assert_eq!(
                choices_path(Path::new(config)),
                PathBuf::from(want),
                "{config}"
            );
        }
    }

    #[test]
    fn the_file_uses_the_documented_format() {
        let dir = TempDir::new();
        let (file, _, _) = ChoiceFile::open(dir.file());
        let mut unknown_length = choice("u\u{1}b");
        unknown_length.duration = None;
        unknown_length.offsets.clear();
        unknown_length.lines = vec![line(1.5, "x")];
        file.save(&book_of(vec![choice("t\u{1}a"), unknown_length]))
            .unwrap();
        assert_eq!(
            fs::read_to_string(dir.file()).unwrap(),
            concat!(
                r#"{"version":1,"songs":["#,
                r#"{"song":"t\u0001a","duration":209.9,"url":"https://test.example/1","#,
                r#""offsets":{"https://test.example/1":0.5},"#,
                r#""lines":[[8.43,"first"],[12.0,"second"]]},"#,
                r#"{"song":"u\u0001b","duration":null,"url":"https://test.example/1","#,
                r#""offsets":{},"lines":[[1.5,"x"]]}]}"#,
            )
        );
    }

    // ---------------------------------------------------------------- 讀寫

    #[test]
    fn a_saved_book_loads_back_unchanged() {
        let dir = TempDir::new();
        // 缺少的上層資料夾會被建立。
        let path = dir.0.join("a").join("b").join("lyric-choices.json");
        let track = TrackIdentity {
            title: "晴天 (Live)".into(),
            artist: "周杰倫".into(),
            album: None,
            duration: Some(269.533_333_333_333_3),
        };
        let mut first = choice(&track.cache_key());
        first.duration = track.duration;
        // 一堆要 17 位有效數字才寫得清的小數：讀回來必須一個位元都不差。
        first.lines = (0..2000)
            .map(|i| line(f64::from(i) * 0.1 / 3.0, "故事的小黃花 \"引號\" \\ \n\t ✓"))
            .collect();
        first.offsets = BTreeMap::from([
            ("https://test.example/1".to_string(), -120.0),
            ("https://test.example/2?a=1&b=%20".to_string(), 0.1 + 0.2),
        ]);
        let mut second = choice("other\u{1}song");
        second.duration = None;
        second.offsets.clear();
        second.lines = vec![line(-0.5, "")];
        let book = book_of(vec![first.clone(), second]);

        let (file, empty, _) = ChoiceFile::open(path.clone());
        assert_eq!(empty, ChoiceBook::default());
        file.save(&book).unwrap();
        let loaded = reload(&path);
        assert_eq!(loaded, book);
        assert_eq!(loaded.find(&track), Some(&first));

        // 覆蓋既有檔案：只有最終檔，沒有殘留的暫存檔，也沒有 .bak。
        let smaller = book_of(vec![choice("t\u{1}a")]);
        file.save(&smaller).unwrap();
        assert_eq!(reload(&path), smaller);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["lyric-choices.json"]);
    }

    #[test]
    fn a_missing_file_is_an_empty_book_without_warnings() {
        let dir = TempDir::new();
        let (file, book, warnings) = ChoiceFile::open(dir.file());
        assert_eq!(book, ChoiceBook::default());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(dir.entries().is_empty(), "open must not create files");
        file.save(&book_of(vec![choice("t\u{1}a")])).unwrap();
        assert_eq!(dir.entries(), ["lyric-choices.json"]);
    }

    #[test]
    fn a_broken_file_is_kept_as_bak_on_the_first_save_only() {
        let broken: [&[u8]; 8] = [
            b"{\"version\":1,\"songs\":[{\"song\":\"SECRET-TITLE\",",
            b"SECRET-LYRIC",
            b"",
            b"{\"version\":2,\"songs\":[]}",
            b"{\"version\":\"SECRET-ARTIST\",\"songs\":[]}",
            b"{\"songs\":[]}",
            b"{\"version\":1,\"songs\":{\"SECRET-TITLE\":1}}",
            b"{\"version\":1,\"songs\":[\"\xff\xfe\"]}",
        ];
        for original in broken {
            let shown = String::from_utf8_lossy(original);
            let dir = TempDir::new();
            fs::write(dir.file(), original).unwrap();
            fs::write(dir.bak(), "older backup").unwrap();

            let (file, book, warnings) = ChoiceFile::open(dir.file());
            assert_eq!(book, ChoiceBook::default(), "{shown}");
            assert_eq!(warnings.len(), 1, "{shown}: {warnings:?}");
            assert_no_content_leak(&warnings);
            // 開檔本身不動檔案。
            assert_eq!(fs::read(dir.file()).unwrap(), original, "{shown}");
            assert_eq!(fs::read(dir.bak()).unwrap(), b"older backup");

            let first = book_of(vec![choice("t\u{1}a")]);
            file.save(&first).unwrap();
            assert_eq!(fs::read(dir.bak()).unwrap(), original, "{shown}");
            assert_eq!(reload(&dir.file()), first);

            // 之後的存檔只覆蓋新檔，.bak 不再被動。
            let second = book_of(vec![choice("u\u{1}b")]);
            file.save(&second).unwrap();
            assert_eq!(fs::read(dir.bak()).unwrap(), original, "{shown}");
            assert_eq!(reload(&dir.file()), second);
            assert_eq!(
                dir.entries(),
                ["lyric-choices.json", "lyric-choices.json.bak"]
            );
        }
    }

    #[test]
    fn a_broken_file_deleted_before_the_first_save_is_not_an_error() {
        let dir = TempDir::new();
        fs::write(dir.file(), "nonsense").unwrap();
        let (file, _, _) = ChoiceFile::open(dir.file());
        fs::remove_file(dir.file()).unwrap();
        let book = book_of(vec![choice("t\u{1}a")]);
        file.save(&book).unwrap();
        assert_eq!(reload(&dir.file()), book);
        assert_eq!(dir.entries(), ["lyric-choices.json"]);
    }

    #[test]
    fn bad_entries_are_skipped_and_offsets_clamped() {
        let dir = TempDir::new();
        let good = |key: &str, extra: &str| {
            format!(
                r#"{{"song":"{key}","duration":200.0,"url":"https://test.example/1",{extra}"lines":[[1.0,"a"]]}}"#
            )
        };
        let bad = [
            // 非 https 網址。
            r#"{"song":"SECRET-TITLE","duration":null,"url":"http://test.example/1","offsets":{},"lines":[[1.0,"SECRET-LYRIC"]]}"#,
            r#"{"song":"b","duration":null,"url":"","offsets":{},"lines":[[1.0,"a"]]}"#,
            // 沒有行。
            r#"{"song":"c","duration":null,"url":"https://test.example/1","offsets":{},"lines":[]}"#,
            r#"{"song":"d","duration":null,"url":"https://test.example/1","offsets":{}}"#,
            // 非有限數字（超出 f64 的字面值，或被寫成 null）。
            r#"{"song":"e","duration":null,"url":"https://test.example/1","offsets":{},"lines":[[1e999,"a"]]}"#,
            r#"{"song":"f","duration":null,"url":"https://test.example/1","offsets":{},"lines":[[null,"a"]]}"#,
            r#"{"song":"g","duration":1e999,"url":"https://test.example/1","offsets":{},"lines":[[1.0,"a"]]}"#,
            r#"{"song":"h","duration":null,"url":"https://test.example/1","offsets":{"https://test.example/1":-1e999},"lines":[[1.0,"a"]]}"#,
            r#"{"song":"i","duration":null,"url":"https://test.example/1","offsets":{"https://test.example/1":null},"lines":[[1.0,"a"]]}"#,
            // 型別或形狀不對。
            r#"{"song":7,"duration":null,"url":"https://test.example/1","offsets":{},"lines":[[1.0,"a"]]}"#,
            r#"{"song":"j","duration":"SECRET-ARTIST","url":"https://test.example/1","offsets":{},"lines":[[1.0,"a"]]}"#,
            r#"{"song":"k","duration":null,"url":"https://test.example/1","offsets":{},"lines":[[1.0]]}"#,
            "null",
            r#""SECRET-TITLE""#,
        ];
        let mut songs = vec![good("first", "")];
        songs.extend(bad.iter().map(|s| s.to_string()));
        songs.push(good(
            "last",
            r#""offsets":{"https://test.example/1":500,"https://test.example/2":-1e6,"https://test.example/3":-2.5},"#,
        ));
        let text = format!(r#"{{"version":1,"songs":[{}]}}"#, songs.join(","));
        fs::write(dir.file(), &text).unwrap();

        let (file, book, warnings) = ChoiceFile::open(dir.file());
        let keys: Vec<&str> = book.songs.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["first", "last"]);
        assert!(book.songs[0].offsets.is_empty());
        assert_eq!(
            book.songs[1].offsets,
            BTreeMap::from([
                ("https://test.example/1".to_string(), 120.0),
                ("https://test.example/2".to_string(), -120.0),
                ("https://test.example/3".to_string(), -2.5),
            ])
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains(&bad.len().to_string()), "{warnings:?}");
        assert_no_content_leak(&warnings);

        // 整份檔案沒壞：存檔直接覆蓋，不做備份。
        file.save(&book).unwrap();
        assert_eq!(dir.entries(), ["lyric-choices.json"]);
        assert_eq!(reload(&dir.file()), book);
    }

    #[test]
    fn a_save_never_writes_an_entry_the_loader_would_reject() {
        let dir = TempDir::new();
        let (file, _, _) = ChoiceFile::open(dir.file());
        let mut songs = Vec::new();
        for (i, time) in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
            .into_iter()
            .enumerate()
        {
            let mut bad = choice(&format!("time{i}"));
            bad.lines.push(line(time, "a"));
            songs.push(bad);
        }
        let mut http = choice("http");
        http.url = "http://test.example/1".into();
        songs.push(http);
        let mut empty = choice("empty");
        empty.lines.clear();
        songs.push(empty);
        // 曲長與偏移可以整理成合法的值，那筆照寫。
        let mut odd = choice("odd");
        odd.duration = Some(f64::NAN);
        odd.offsets = BTreeMap::from([
            ("https://test.example/1".to_string(), f64::NAN),
            ("https://test.example/2".to_string(), f64::INFINITY),
            ("https://test.example/3".to_string(), -900.0),
        ]);
        songs.push(odd);

        file.save(&book_of(songs)).unwrap();
        let loaded = reload(&dir.file());
        let mut want = choice("odd");
        want.duration = None;
        want.offsets = BTreeMap::from([
            ("https://test.example/1".to_string(), 0.0),
            ("https://test.example/2".to_string(), 120.0),
            ("https://test.example/3".to_string(), -120.0),
        ]);
        assert_eq!(loaded, book_of(vec![want]));
    }

    // ---------------------------------------------------------------- 失敗

    /// 只讓「暫存檔 → 正式檔」那一步失敗的改名函式（備份那一步照常成功）。
    fn rename_failing_the_final_step(from: &Path, to: &Path) -> io::Result<()> {
        if from.to_string_lossy().contains(".tmp-") {
            Err(io::Error::other("injected failure"))
        } else {
            fs::rename(from, to)
        }
    }

    #[test]
    fn a_failed_save_leaves_the_old_file() {
        let dir = TempDir::new();
        let old = book_of(vec![choice("t\u{1}a")]);
        let (file, _, _) = ChoiceFile::open(dir.file());
        file.save(&old).unwrap();
        let bytes = fs::read(dir.file()).unwrap();

        let (mut file, _, _) = ChoiceFile::open(dir.file());
        file.rename = rename_failing_the_final_step;
        assert!(file.save(&book_of(vec![choice("u\u{1}b")])).is_err());
        assert_eq!(fs::read(dir.file()).unwrap(), bytes);
        assert_eq!(dir.entries(), ["lyric-choices.json"], "no temp file left");
    }

    #[test]
    fn the_backup_happens_once_even_if_the_final_step_then_fails() {
        let dir = TempDir::new();
        fs::write(dir.file(), "broken original").unwrap();
        let (mut file, _, _) = ChoiceFile::open(dir.file());
        file.rename = rename_failing_the_final_step;
        assert!(file.save(&book_of(vec![choice("t\u{1}a")])).is_err());
        assert_eq!(fs::read(dir.bak()).unwrap(), b"broken original");
        assert_eq!(dir.entries(), ["lyric-choices.json.bak"]);

        // 之後路徑上又出現一個檔案，重試不得再備份一次而把 .bak 裡的原檔蓋掉。
        fs::write(dir.file(), "something else").unwrap();
        file.rename = |from, to| fs::rename(from, to);
        let book = book_of(vec![choice("u\u{1}b")]);
        file.save(&book).unwrap();
        assert_eq!(fs::read(dir.bak()).unwrap(), b"broken original");
        assert_eq!(reload(&dir.file()), book);
    }

    #[test]
    fn nothing_is_written_while_the_backup_fails() {
        let dir = TempDir::new();
        fs::write(dir.file(), "broken original").unwrap();
        // 一個非空資料夾佔住 .bak 的位置：備份的改名一定失敗。
        fs::create_dir(dir.bak()).unwrap();
        fs::write(dir.bak().join("keep"), "x").unwrap();
        let (file, _, _) = ChoiceFile::open(dir.file());
        let book = book_of(vec![choice("t\u{1}a")]);
        for _ in 0..2 {
            assert!(file.save(&book).is_err());
            assert_eq!(fs::read(dir.file()).unwrap(), b"broken original");
            assert_eq!(
                dir.entries(),
                ["lyric-choices.json", "lyric-choices.json.bak"],
                "no temp file left"
            );
        }
        // 障礙排除後，下一次存檔照樣先備份。
        fs::remove_dir_all(dir.bak()).unwrap();
        file.save(&book).unwrap();
        assert_eq!(fs::read(dir.bak()).unwrap(), b"broken original");
        assert_eq!(reload(&dir.file()), book);
    }

    #[test]
    fn a_directory_at_the_path_is_refused_without_touching_it() {
        let dir = TempDir::new();
        fs::create_dir(dir.file()).unwrap();
        fs::write(dir.file().join("inner"), "keep me").unwrap();
        let (file, book, warnings) = ChoiceFile::open(dir.file());
        assert_eq!(book, ChoiceBook::default());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(file.save(&book_of(vec![choice("t\u{1}a")])).is_err());
        assert_eq!(fs::read(dir.file().join("inner")).unwrap(), b"keep me");
        assert_eq!(dir.entries(), ["lyric-choices.json"]);
    }

    #[test]
    fn the_size_cap_applies_to_loading_and_saving_alike() {
        let dir = TempDir::new();
        // 合法的 JSON（結尾補空白），剛好在上限與超過一位元組。
        let mut bytes = br#"{"version":1,"songs":[]}"#.to_vec();
        bytes.resize(MAX_CHOICE_BYTES as usize, b' ');
        fs::write(dir.file(), &bytes).unwrap();
        reload(&dir.file());
        bytes.push(b' ');
        fs::write(dir.file(), &bytes).unwrap();
        let (_, _, warnings) = ChoiceFile::open(dir.file());
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        // 讀不回來的大小就不寫，原檔不動。
        fs::write(dir.file(), br#"{"version":1,"songs":[]}"#).unwrap();
        let (file, _, _) = ChoiceFile::open(dir.file());
        let mut huge = choice("t\u{1}a");
        huge.lines = vec![line(1.0, &"x".repeat(MAX_CHOICE_BYTES as usize))];
        assert!(file.save(&book_of(vec![huge])).is_err());
        assert_eq!(
            fs::read(dir.file()).unwrap(),
            br#"{"version":1,"songs":[]}"#
        );
        assert_eq!(dir.entries(), ["lyric-choices.json"]);
    }
}
