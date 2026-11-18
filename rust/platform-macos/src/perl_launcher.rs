//! 以 Apple 簽章的 `/usr/bin/perl` 載入 helper 動態庫並讀取它的輸出（移植自 Swift
//! `PerlHelperLauncher.swift`）。
//!
//! 系統只把現正播放的資料交給 Apple 簽章的行程，所以讀取發生在 perl 裡：perl 以 DynaLoader 載入
//! ad-hoc 簽章的動態庫，把入口符號當 XSUB 呼叫。helper 把 JSON 行寫到 stdout，並在 stdin 到達 EOF 時結束；
//! stdin 是一條由回傳的 handle 持有的 pipe，handle 活著它就保持開啟。
//!
//! 只用 `std::process`，沒有 macOS 專屬 API，所以在所有目標上都編譯（Windows CI 的 `cargo check` 也包含它）；
//! 只有「執行檔權限位元」與「被訊號殺死的結束碼」兩處以 `cfg(unix)` 區分。
//!
//! # 行程所有權與執行緒
//!
//! - `Child` 放在 `Arc<Mutex<Child>>`，由 handle 與**一條**讀取執行緒共用。鎖只在 `try_wait`／`kill`
//!   這種立即返回的呼叫期間持有，絕不在阻塞讀取、`wait()` 或呼叫 sink 時持有，
//!   所以 `terminate()` 在任何時刻都能從別的執行緒殺掉行程。
//! - 讀取執行緒阻塞在 stdout 上（行程執行期間零輪詢）；讀到 EOF（行程結束或被殺）後，每 20 ms `try_wait`
//!   直到回收行程，再送出恰好一次 [`HelperEvent::Exited`]。因此 `Exited` 一定排在所有 `Line` 之後。
//!   不用 `libc::kill`（不新增依賴，也避免 pid 被重用時誤殺）；不用「執行緒持有 Child 並 `wait()`」
//!   （那會讓別的執行緒無法透過 `Child::kill` 殺行程）。
//! - 執行緒不 join：sink 可能在讀取執行緒上放掉最後一個 supervisor 參考，進而在該執行緒上 drop handle，
//!   所以 handle 的 `Drop` 只做 `terminate()`（關 stdin、kill、在上限 2 秒內回收），不碰 `JoinHandle`。
//!   `terminate()` 之後行程死亡使 stdout EOF，執行緒自行結束。
//! - 結束碼：正常結束回報行程的 exit code；被訊號殺死（`terminate()` 的 SIGKILL）回報 `128 + 訊號編號`
//!   （非 unix 為 -1）；等待失敗為 -1。`terminate()` 後仍會送出 `Exited`（Swift 的 terminationHandler 也一樣），
//!   由 supervisor 以 generation 忽略。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::helper_wire::LineBuffer;
use crate::supervisor::{HelperEvent, HelperHandle, HelperLauncher, HelperSink, LaunchError};
use crate::DYLIB_NAME;

/// 固定的 perl 主機腳本。`$ARGV[0]` 是動態庫的**絕對**路徑（perl 的 hardened runtime 拒絕相對路徑）。
/// 與 Swift 版逐行相同，僅符號改為 `musicat_helper_main`（即 `crate::HELPER_SYMBOL`；字面值在這裡寫死、由測試比對）。
pub const SCRIPT: &str = r#"use DynaLoader;
my $lib = DynaLoader::dl_load_file($ARGV[0], 0) or die "load: " . DynaLoader::dl_error() . "\n";
my $sym = DynaLoader::dl_find_symbol($lib, "musicat_helper_main") or die "symbol: " . DynaLoader::dl_error() . "\n";
my $xs = DynaLoader::dl_install_xsub("main::musicat_helper_main", $sym);
&$xs();"#;

/// 要從環境中移除的變數前綴：這些會讓 perl 或動態載入器從外面載入程式碼。
const BLOCKED_PREFIXES: [&str; 3] = ["PERL", "DYLD_", "LD_"];

/// 結束輪詢間隔（只在 stdout 已 EOF、等行程被回收的短暫階段使用）。
const REAP_POLL: Duration = Duration::from_millis(20);
/// `terminate()` 在 kill 之後最多等多久回收行程。
const TERMINATE_REAP_LIMIT: Duration = Duration::from_secs(2);
/// stdout 的單次讀取大小。
const READ_CHUNK: usize = 64 * 1024;

/// 靜態旗標：缺動態庫的訊息整個行程只印一次（不是每次重試印一次）。
static DID_LOG_MISSING_DYLIB: AtomicBool = AtomicBool::new(false);

/// `env` 去掉前綴為 `PERL`、`DYLD_`、`LD_` 的變數，確保 app 繼承來的東西不會被載進 Apple 簽章的主機行程。
pub fn sanitized_env(env: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    env.filter(|(key, _)| !BLOCKED_PREFIXES.iter().any(|p| key.starts_with(p)))
        .collect()
}

/// 可執行檔旁的動態庫；不存在時為 `None`。
fn default_dylib() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(DYLIB_NAME);
    path.exists().then_some(path)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 缺動態庫的訊息只在 `flag` 第一次被翻起時呼叫 `log`。`flag` 與 `log` 由呼叫端注入以便測試；
/// 實際使用時是 [`DID_LOG_MISSING_DYLIB`] 與 `eprintln!`。路徑是 app 自己的動態庫，不是使用者資料。
pub(crate) fn log_missing_dylib_once(
    flag: &AtomicBool,
    dylib: Option<&Path>,
    log: impl FnOnce(&str),
) {
    if flag.swap(true, Ordering::SeqCst) {
        return;
    }
    let where_ = dylib.map_or_else(
        || "nothing next to the executable".to_string(),
        |p| p.display().to_string(),
    );
    log(&format!(
        "MusiCat: {DYLIB_NAME} not found ({where_}); system now-playing is unavailable"
    ));
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 啟動 `perl -e SCRIPT <dylib>` 的 [`HelperLauncher`]。
#[derive(Clone, Debug)]
pub struct PerlLauncher {
    perl_path: PathBuf,
    dylib: Option<PathBuf>,
}

impl PerlLauncher {
    pub fn new(perl_path: PathBuf, dylib: Option<PathBuf>) -> Self {
        Self { perl_path, dylib }
    }

    /// 與 [`HelperLauncher::launch`] 相同，但回傳具體型別（可查 pid 與是否已結束；供測試與診斷）。
    pub fn spawn(&self, sink: HelperSink) -> Result<PerlHelperHandle, LaunchError> {
        if !is_executable_file(&self.perl_path) {
            return Err(LaunchError::PerlNotFound);
        }
        let dylib = match &self.dylib {
            Some(path) if path.exists() => path,
            other => {
                log_missing_dylib_once(&DID_LOG_MISSING_DYLIB, other.as_deref(), |m| {
                    eprintln!("{m}")
                });
                return Err(LaunchError::DylibNotFound);
            }
        };
        let absolute = std::fs::canonicalize(dylib).unwrap_or_else(|_| dylib.clone());

        let env = sanitized_env(
            std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))),
        );
        let mut child = Command::new(&self.perl_path)
            .arg("-e")
            .arg(SCRIPT)
            .arg(&absolute)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // helper 的診斷輸出留在父行程的 stderr（與 Swift 版相同）。
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| LaunchError::Spawn(e.to_string()))?;

        let id = child.id();
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LaunchError::Spawn("child pipes unavailable".into()));
        };
        let child = Arc::new(Mutex::new(child));
        let reader_child = Arc::clone(&child);
        let spawned = std::thread::Builder::new()
            .name("musicat-helper-reader".into())
            .spawn(move || read_until_exit(stdout, &reader_child, &sink));
        if let Err(e) = spawned {
            let mut c = lock(&child);
            let _ = c.kill();
            let _ = c.wait();
            return Err(LaunchError::Spawn(format!("reader thread: {e}")));
        }
        Ok(PerlHelperHandle {
            id,
            child,
            stdin: Some(stdin),
        })
    }
}

impl Default for PerlLauncher {
    /// `/usr/bin/perl` 與可執行檔旁的動態庫。
    fn default() -> Self {
        Self::new(PathBuf::from("/usr/bin/perl"), default_dylib())
    }
}

impl HelperLauncher for PerlLauncher {
    fn launch(&self, sink: HelperSink) -> Result<Box<dyn HelperHandle>, LaunchError> {
        self.spawn(sink)
            .map(|h| Box::new(h) as Box<dyn HelperHandle>)
    }
}

/// 讀取執行緒本體：stdout → `LineBuffer` → sink；EOF 後回收行程並送出 `Exited`。
fn read_until_exit(mut stdout: impl Read, child: &Mutex<Child>, sink: &HelperSink) {
    let mut lines = LineBuffer::new();
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for line in lines.push(&buf[..n]) {
                    sink(HelperEvent::Line(line));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    // 鎖只在每次 try_wait 期間持有；睡眠時放開，terminate() 隨時能插進來。
    let code = loop {
        match lock(child).try_wait() {
            Ok(Some(status)) => break exit_code(status),
            Ok(None) => {}
            Err(_) => break -1,
        }
        std::thread::sleep(REAP_POLL);
    };
    sink(HelperEvent::Exited(code));
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

/// 一個執行中的 perl helper。drop 時終止並回收行程。
pub struct PerlHelperHandle {
    id: u32,
    child: Arc<Mutex<Child>>,
    /// 持有它 helper 的 stdin 才保持開啟；放掉（關閉）後 helper 在 EOF 結束。
    stdin: Option<ChildStdin>,
}

impl PerlHelperHandle {
    /// 行程 id（診斷與測試用）。
    pub fn id(&self) -> u32 {
        self.id
    }

    /// 行程是否已結束並被回收。
    pub fn has_exited(&self) -> bool {
        !matches!(lock(&self.child).try_wait(), Ok(None))
    }
}

impl HelperHandle for PerlHelperHandle {
    /// 關 stdin、kill 行程（若還在跑），並在上限 2 秒內等它被回收。可重複呼叫，行程已結束後呼叫也無害。
    fn terminate(&mut self) {
        drop(self.stdin.take());
        {
            let mut child = lock(&self.child);
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
        }
        let deadline = Instant::now() + TERMINATE_REAP_LIMIT;
        while Instant::now() < deadline {
            if !matches!(lock(&self.child).try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for PerlHelperHandle {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HELPER_SYMBOL;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Tests that exec a freshly written script or real perl are serialised: a fork in
    /// another test thread while the script file is still open for writing makes exec fail
    /// with ETXTBSY.
    static SPAWN_LOCK: Mutex<()> = Mutex::new(());

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "musicat-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        p
    }

    /// An existing executable file stands in for "perl" in tests that fail before spawning.
    fn existing_executable() -> PathBuf {
        std::env::current_exe().expect("current_exe")
    }

    fn collecting_sink() -> (HelperSink, mpsc::Receiver<HelperEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: HelperSink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        (sink, rx)
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    fn wait_for(
        rx: &mpsc::Receiver<HelperEvent>,
        secs: u64,
        pred: impl Fn(&HelperEvent) -> bool,
    ) -> Option<HelperEvent> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(e) if pred(&e) => return Some(e),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    // ---- ported from MediaRemoteSourceTests.swift (PerlHelperLauncher) ----

    #[test]
    fn perl_launcher_throws_when_perl_missing() {
        let dylib = temp_path("dylib");
        std::fs::write(&dylib, b"").unwrap();
        let launcher = PerlLauncher::new(
            PathBuf::from("/nonexistent/musicat/perl"),
            Some(dylib.clone()),
        );
        let (sink, _rx) = collecting_sink();
        let result = launcher.launch(sink);
        let _ = std::fs::remove_file(&dylib);
        assert_eq!(result.err(), Some(LaunchError::PerlNotFound));
    }

    #[test]
    fn perl_launcher_throws_when_dylib_missing() {
        let missing = PerlLauncher::new(
            existing_executable(),
            Some(PathBuf::from(
                "/nonexistent/musicat/libmusicat_mediaremote_helper.dylib",
            )),
        );
        let (sink, _rx) = collecting_sink();
        assert_eq!(missing.launch(sink).err(), Some(LaunchError::DylibNotFound));
        let unset = PerlLauncher::new(existing_executable(), None);
        let (sink, _rx) = collecting_sink();
        assert_eq!(unset.launch(sink).err(), Some(LaunchError::DylibNotFound));
    }

    #[test]
    fn sanitized_environment_drops_loader_and_perl_variables() {
        let result = sanitized_env(
            env(&[
                ("PERL5OPT", "-Mfoo"),
                ("PERL5LIB", "/tmp/lib"),
                ("PERLLIB", "/tmp/lib"),
                ("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib"),
                ("LD_PRELOAD", "/tmp/evil.so"),
                ("PATH", "/usr/bin:/bin"),
                ("HOME", "/Users/someone"),
            ])
            .into_iter(),
        );
        assert_eq!(
            result,
            env(&[("PATH", "/usr/bin:/bin"), ("HOME", "/Users/someone")])
        );
    }

    // ---- plan extras ----

    #[test]
    fn script_uses_the_helper_symbol() {
        assert!(SCRIPT.contains(&format!("\"{HELPER_SYMBOL}\"")));
        assert!(SCRIPT.contains(&format!("\"main::{HELPER_SYMBOL}\"")));
        assert!(!SCRIPT.contains("notchcat"));
        assert!(SCRIPT.contains("$ARGV[0]"));
    }

    #[test]
    fn script_is_valid_perl_with_single_backslash_newlines() {
        assert!(!SCRIPT.contains("\\\\n"), "doubled backslash in raw string");
        if !Path::new("/usr/bin/perl").exists() {
            return;
        }
        let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let out = Command::new("/usr/bin/perl")
            .arg("-c")
            .arg("-e")
            .arg(SCRIPT)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn missing_dylib_message_is_logged_only_once() {
        let flag = AtomicBool::new(false);
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..3 {
            log_missing_dylib_once(&flag, Some(Path::new("/x/lib.dylib")), |m| {
                lines.push(m.to_string())
            });
        }
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("/x/lib.dylib"), "{}", lines[0]);
        assert!(lines[0].contains(DYLIB_NAME), "{}", lines[0]);
        assert!(flag.load(Ordering::SeqCst));
        let flag = AtomicBool::new(false);
        let mut lines: Vec<String> = Vec::new();
        log_missing_dylib_once(&flag, None, |m| lines.push(m.to_string()));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("nothing next to the executable"));
    }

    #[test]
    fn default_dylib_is_none_when_not_next_to_the_executable() {
        // The unit-test executable lives in target/<profile>/deps/, where the dylib is not.
        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        if !exe_dir.join(DYLIB_NAME).exists() {
            assert_eq!(default_dylib(), None);
        }
    }

    #[cfg(unix)]
    fn write_script(tag: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path(tag);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn lines_flow_to_the_sink_and_terminate_kills_and_reports_exit_once() {
        let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dylib = temp_path("dylib");
        std::fs::write(&dylib, b"").unwrap();
        let script = write_script("fakeperl", "echo hello\necho world\nexec cat");
        let launcher = PerlLauncher::new(script.clone(), Some(dylib.clone()));
        let (sink, rx) = collecting_sink();
        let mut handle = launcher.launch(sink).expect("launch");
        drop(_guard);

        assert_eq!(
            wait_for(
                &rx,
                5,
                |e| matches!(e, HelperEvent::Line(l) if l == "hello")
            ),
            Some(HelperEvent::Line("hello".into()))
        );
        assert_eq!(
            wait_for(
                &rx,
                5,
                |e| matches!(e, HelperEvent::Line(l) if l == "world")
            ),
            Some(HelperEvent::Line("world".into()))
        );
        handle.terminate();
        handle.terminate(); // second call is harmless
        let exited = wait_for(&rx, 5, |e| matches!(e, HelperEvent::Exited(_))).expect("Exited");
        match exited {
            HelperEvent::Exited(code) => assert_ne!(code, 0, "killed by signal must be non-zero"),
            _ => unreachable!(),
        }
        handle.terminate(); // after exit
        drop(handle);
        // Exactly once.
        assert_eq!(
            wait_for(&rx, 1, |e| matches!(e, HelperEvent::Exited(_))),
            None
        );
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&dylib);
    }

    #[cfg(unix)]
    #[test]
    fn natural_exit_reports_its_code_after_all_lines() {
        let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dylib = temp_path("dylib");
        std::fs::write(&dylib, b"").unwrap();
        let script = write_script("fakeperl-exit", "echo one\nexit 7");
        let launcher = PerlLauncher::new(script.clone(), Some(dylib.clone()));
        let (sink, rx) = collecting_sink();
        let mut handle = launcher.launch(sink).expect("launch");
        drop(_guard);

        let mut events = Vec::new();
        while let Some(e) = wait_for(&rx, 5, |_| true) {
            let done = matches!(e, HelperEvent::Exited(_));
            events.push(e);
            if done {
                break;
            }
        }
        assert_eq!(
            events,
            vec![HelperEvent::Line("one".into()), HelperEvent::Exited(7)]
        );
        handle.terminate(); // after the process exited on its own
        drop(handle); // dropped without terminate() after Exited is also fine
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&dylib);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_handle_kills_the_helper() {
        let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dylib = temp_path("dylib");
        std::fs::write(&dylib, b"").unwrap();
        let script = write_script("fakeperl-drop", "echo up\nexec cat");
        let launcher = PerlLauncher::new(script.clone(), Some(dylib.clone()));
        let (sink, rx) = collecting_sink();
        let handle = launcher.launch(sink).expect("launch");
        drop(_guard);
        assert!(wait_for(&rx, 5, |e| matches!(e, HelperEvent::Line(_))).is_some());
        drop(handle);
        assert!(wait_for(&rx, 5, |e| matches!(e, HelperEvent::Exited(_))).is_some());
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&dylib);
    }

    #[cfg(unix)]
    #[test]
    fn environment_is_cleared_and_sanitized() {
        let _guard = SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dylib = temp_path("dylib");
        std::fs::write(&dylib, b"").unwrap();
        // Prints whether the variables are visible, then the arguments count.
        let script = write_script(
            "fakeperl-env",
            "echo \"perl=${PERL5LIB:-unset} dyld=${DYLD_INSERT_LIBRARIES:-unset} nargs=$#\"\nexit 0",
        );
        std::env::set_var("PERL5LIB", "/tmp/evil");
        let launcher = PerlLauncher::new(script.clone(), Some(dylib.clone()));
        let (sink, rx) = collecting_sink();
        let handle = launcher.launch(sink).expect("launch");
        drop(_guard);
        let line = wait_for(&rx, 5, |e| matches!(e, HelperEvent::Line(_))).expect("line");
        std::env::remove_var("PERL5LIB");
        assert_eq!(
            line,
            HelperEvent::Line("perl=unset dyld=unset nargs=3".into())
        );
        drop(handle);
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&dylib);
    }

    // ---- real helper (manual) ----

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs MUSICAT_REAL_HELPER=1; talks to the local MediaRemote service"]
    fn real_helper_emits_at_least_one_line_within_five_seconds() {
        if std::env::var("MUSICAT_REAL_HELPER").as_deref() != Ok("1") {
            eprintln!("SKIP: set MUSICAT_REAL_HELPER=1 to run");
            return;
        }
        // Test executables live in target/<profile>/deps/; build.rs put the dylib one level up.
        let exe = std::env::current_exe().expect("current_exe");
        let dylib = exe.parent().unwrap().parent().unwrap().join(DYLIB_NAME);
        assert!(dylib.exists(), "{dylib:?} missing (build.rs output)");

        let launcher = PerlLauncher::new(PathBuf::from("/usr/bin/perl"), Some(dylib));
        let (sink, rx) = collecting_sink();
        let mut handle = launcher.spawn(sink).expect("spawn");
        let pid = handle.id();
        let first = wait_for(&rx, 5, |e| matches!(e, HelperEvent::Line(_)));
        eprintln!("first event: {first:?}");
        assert!(first.is_some(), "no line within 5 s");

        handle.terminate();
        let exited = wait_for(&rx, 5, |e| matches!(e, HelperEvent::Exited(_)));
        eprintln!("exit event: {exited:?}");
        assert!(exited.is_some(), "no Exited event after terminate");
        assert!(handle.has_exited(), "perl pid {pid} still running");
        let ps = std::process::Command::new("ps")
            .args(["-p", &pid.to_string()])
            .output()
            .unwrap();
        eprintln!(
            "ps -p {pid} status: {:?} (non-zero = process gone)",
            ps.status.code()
        );
        assert!(!ps.status.success(), "perl pid {pid} is still listed by ps");
    }
}
