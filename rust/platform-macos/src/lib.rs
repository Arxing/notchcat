//! musicat-platform-macos：macOS 專屬實作。
//!
//! 在非 macOS 平台上這是一個空 crate（只有兩個與平台無關的字串常數）。
//! `build.rs` 只在目標為 macOS 時才編譯 `helper/helper.c`。

pub mod helper_wire;
pub mod mediaremote_control;
pub mod perl_launcher;
pub mod supervisor;

/// 建置後放在 `target/<profile>/`（與可執行檔同目錄）的動態庫檔名；
/// perl 以絕對路徑經 DynaLoader 載入。
pub const DYLIB_NAME: &str = "libmusicat_mediaremote_helper.dylib";

/// 動態庫的入口符號（以 XSUB 簽章被 perl 呼叫）。
pub const HELPER_SYMBOL: &str = "musicat_helper_main";
