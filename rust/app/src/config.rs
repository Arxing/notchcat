//! TOML 設定檔：載入（永遠成功）、原子存檔、損毀檔案的備份規則。
//!
//! 沒有任何 OS 專屬程式碼（唯一的平台判斷是 [`Os::current`]；路徑解析 [`config_path_from`] 對輸入是純函式），
//! 也不依賴 GPUI，所以 macOS 與 Windows 都能編譯與測試。
//!
//! # 設定檔位置
//!
//! | 平台 | 路徑 |
//! |---|---|
//! | macOS | `$HOME/Library/Application Support/MusiCat/musicat.toml` |
//! | Windows | `%APPDATA%\MusiCat\musicat.toml` |
//! | 其他 | `$XDG_CONFIG_HOME/musicat/musicat.toml`（需為絕對路徑），否則 `$HOME/.config/musicat/musicat.toml` |
//!
//! # 格式
//!
//! ```toml
//! # MusiCat 設定檔 — 由程式寫入；格式見 README
//! mode = "notch"                 # 目前只有 "notch"
//! screen = "1"                   # 選填：顯示在哪個螢幕（螢幕 id）；省略 = 自動
//! lyric_strip_visible = true     # 瀏海下方的歌詞條
//!
//! [sources]                      # 歌詞來源開關
//! lrclib = true
//! netease = true
//!
//! [macos]
//! perl_host = true               # 以 perl 宿主讀取現正播放（false 時只剩視窗標題備援）
//!
//! [notch]                        # 沒有實體瀏海的螢幕上畫的虛擬瀏海
//! virtual_width = 200            # 夾在 40..=2000
//! virtual_height = 32            # 夾在 10..=400
//! ```
//!
//! # 讀取規則（[`load`] 永遠不失敗、不 panic）
//!
//! - 檔案不存在 → 全部預設值、沒有警告（[`LoadSource::Missing`]）。
//! - TOML 語法錯誤、不是 UTF-8、超過 1 MiB、不是一般檔案或讀不了 → 全部預設值加一則警告
//!   （[`LoadSource::Broken`]）。**`load` 不會動檔案**。
//! - 未知的鍵 → 忽略並警告（警告只帶鍵名，永遠不帶值）。
//! - 型別錯誤的值 → 只有那個欄位退回預設值並警告，其他欄位照常讀。
//! - 虛擬瀏海尺寸超出範圍 → 夾到範圍內並警告；NaN → 預設值並警告。
//! - `mode` 值不認得 → 預設值並警告；`screen = ""` 視為沒有（自動）。
//! - 設定檔裡的螢幕 id 對不對得上目前接的螢幕，**這一層不檢查**：暫時拔掉螢幕不會讓使用者的選擇消失。
//!
//! # 存檔規則
//!
//! - 原子：同目錄寫暫存檔、`sync_all`、再改名覆蓋；中途失敗不會留下半截檔案（暫存檔會被清掉）。會建立缺少的上層資料夾。
//! - **損毀檔案規則**：檔案是以 [`LoadSource::Broken`] 載入的，第一次存檔會先把原檔改名為 `musicat.toml.bak`
//!   （蓋掉更舊的 `.bak`）再寫入新檔，使用者手寫到一半的檔案不會被悄悄毀掉；同一次執行之後的存檔直接覆蓋。
//!   新檔的暫存檔先寫好才備份，所以寫入失敗時原檔原封不動。
//! - **有效的檔案**存檔時會被重寫成標準格式：**註解與未知的鍵會遺失**（這一層不保留它們），
//!   也不會有 `.bak`。想保留手寫註解的人請把設定放在版本控制裡。
//! - 路徑是資料夾 → 存檔回 `Err`，什麼都不動。

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml::{Table, Value};

/// 設定檔大小上限；更大的檔案視為損毀（不讀進記憶體）。
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
/// 單次載入最多回報的警告數；超過的合併成一則「另有 N 則」。
pub const MAX_WARNINGS: usize = 20;
/// 警告裡顯示的鍵名最長字元數。
const MAX_KEY_CHARS: usize = 48;

/// 虛擬瀏海寬度的合理範圍。
pub const VIRTUAL_WIDTH_RANGE: (f64, f64) = (40.0, 2000.0);
/// 虛擬瀏海高度的合理範圍。
pub const VIRTUAL_HEIGHT_RANGE: (f64, f64) = (10.0, 400.0);

const FILE_HEADER: &str = "# MusiCat 設定檔 — 由程式寫入；格式見 README";

/// 顯示模式。R4 會加入 `FloatingWindow`。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Notch,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Notch => "notch",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        s.eq_ignore_ascii_case("notch").then_some(Mode::Notch)
    }
}

/// 歌詞來源開關。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sources {
    pub lrclib: bool,
    pub netease: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            lrclib: true,
            netease: true,
        }
    }
}

/// macOS 專屬設定（其他平台讀得到、存得回去，但沒有作用）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MacosConfig {
    /// 以 perl 宿主讀取現正播放；關閉時只剩視窗標題備援。
    pub perl_host: bool,
}

impl Default for MacosConfig {
    fn default() -> Self {
        Self { perl_host: true }
    }
}

/// 沒有實體瀏海的螢幕上畫的虛擬瀏海大小。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NotchConfig {
    pub virtual_width: f64,
    pub virtual_height: f64,
}

impl Default for NotchConfig {
    fn default() -> Self {
        Self {
            virtual_width: 200.0,
            virtual_height: 32.0,
        }
    }
}

/// 使用者設定。
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub mode: Mode,
    /// 使用者選的螢幕 id；`None` = 自動。可能是目前接不上的螢幕（見模組說明）。
    pub screen: Option<String>,
    pub sources: Sources,
    pub macos: MacosConfig,
    pub notch: NotchConfig,
    /// 歌詞條是否顯示（對應 Swift 的 `LyricStripPreference.isVisible`）。
    pub lyric_strip_visible: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Notch,
            screen: None,
            sources: Sources::default(),
            macos: MacosConfig::default(),
            notch: NotchConfig::default(),
            lyric_strip_visible: true,
        }
    }
}

// ------------------------------------------------------------------ 路徑

/// 路徑解析用的作業系統分類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    MacOs,
    Windows,
    Other,
}

impl Os {
    /// 目前編譯的目標平台。整個 app 函式庫唯一的平台判斷。
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// 設定檔路徑（對 `os` 與環境變數查詢是純函式，所以每個平台都能在任何主機上測）。
/// 需要的環境變數缺少或為空字串 → `None`。
pub fn config_path_from(os: Os, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |name: &str| env(name).filter(|v| !v.is_empty());
    match os {
        Os::MacOs => Some(
            PathBuf::from(var("HOME")?)
                .join("Library")
                .join("Application Support")
                .join("MusiCat")
                .join("musicat.toml"),
        ),
        Os::Windows => Some(
            PathBuf::from(var("APPDATA")?)
                .join("MusiCat")
                .join("musicat.toml"),
        ),
        Os::Other => {
            // XDG 規範：相對路徑無效，忽略。
            let base = match var("XDG_CONFIG_HOME").map(PathBuf::from) {
                Some(p) if p.is_absolute() => p,
                _ => PathBuf::from(var("HOME")?).join(".config"),
            };
            Some(base.join("musicat").join("musicat.toml"))
        }
    }
}

/// 目前平台與行程環境的設定檔路徑；找不到家目錄（或 `%APPDATA%`）時為 `None`。
pub fn config_path() -> Option<PathBuf> {
    config_path_from(Os::current(), &|name| std::env::var(name).ok())
}

// ------------------------------------------------------------------ 載入

/// 設定來自哪裡。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadSource {
    /// 沒有檔案。
    Missing,
    /// 讀到並解析了（個別欄位可能因警告退回預設值）。
    Loaded,
    /// 有檔案但整份讀不了；全部用預設值。第一次存檔會備份原檔。
    Broken,
}

#[derive(Clone, PartialEq, Debug)]
pub struct LoadResult {
    pub config: Config,
    /// 給人看的警告（寫日誌用）。只帶鍵名與位置，不帶值。
    pub warnings: Vec<String>,
    pub source: LoadSource,
}

impl LoadResult {
    fn defaults(source: LoadSource, warnings: Vec<String>) -> Self {
        Self {
            config: Config::default(),
            warnings,
            source,
        }
    }
}

/// 讀設定檔。永遠成功、不 panic、不修改檔案（規則見模組說明）。
pub fn load(path: &Path) -> LoadResult {
    // 先看 metadata：開 FIFO 之類的特殊檔案會永遠卡住，所以只讀一般檔案。
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return LoadResult::defaults(LoadSource::Missing, Vec::new());
        }
        Err(e) => return broken(format!("無法讀取設定檔（{:?}）", e.kind())),
    };
    if !meta.is_file() {
        return broken("設定檔路徑不是一般檔案".into());
    }
    let mut bytes = Vec::new();
    let read = File::open(path).and_then(|f| f.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes));
    if let Err(e) = read {
        return if e.kind() == io::ErrorKind::NotFound {
            LoadResult::defaults(LoadSource::Missing, Vec::new())
        } else {
            broken(format!("無法讀取設定檔（{:?}）", e.kind()))
        };
    }
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return broken("設定檔超過 1 MiB，視為損毀".into());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return broken("設定檔不是有效的 UTF-8".into());
    };
    parse(&text)
}

fn broken(reason: String) -> LoadResult {
    LoadResult::defaults(
        LoadSource::Broken,
        vec![format!(
            "{reason}；改用預設值，第一次儲存設定時會把原檔備份為 .bak"
        )],
    )
}

/// 解析設定文字（已確定是 UTF-8、大小合理）。
fn parse(text: &str) -> LoadResult {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let table: Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            // 錯誤訊息會引用原文，可能含值；只取行號。
            let line = e.span().map(|s| {
                1 + text.as_bytes()[..s.start.min(text.len())]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count()
            });
            let place = line.map_or(String::new(), |l| format!("（第 {l} 行附近）"));
            return broken(format!("設定檔 TOML 語法錯誤{place}"));
        }
    };

    let mut w = Warnings::default();
    let mut cfg = Config::default();

    w.unknown_keys(
        &table,
        &[
            "mode",
            "screen",
            "lyric_strip_visible",
            "sources",
            "macos",
            "notch",
        ],
        "",
    );

    if let Some(v) = table.get("mode") {
        match v {
            Value::String(s) => match Mode::parse(s) {
                Some(m) => cfg.mode = m,
                None => w.push("`mode` 的值無法辨識，改用預設值".into()),
            },
            _ => w.wrong_type("mode", "字串"),
        }
    }
    if let Some(v) = table.get("screen") {
        match v {
            Value::String(s) if s.is_empty() => {}
            Value::String(s) => cfg.screen = Some(s.clone()),
            _ => w.wrong_type("screen", "字串"),
        }
    }
    cfg.lyric_strip_visible = w.boolean(&table, "lyric_strip_visible", "", true);

    if let Some(t) = w.section(&table, "sources") {
        w.unknown_keys(t, &["lrclib", "netease"], "sources");
        cfg.sources.lrclib = w.boolean(t, "lrclib", "sources", true);
        cfg.sources.netease = w.boolean(t, "netease", "sources", true);
    }
    if let Some(t) = w.section(&table, "macos") {
        w.unknown_keys(t, &["perl_host"], "macos");
        cfg.macos.perl_host = w.boolean(t, "perl_host", "macos", true);
    }
    if let Some(t) = w.section(&table, "notch") {
        w.unknown_keys(t, &["virtual_width", "virtual_height"], "notch");
        let d = NotchConfig::default();
        cfg.notch.virtual_width = w.size(t, "virtual_width", d.virtual_width, VIRTUAL_WIDTH_RANGE);
        cfg.notch.virtual_height =
            w.size(t, "virtual_height", d.virtual_height, VIRTUAL_HEIGHT_RANGE);
    }

    LoadResult {
        config: cfg,
        warnings: w.finish(),
        source: LoadSource::Loaded,
    }
}

#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    dropped: usize,
}

impl Warnings {
    fn push(&mut self, msg: String) {
        if self.list.len() < MAX_WARNINGS {
            self.list.push(msg);
        } else {
            self.dropped += 1;
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            let n = self.dropped;
            self.list.push(format!("另有 {n} 則警告已省略"));
        }
        self.list
    }

    fn wrong_type(&mut self, name: &str, expected: &str) {
        self.push(format!("`{name}` 的型別不對（應為{expected}），改用預設值"));
    }

    /// 鍵名只用來辨識；截短並以 Debug 跳脫，避免超長或帶控制字元的鍵洗版日誌。
    fn unknown_keys(&mut self, table: &Table, known: &[&str], prefix: &str) {
        for key in table.keys().filter(|k| !known.contains(&k.as_str())) {
            let shown: String = key.chars().take(MAX_KEY_CHARS).collect();
            let ellipsis = if key.chars().count() > MAX_KEY_CHARS {
                "…"
            } else {
                ""
            };
            let dotted = if prefix.is_empty() {
                format!("{shown:?}")
            } else {
                format!("{prefix}.{shown:?}")
            };
            self.push(format!("未知的設定鍵 {dotted}{ellipsis}，已忽略"));
        }
    }

    fn section<'a>(&mut self, table: &'a Table, name: &str) -> Option<&'a Table> {
        match table.get(name)? {
            Value::Table(t) => Some(t),
            _ => {
                self.wrong_type(name, "表格");
                None
            }
        }
    }

    fn boolean(&mut self, table: &Table, key: &str, prefix: &str, default: bool) -> bool {
        match table.get(key) {
            None => default,
            Some(Value::Boolean(b)) => *b,
            Some(_) => {
                self.wrong_type(&dotted(prefix, key), "布林值");
                default
            }
        }
    }

    fn size(&mut self, table: &Table, key: &str, default: f64, range: (f64, f64)) -> f64 {
        let name = dotted("notch", key);
        let value = match table.get(key) {
            None => return default,
            Some(Value::Integer(i)) => *i as f64,
            Some(Value::Float(f)) => *f,
            Some(_) => {
                self.wrong_type(&name, "數字");
                return default;
            }
        };
        if value.is_nan() {
            self.push(format!("`{name}` 不是有效的數字，改用預設值"));
            return default;
        }
        let clamped = value.clamp(range.0, range.1);
        if clamped != value {
            self.push(format!(
                "`{name}` 超出範圍 {}..={}，已調整為合理值",
                range.0, range.1
            ));
        }
        clamped
    }
}

fn dotted(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

// ------------------------------------------------------------------ 存檔

/// 把設定寫成標準格式的文字。虛擬瀏海尺寸先夾進範圍（NaN → 預設值），所以寫出的檔案一定讀得回來。
fn serialize(cfg: &Config) -> String {
    let d = NotchConfig::default();
    let width = sanitize_size(
        cfg.notch.virtual_width,
        d.virtual_width,
        VIRTUAL_WIDTH_RANGE,
    );
    let height = sanitize_size(
        cfg.notch.virtual_height,
        d.virtual_height,
        VIRTUAL_HEIGHT_RANGE,
    );

    let mut out = String::new();
    let _ = writeln!(out, "{FILE_HEADER}");
    let _ = writeln!(out, "mode = {}", Value::String(cfg.mode.as_str().into()));
    if let Some(screen) = cfg.screen.as_deref().filter(|s| !s.is_empty()) {
        let _ = writeln!(out, "screen = {}", Value::String(screen.into()));
    }
    let _ = writeln!(out, "lyric_strip_visible = {}", cfg.lyric_strip_visible);
    let _ = writeln!(out, "\n[sources]");
    let _ = writeln!(out, "lrclib = {}", cfg.sources.lrclib);
    let _ = writeln!(out, "netease = {}", cfg.sources.netease);
    let _ = writeln!(out, "\n[macos]");
    let _ = writeln!(out, "perl_host = {}", cfg.macos.perl_host);
    let _ = writeln!(out, "\n[notch]");
    let _ = writeln!(out, "virtual_width = {width:?}");
    let _ = writeln!(out, "virtual_height = {height:?}");
    out
}

fn sanitize_size(value: f64, default: f64, range: (f64, f64)) -> f64 {
    if value.is_nan() {
        default
    } else {
        value.clamp(range.0, range.1)
    }
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "設定檔路徑沒有檔名"))?
        .to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

/// 原子存檔（規則見模組說明）。`load_source` 是這份設定當初載入的結果：
/// `Broken` 時，原檔會先被改名為 `.bak`。
pub fn save(path: &Path, cfg: &Config, load_source: LoadSource) -> io::Result<()> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "設定檔路徑是資料夾",
        ));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    // 1. 先把新內容完整寫進同目錄的暫存檔：失敗時原檔原封不動。
    let tmp = write_temp(path, &serialize(cfg))?;

    // 2. 損毀的原檔先備份（蓋掉更舊的 .bak）。備份失敗就放棄，不能毀掉使用者的檔案。
    if load_source == LoadSource::Broken {
        let backup =
            sibling_with_suffix(path, ".bak").and_then(|bak| match fs::rename(path, bak) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()), // 載入後被使用者刪掉了
                other => other,
            });
        if let Err(e) = backup {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }

    // 3. 改名覆蓋。
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn write_temp(path: &Path, contents: &str) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = sibling_with_suffix(path, &suffix)?;
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 設定檔加上記憶體中的設定：載入一次、每次更新就存檔，並記得「損毀檔案的備份是否已做過」。
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    cfg: Config,
    source: LoadSource,
    backed_up: bool,
}

impl ConfigStore {
    /// 載入 `path`（永遠成功）；回傳警告給呼叫端寫日誌。
    pub fn open(path: impl Into<PathBuf>) -> (Self, Vec<String>) {
        let path = path.into();
        let LoadResult {
            config,
            warnings,
            source,
        } = load(&path);
        (
            Self {
                path,
                cfg: config,
                source,
                backed_up: false,
            },
            warnings,
        )
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 套用變更並存檔。存檔失敗回 `Err`，但記憶體中的變更保留（程式照常用新設定，只是沒存下來）。
    pub fn update(&mut self, change: impl FnOnce(&mut Config)) -> io::Result<()> {
        change(&mut self.cfg);
        let needs_backup = self.source == LoadSource::Broken && !self.backed_up;
        let source = if needs_backup {
            LoadSource::Broken
        } else {
            LoadSource::Loaded
        };
        save(&self.path, &self.cfg, source)?;
        if needs_backup {
            self.backed_up = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    // ---------------------------------------------------------------- helpers

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "musicat-config-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("musicat.toml")
        }

        fn bak(&self) -> PathBuf {
            self.0.join("musicat.toml.bak")
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

    fn write(path: &Path, text: impl AsRef<[u8]>) {
        fs::write(path, text).unwrap();
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn non_default() -> Config {
        Config {
            mode: Mode::Notch,
            screen: Some("ext-9".into()),
            sources: Sources {
                lrclib: false,
                netease: true,
            },
            macos: MacosConfig { perl_host: false },
            notch: NotchConfig {
                virtual_width: 240.5,
                virtual_height: 40.0,
            },
            lyric_strip_visible: false,
        }
    }

    fn assert_no_value_leak(warnings: &[String], secrets: &[&str]) {
        for w in warnings {
            for s in secrets {
                assert!(!w.contains(s), "warning leaks a value: {w}");
            }
        }
    }

    // ---------------------------------------------------------------- 路徑

    #[test]
    fn macos_path_is_under_application_support() {
        let env = env_of(&[("HOME", "/Users/me")]);
        assert_eq!(
            config_path_from(Os::MacOs, &env),
            Some(
                PathBuf::from("/Users/me")
                    .join("Library")
                    .join("Application Support")
                    .join("MusiCat")
                    .join("musicat.toml")
            )
        );
        assert_eq!(config_path_from(Os::MacOs, &env_of(&[])), None);
    }

    #[test]
    fn windows_path_is_under_appdata() {
        let env = env_of(&[("APPDATA", r"C:\Users\me\AppData\Roaming"), ("HOME", "/x")]);
        assert_eq!(
            config_path_from(Os::Windows, &env),
            Some(
                PathBuf::from(r"C:\Users\me\AppData\Roaming")
                    .join("MusiCat")
                    .join("musicat.toml")
            )
        );
        assert_eq!(
            config_path_from(Os::Windows, &env_of(&[("HOME", "/x")])),
            None
        );
    }

    #[test]
    fn other_path_prefers_xdg_config_home_then_home_dot_config() {
        let xdg = env_of(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/me")]);
        assert_eq!(
            config_path_from(Os::Other, &xdg),
            Some(PathBuf::from("/xdg/musicat/musicat.toml"))
        );
        let home = env_of(&[("HOME", "/home/me")]);
        assert_eq!(
            config_path_from(Os::Other, &home),
            Some(PathBuf::from("/home/me/.config/musicat/musicat.toml"))
        );
        assert_eq!(config_path_from(Os::Other, &env_of(&[])), None);
    }

    #[test]
    fn empty_or_relative_environment_values_are_ignored() {
        let empty = env_of(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/me")]);
        assert_eq!(
            config_path_from(Os::Other, &empty),
            Some(PathBuf::from("/home/me/.config/musicat/musicat.toml"))
        );
        let relative = env_of(&[("XDG_CONFIG_HOME", "rel/dir"), ("HOME", "/home/me")]);
        assert_eq!(
            config_path_from(Os::Other, &relative),
            Some(PathBuf::from("/home/me/.config/musicat/musicat.toml"))
        );
        assert_eq!(config_path_from(Os::MacOs, &env_of(&[("HOME", "")])), None);
    }

    #[test]
    fn config_path_uses_the_process_environment() {
        // 不改環境變數（會干擾平行測試）：只確認有 HOME 時結果以 musicat.toml 結尾。
        if let Some(p) = config_path() {
            assert_eq!(p.file_name().unwrap(), "musicat.toml");
        }
    }

    // ---------------------------------------------------------------- 預設與載入

    #[test]
    fn defaults_match_the_documented_values() {
        let c = Config::default();
        assert_eq!(c.mode, Mode::Notch);
        assert_eq!(c.screen, None);
        assert!(c.sources.lrclib && c.sources.netease);
        assert!(c.macos.perl_host);
        assert_eq!(
            (c.notch.virtual_width, c.notch.virtual_height),
            (200.0, 32.0)
        );
        assert!(c.lyric_strip_visible);
    }

    #[test]
    fn missing_file_yields_defaults_without_warnings() {
        let dir = TempDir::new();
        let r = load(&dir.file());
        assert_eq!(r.config, Config::default());
        assert!(r.warnings.is_empty());
        assert_eq!(r.source, LoadSource::Missing);
        assert!(dir.entries().is_empty(), "load must not create files");
    }

    #[test]
    fn syntactically_broken_toml_yields_defaults_and_a_warning_and_does_not_overwrite_the_file() {
        let dir = TempDir::new();
        let original = b"mode = \"notch\"\nsecret_token = [1, 2\n[[[ nonsense = = =\n";
        write(&dir.file(), original);

        let r = load(&dir.file());
        assert_eq!(r.config, Config::default());
        assert_eq!(r.source, LoadSource::Broken);
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        assert_no_value_leak(&r.warnings, &["secret_token"]);
        // load 本身不動檔案。
        assert_eq!(fs::read(dir.file()).unwrap(), original);
        assert_eq!(dir.entries(), ["musicat.toml"]);

        // 第一次存檔：原檔變 .bak（位元組相同），新檔是有效的設定。
        let (mut store, warnings) = ConfigStore::open(dir.file());
        assert_eq!(warnings.len(), 1);
        store.update(|c| c.sources.lrclib = false).unwrap();
        assert_eq!(fs::read(dir.bak()).unwrap(), original);
        let reloaded = load(&dir.file());
        assert_eq!(reloaded.source, LoadSource::Loaded);
        assert!(!reloaded.config.sources.lrclib);

        // 同一次執行之後的存檔只覆蓋新檔，.bak 不再被動。
        store.update(|c| c.sources.netease = false).unwrap();
        assert_eq!(fs::read(dir.bak()).unwrap(), original);
        assert!(!load(&dir.file()).config.sources.netease);
        assert_eq!(dir.entries(), ["musicat.toml", "musicat.toml.bak"]);
    }

    #[test]
    fn the_backup_replaces_an_older_bak() {
        let dir = TempDir::new();
        write(&dir.bak(), "older backup");
        write(&dir.file(), "broken = = =");
        let (mut store, _) = ConfigStore::open(dir.file());
        store.update(|c| c.lyric_strip_visible = false).unwrap();
        assert_eq!(fs::read(dir.bak()).unwrap(), b"broken = = =");
    }

    #[test]
    fn a_valid_file_is_rewritten_in_place_without_a_bak() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            "# my comment\nlyric_strip_visible = false\nfuture_key = 1\n",
        );
        let (mut store, warnings) = ConfigStore::open(dir.file());
        assert_eq!(warnings.len(), 1); // future_key
        store.update(|c| c.macos.perl_host = false).unwrap();
        assert_eq!(dir.entries(), ["musicat.toml"]);
        let text = fs::read_to_string(dir.file()).unwrap();
        assert!(!text.contains("my comment") && !text.contains("future_key"));
        let r = load(&dir.file());
        assert!(r.warnings.is_empty());
        assert!(!r.config.lyric_strip_visible && !r.config.macos.perl_host);
    }

    #[test]
    fn a_broken_file_deleted_before_the_first_save_is_not_an_error() {
        let dir = TempDir::new();
        write(&dir.file(), "= = =");
        let (mut store, _) = ConfigStore::open(dir.file());
        fs::remove_file(dir.file()).unwrap();
        store.update(|c| c.lyric_strip_visible = false).unwrap();
        assert!(!load(&dir.file()).config.lyric_strip_visible);
        assert!(!dir.bak().exists());
    }

    #[test]
    fn unknown_keys_are_ignored_with_a_warning() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            "colour = \"SECRET-VALUE\"\nlyric_strip_visible = false\n\
             [sources]\nbing = true\nlrclib = false\n\
             [extra]\nx = \"SECRET-VALUE\"\n\
             [macos]\nwayland = 1\n",
        );
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Loaded);
        // 已知欄位照常生效。
        assert!(!r.config.lyric_strip_visible);
        assert!(!r.config.sources.lrclib);
        assert!(r.config.sources.netease);
        // 每個未知鍵一則警告，且只含鍵名。
        let joined = r.warnings.join("\n");
        for key in ["colour", "bing", "extra", "wayland"] {
            assert!(joined.contains(key), "{key} missing from {joined}");
        }
        assert_eq!(r.warnings.len(), 4, "{:?}", r.warnings);
        assert_no_value_leak(&r.warnings, &["SECRET-VALUE"]);
    }

    #[test]
    fn many_or_huge_unknown_keys_do_not_flood_the_warnings() {
        let dir = TempDir::new();
        let mut text = String::new();
        for i in 0..5000 {
            text.push_str(&format!("junk_{i} = {i}\n"));
        }
        text.push_str(&format!("{} = 1\n", "k".repeat(10_000)));
        write(&dir.file(), text);
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Loaded);
        assert!(r.warnings.len() <= MAX_WARNINGS + 1, "{}", r.warnings.len());
        assert!(r.warnings.iter().all(|w| w.chars().count() < 200));
        assert!(r.warnings.last().unwrap().contains("省略"));
    }

    #[test]
    fn wrong_typed_value_falls_back_to_that_fields_default_only() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            "lyric_strip_visible = \"yes\"\nscreen = \"ext\"\n\
             [sources]\nlrclib = 1\nnetease = false\n\
             [macos]\nperl_host = false\n\
             [notch]\nvirtual_width = \"wide\"\nvirtual_height = 50\n",
        );
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Loaded);
        let c = &r.config;
        assert!(c.lyric_strip_visible, "wrong type -> default (true)");
        assert!(c.sources.lrclib, "wrong type -> default (true)");
        assert!(!c.sources.netease, "other fields keep their value");
        assert!(!c.macos.perl_host);
        assert_eq!(c.notch.virtual_width, 200.0, "wrong type -> default");
        assert_eq!(c.notch.virtual_height, 50.0);
        assert_eq!(c.screen.as_deref(), Some("ext"));
        assert_eq!(r.warnings.len(), 3, "{:?}", r.warnings);
        let joined = r.warnings.join("\n");
        for key in [
            "lyric_strip_visible",
            "sources.lrclib",
            "notch.virtual_width",
        ] {
            assert!(joined.contains(key), "{key} missing from {joined}");
        }
        assert_no_value_leak(&r.warnings, &["wide", "yes"]);
    }

    #[test]
    fn a_section_of_the_wrong_type_resets_only_that_section() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            "sources = 5\nscreen = 7\nmode = 3\n[macos]\nperl_host = false\n",
        );
        let r = load(&dir.file());
        assert_eq!(r.config.sources, Sources::default());
        assert_eq!(r.config.screen, None);
        assert!(!r.config.macos.perl_host);
        assert_eq!(r.warnings.len(), 3, "{:?}", r.warnings);
    }

    #[test]
    fn mode_values() {
        let dir = TempDir::new();
        write(&dir.file(), "mode = \"NOTCH\"\n");
        let r = load(&dir.file());
        assert_eq!(r.config.mode, Mode::Notch);
        assert!(r.warnings.is_empty());

        write(&dir.file(), "mode = \"floating-SECRET\"\n");
        let r = load(&dir.file());
        assert_eq!(r.config.mode, Mode::Notch);
        assert_eq!(r.warnings.len(), 1);
        assert_no_value_leak(&r.warnings, &["floating-SECRET"]);
    }

    #[test]
    fn empty_screen_means_auto() {
        let dir = TempDir::new();
        write(&dir.file(), "screen = \"\"\n");
        let r = load(&dir.file());
        assert_eq!(r.config.screen, None);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn screen_id_that_no_longer_exists_is_kept_in_the_file() {
        // 這一層從不對照目前接的螢幕：id 照樣讀進來、改別的欄位存檔時照樣寫回去。
        let dir = TempDir::new();
        write(&dir.file(), "screen = \"disconnected-monitor-42\"\n");
        let (mut store, warnings) = ConfigStore::open(dir.file());
        assert!(warnings.is_empty());
        assert_eq!(
            store.config().screen.as_deref(),
            Some("disconnected-monitor-42")
        );
        store.update(|c| c.lyric_strip_visible = false).unwrap();
        let r = load(&dir.file());
        assert_eq!(r.config.screen.as_deref(), Some("disconnected-monitor-42"));
        assert!(fs::read_to_string(dir.file())
            .unwrap()
            .contains("disconnected-monitor-42"));
    }

    #[test]
    fn out_of_range_and_non_finite_virtual_sizes_are_clamped_with_a_warning() {
        let cases: [(&str, f64, f64, usize); 6] = [
            // (toml, width, height, warnings)
            ("virtual_width = 0\nvirtual_height = -5", 40.0, 10.0, 2),
            (
                "virtual_width = 1000000000\nvirtual_height = 1e9",
                2000.0,
                400.0,
                2,
            ),
            (
                "virtual_width = inf\nvirtual_height = -inf",
                2000.0,
                10.0,
                2,
            ),
            ("virtual_width = nan\nvirtual_height = nan", 200.0, 32.0, 2),
            ("virtual_width = 3000.5", 2000.0, 32.0, 1),
            ("virtual_width = 40\nvirtual_height = 400", 40.0, 400.0, 0),
        ];
        for (body, w, h, warnings) in cases {
            let dir = TempDir::new();
            write(&dir.file(), format!("[notch]\n{body}\n"));
            let r = load(&dir.file());
            assert_eq!(r.source, LoadSource::Loaded, "{body}");
            assert_eq!(
                (r.config.notch.virtual_width, r.config.notch.virtual_height),
                (w, h),
                "{body}"
            );
            assert_eq!(r.warnings.len(), warnings, "{body}: {:?}", r.warnings);
        }
    }

    #[test]
    fn a_non_finite_size_in_memory_is_sanitised_when_saved() {
        let dir = TempDir::new();
        let mut cfg = Config::default();
        cfg.notch.virtual_width = f64::NAN;
        cfg.notch.virtual_height = f64::INFINITY;
        save(&dir.file(), &cfg, LoadSource::Missing).unwrap();
        let r = load(&dir.file());
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.config.notch.virtual_width, 200.0);
        assert_eq!(r.config.notch.virtual_height, 400.0);
    }

    #[test]
    fn empty_file_and_comment_only_file_are_valid_defaults() {
        let dir = TempDir::new();
        for body in ["", "\n\n", "# just a comment\n# another\n"] {
            write(&dir.file(), body);
            let r = load(&dir.file());
            assert_eq!(r.config, Config::default());
            assert!(r.warnings.is_empty(), "{body:?}");
            assert_eq!(r.source, LoadSource::Loaded);
        }
    }

    #[test]
    fn bom_and_crlf_are_accepted() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            "\u{feff}lyric_strip_visible = false\r\n[macos]\r\nperl_host = false\r\n",
        );
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Loaded);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(!r.config.lyric_strip_visible && !r.config.macos.perl_host);
    }

    #[test]
    fn a_five_megabyte_file_is_broken_without_being_read_whole() {
        let dir = TempDir::new();
        write(&dir.file(), vec![b'x'; 5 * 1024 * 1024]);
        let start = Instant::now();
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Broken);
        assert_eq!(r.config, Config::default());
        assert_eq!(r.warnings.len(), 1);
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn the_size_cap_is_exactly_one_mebibyte() {
        let dir = TempDir::new();
        // 有效的 TOML（全是註解），剛好在上限與超過一位元組。
        let mut at_cap = vec![b'#'; MAX_CONFIG_BYTES as usize];
        *at_cap.last_mut().unwrap() = b'\n';
        write(&dir.file(), &at_cap);
        assert_eq!(load(&dir.file()).source, LoadSource::Loaded);
        at_cap.push(b'\n');
        write(&dir.file(), &at_cap);
        assert_eq!(load(&dir.file()).source, LoadSource::Broken);
    }

    #[test]
    fn non_utf8_bytes_are_broken() {
        let dir = TempDir::new();
        write(
            &dir.file(),
            b"lyric_strip_visible = false\n# \xff\xfe\xfd\n",
        );
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Broken);
        assert_eq!(r.config, Config::default());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn a_directory_at_the_path_is_broken_and_save_refuses_without_touching_it() {
        let dir = TempDir::new();
        fs::create_dir(dir.file()).unwrap();
        fs::write(dir.file().join("inner"), "keep me").unwrap();
        let r = load(&dir.file());
        assert_eq!(r.source, LoadSource::Broken);
        assert_eq!(r.config, Config::default());

        let (mut store, _) = ConfigStore::open(dir.file());
        assert!(store.update(|c| c.lyric_strip_visible = false).is_err());
        assert!(
            !store.config().lyric_strip_visible,
            "in-memory change is kept"
        );
        assert_eq!(fs::read(dir.file().join("inner")).unwrap(), b"keep me");
        assert_eq!(
            dir.entries(),
            ["musicat.toml"],
            "no temp or .bak left behind"
        );
    }

    #[test]
    fn save_errors_when_the_parent_cannot_be_created() {
        let dir = TempDir::new();
        let blocker = dir.0.join("blocker");
        write(&blocker, "i am a file");
        let (mut store, _) = ConfigStore::open(blocker.join("sub").join("musicat.toml"));
        assert!(store.update(|c| c.macos.perl_host = false).is_err());
        assert!(!store.config().macos.perl_host);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_parent_directory_makes_save_fail_without_panicking_or_leaving_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let ro = dir.0.join("ro");
        fs::create_dir(&ro).unwrap();
        write(&ro.join("musicat.toml"), "lyric_strip_visible = false\n");
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        // root 不受權限限制：此時略過斷言。
        let enforced = File::create(ro.join("probe")).is_err();

        let (mut store, _) = ConfigStore::open(ro.join("musicat.toml"));
        let result = store.update(|c| c.macos.perl_host = false);
        let after = fs::read_to_string(ro.join("musicat.toml")).unwrap();
        let leftovers = fs::read_dir(&ro).unwrap().count();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();

        if enforced {
            assert!(result.is_err());
            assert_eq!(after, "lyric_strip_visible = false\n", "original untouched");
            assert_eq!(leftovers, 1, "no temp file left");
            assert!(!store.config().macos.perl_host, "in-memory change is kept");
        }
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let dir = TempDir::new();
        // 巢狀路徑：缺少的資料夾會被建立。
        let path = dir.0.join("a").join("b").join("musicat.toml");
        let cfg = non_default();
        save(&path, &cfg, LoadSource::Missing).unwrap();
        let r = load(&path);
        assert_eq!(r.config, cfg);
        assert!(r.warnings.is_empty());
        assert_eq!(r.source, LoadSource::Loaded);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# MusiCat 設定檔 — 由程式寫入；格式見 README\n"));

        // 覆蓋既有檔案：目錄裡只有最終檔，沒有殘留的暫存檔。
        let mut cfg2 = cfg.clone();
        cfg2.screen = None;
        cfg2.lyric_strip_visible = true;
        save(&path, &cfg2, LoadSource::Loaded).unwrap();
        assert_eq!(load(&path).config, cfg2);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["musicat.toml"]);
    }

    #[test]
    fn tricky_screen_ids_round_trip() {
        let dir = TempDir::new();
        for id in [
            "with \"quotes\" and \\ backslash",
            "換行\nand\ttab",
            "中文螢幕 ✓",
            "'single'",
        ] {
            let cfg = Config {
                screen: Some(id.into()),
                ..Config::default()
            };
            save(&dir.file(), &cfg, LoadSource::Loaded).unwrap();
            let r = load(&dir.file());
            assert!(r.warnings.is_empty(), "{id:?}: {:?}", r.warnings);
            assert_eq!(r.config.screen.as_deref(), Some(id));
        }
    }

    #[test]
    fn lyric_strip_visible_defaults_to_true() {
        // Swift: LyricStripPreferenceTests.testDefaultsToVisible
        let dir = TempDir::new();
        let (store, _) = ConfigStore::open(dir.file());
        assert!(store.config().lyric_strip_visible);
        assert!(Config::default().lyric_strip_visible);
    }

    #[test]
    fn lyric_strip_visible_persists_across_load_save_cycles() {
        // Swift: LyricStripPreferenceTests.testPersistsAcrossInstances
        let dir = TempDir::new();
        let (mut first, _) = ConfigStore::open(dir.file());
        first.update(|c| c.lyric_strip_visible = false).unwrap();

        let (second, _) = ConfigStore::open(dir.file());
        assert!(!second.config().lyric_strip_visible);

        first.update(|c| c.lyric_strip_visible = true).unwrap();
        let (third, _) = ConfigStore::open(dir.file());
        assert!(third.config().lyric_strip_visible);
    }

    #[test]
    fn perl_host_false_round_trips() {
        let dir = TempDir::new();
        let (mut store, _) = ConfigStore::open(dir.file());
        assert!(store.config().macos.perl_host);
        store.update(|c| c.macos.perl_host = false).unwrap();
        let (again, warnings) = ConfigStore::open(dir.file());
        assert!(warnings.is_empty());
        assert!(!again.config().macos.perl_host);
        assert!(again.config().sources.lrclib, "other fields keep defaults");
    }

    #[test]
    fn a_failed_save_keeps_the_in_memory_change_and_a_later_save_can_succeed() {
        let dir = TempDir::new();
        let blocker = dir.0.join("blocker");
        let (mut store, _) = ConfigStore::open(blocker.join("musicat.toml"));
        write(&blocker, "file in the way");
        assert!(store.update(|c| c.sources.lrclib = false).is_err());
        assert!(!store.config().sources.lrclib);
        fs::remove_file(&blocker).unwrap();
        store.update(|c| c.sources.netease = false).unwrap();
        let r = load(store.path());
        assert!(!r.config.sources.lrclib && !r.config.sources.netease);
    }
}
