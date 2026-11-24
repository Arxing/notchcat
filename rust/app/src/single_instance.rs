//! 單一執行個體守衛：在設定檔旁取一把「建議性」檔案鎖（`musicat.lock`），行程存活期間一直持有。
//!
//! 作業系統在行程結束時（包括 `kill -9` 與崩潰）自動釋放鎖，所以不會有過期的鎖；這裡刻意不寫 pid。
//! 純標準函式庫（`File::try_lock`，Rust 1.89 起穩定），沒有任何 OS 專屬或 GPUI 程式碼，Windows 也能編譯。

use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// 鎖檔的檔名。
pub const LOCK_FILE_NAME: &str = "musicat.lock";

/// 持有鎖的守衛：存活期間（直到被丟棄或行程結束）鎖都在。
pub struct InstanceLock {
    _file: File,
}

impl fmt::Debug for InstanceLock {
    // 不印路徑（會含家目錄）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InstanceLock")
    }
}

#[derive(Debug)]
pub enum Acquire {
    Acquired(InstanceLock),
    /// 已有另一個執行個體持有這把鎖。
    AlreadyRunning,
    /// 守衛本身出了問題（資料夾建不起來、唯讀、檔案系統不支援鎖…）：呼叫端應警告後照常啟動。
    /// 原因字串不含路徑。
    Skipped(String),
}

/// 設定檔所在資料夾裡的 `musicat.lock`；設定檔路徑沒有上層（如 `musicat.toml`）時放在目前目錄。
pub fn lock_path_for(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(LOCK_FILE_NAME)
}

/// 嘗試取得鎖（不等待）。必要時先建立上層資料夾；檔案不存在就建立（內容無意義，不截斷）。
/// 回傳的 [`InstanceLock`] 必須持有到行程結束。
pub fn acquire(lock_path: &Path) -> Acquire {
    if let Some(parent) = lock_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                return Acquire::Skipped(format!(
                    "cannot create the lock directory ({})",
                    e.kind()
                ));
            }
        }
    }
    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(e) => return Acquire::Skipped(format!("cannot open the lock file ({})", e.kind())),
    };
    match file.try_lock() {
        Ok(()) => Acquire::Acquired(InstanceLock { _file: file }),
        Err(TryLockError::WouldBlock) => Acquire::AlreadyRunning,
        Err(TryLockError::Error(e)) => {
            Acquire::Skipped(format!("cannot lock the file ({})", e.kind()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 唯一的暫存資料夾；丟棄時清掉。
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "musicat-single-instance-test-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_acquire_succeeds_and_second_reports_already_running() {
        let dir = TempDir::new();
        let path = dir.path().join(LOCK_FILE_NAME);
        let first = acquire(&path);
        assert!(matches!(first, Acquire::Acquired(_)), "{first:?}");
        let second = acquire(&path);
        assert!(matches!(second, Acquire::AlreadyRunning), "{second:?}");
        // 第一把仍然有效（沒被第二次嘗試弄壞）。
        let third = acquire(&path);
        assert!(matches!(third, Acquire::AlreadyRunning), "{third:?}");
        drop(first);
    }

    #[test]
    fn lock_is_released_when_the_guard_is_dropped() {
        let dir = TempDir::new();
        let path = dir.path().join(LOCK_FILE_NAME);
        let Acquire::Acquired(guard) = acquire(&path) else {
            panic!("first acquire failed");
        };
        drop(guard);
        let again = acquire(&path);
        assert!(matches!(again, Acquire::Acquired(_)), "{again:?}");
    }

    #[test]
    fn different_lock_paths_do_not_conflict() {
        let a = TempDir::new();
        let b = TempDir::new();
        let first = acquire(&a.path().join(LOCK_FILE_NAME));
        let second = acquire(&b.path().join(LOCK_FILE_NAME));
        assert!(matches!(first, Acquire::Acquired(_)), "{first:?}");
        assert!(matches!(second, Acquire::Acquired(_)), "{second:?}");
    }

    #[test]
    fn lock_path_is_next_to_the_config_file() {
        assert_eq!(
            lock_path_for(Path::new("/some/dir/musicat.toml")),
            Path::new("/some/dir").join("musicat.lock")
        );
        // 沒有上層資料夾：放在目前目錄，不 panic。
        let relative = lock_path_for(Path::new("musicat.toml"));
        assert_eq!(relative.file_name().unwrap(), "musicat.lock");
        assert!(
            relative == Path::new("musicat.lock") || relative == Path::new("./musicat.lock"),
            "{relative:?}"
        );
        // 怪路徑也不 panic，檔名固定。
        for odd in ["", "/", ".", ".."] {
            let p = lock_path_for(Path::new(odd));
            assert_eq!(p.file_name().unwrap(), "musicat.lock", "{odd:?} -> {p:?}");
        }
    }

    #[test]
    fn missing_parent_directory_is_created() {
        let dir = TempDir::new();
        let path = dir.path().join("a").join("b").join(LOCK_FILE_NAME);
        let result = acquire(&path);
        assert!(matches!(result, Acquire::Acquired(_)), "{result:?}");
        assert!(path.is_file());
    }

    #[test]
    fn unwritable_directory_returns_skipped_not_a_panic() {
        let dir = TempDir::new();
        // 上層是一般檔案：建不了資料夾也開不了鎖檔。
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, b"x").unwrap();
        let result = acquire(&blocker.join(LOCK_FILE_NAME));
        let Acquire::Skipped(reason) = result else {
            panic!("expected Skipped, got {result:?}");
        };
        assert!(!reason.is_empty());
        assert!(
            !reason.contains(dir.path().to_str().unwrap()),
            "reason leaks the path: {reason}"
        );
    }

    #[test]
    fn the_debug_output_does_not_contain_the_path() {
        let dir = TempDir::new();
        let path = dir.path().join(LOCK_FILE_NAME);
        let result = acquire(&path);
        let shown = format!("{result:?}");
        assert!(shown.contains("Acquired"), "{shown}");
        assert!(!shown.contains(dir.path().to_str().unwrap()), "{shown}");
        assert!(!shown.contains(LOCK_FILE_NAME), "{shown}");
    }
}
