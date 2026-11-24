//! 全 Rust 程式碼裡唯一允許碰私有 MediaRemote 框架的檔案（`helper/` 內的 C 讀取端除外）。
//! 這裡只送傳輸控制指令：不要在此加入任何讀取呼叫。
//! 讀取 now-playing 狀態一律在 helper 行程內進行，絕不在本行程內讀。
//!
//! 若 Apple 日後撤下該符號，`load()` 回 `None`，只需改這個檔案：
//! 模組會在沒有傳輸控制鈕的情況下繼續執行，而不是把整個 shell 拖垮。
//!
//! 載入器只在 macOS 上存在（`libloading` 只在 macOS 目標才相依）；
//! `command_code` 與常數在所有平台都能編譯與測試。

use musicat_core::nowplaying::PlaybackCommand;

/// 預設的框架路徑。
pub const DEFAULT_FRAMEWORK_PATH: &str =
    "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote";

/// 送控制指令的 C 符號。
pub const SEND_COMMAND_SYMBOL: &str = "MRMediaRemoteSendCommand";

/// MediaRemote 的原始指令碼。
pub fn command_code(cmd: PlaybackCommand) -> u32 {
    match cmd {
        PlaybackCommand::Play => 0,
        PlaybackCommand::Pause => 1,
        PlaybackCommand::TogglePlayPause => 2,
        PlaybackCommand::Next => 4,
        PlaybackCommand::Previous => 5,
    }
}

#[cfg(target_os = "macos")]
pub use loader::MediaRemoteControl;

#[cfg(target_os = "macos")]
mod loader {
    use super::{command_code, DEFAULT_FRAMEWORK_PATH, SEND_COMMAND_SYMBOL};
    use libloading::os::unix::{Library, RTLD_NOW};
    use musicat_core::nowplaying::{PlaybackCommand, PlaybackControl};
    use std::ffi::c_void;

    /// `bool MRMediaRemoteSendCommand(uint32_t command, CFDictionaryRef options)`。
    /// 選項字典永遠不用，所以以不透明指標傳 null。
    type SendCommandFn = unsafe extern "C" fn(u32, *const c_void) -> bool;

    /// 向系統的 now-playing 服務送傳輸指令。
    pub struct MediaRemoteControl {
        // 只為了讓框架在整個物件生命週期內保持載入；`send_command` 取的是下面這個函式指標，
        // 所以不能比它早卸載（這裡永遠不 dlclose，直到物件被丟棄）。
        _library: Library,
        send_command: SendCommandFn,
    }

    impl MediaRemoteControl {
        /// 以預設路徑與符號載入；任一步失敗回 `None`。
        pub fn load() -> Option<MediaRemoteControl> {
            Self::load_from(DEFAULT_FRAMEWORK_PATH, SEND_COMMAND_SYMBOL)
        }

        /// 以指定的框架路徑與符號名載入（測試用）；任一步失敗回 `None`，不 panic。
        pub fn load_from(framework_path: &str, symbol_name: &str) -> Option<MediaRemoteControl> {
            // SAFETY: 載入系統框架會執行其初始化常式；這裡載入的是 Apple 自己的框架（或測試
            // 指定的路徑），與 Swift 版 dlopen 相同的信任界線。
            let library = unsafe { Library::open(Some(framework_path), RTLD_NOW) }.ok()?;
            // SAFETY: 簽章與 C 宣告一致（uint32_t, 指標 -> bool）。複製出的函式指標不帶生命週期，
            // 但 `library` 與它同存於結構內，所以在結構存活期間它一定有效。
            let send_command: SendCommandFn =
                unsafe { *library.get::<SendCommandFn>(symbol_name.as_bytes()).ok()? };
            Some(MediaRemoteControl {
                _library: library,
                send_command,
            })
        }

        /// 送出指令並回傳系統的答覆（`false` 表示系統拒絕），供診斷用。
        pub fn send_command(&self, cmd: PlaybackCommand) -> bool {
            // SAFETY: 函式指標由 `load_from` 解析自仍載入中的框架，簽章已核對；
            // 選項指標為 null，C 端允許。
            unsafe { (self.send_command)(command_code(cmd), std::ptr::null()) }
        }
    }

    // `Library` 與函式指標本身都是 Send + Sync，所以這個型別自動是 Send + Sync，不需 unsafe impl。
    impl PlaybackControl for MediaRemoteControl {
        fn send(&self, cmd: PlaybackCommand) {
            // 系統拒絕不是錯誤：沒有可採取的動作，也不記錄（避免洩漏使用者資料）。
            let _ = self.send_command(cmd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_raw_values_match_mediaremote() {
        assert_eq!(command_code(PlaybackCommand::Play), 0);
        assert_eq!(command_code(PlaybackCommand::Pause), 1);
        assert_eq!(command_code(PlaybackCommand::TogglePlayPause), 2);
        assert_eq!(command_code(PlaybackCommand::Next), 4);
        assert_eq!(command_code(PlaybackCommand::Previous), 5);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn init_fails_when_framework_path_is_wrong() {
        assert!(
            MediaRemoteControl::load_from("/nonexistent/MediaRemote", SEND_COMMAND_SYMBOL)
                .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn init_fails_when_symbol_is_absent() {
        assert!(MediaRemoteControl::load_from(
            DEFAULT_FRAMEWORK_PATH,
            "MRMediaRemoteSendCommand_DoesNotExist"
        )
        .is_none());
    }

    /// 回 None 代表私有框架搬家或控制符號被撤下；模組此時應不帶傳輸控制鈕地執行。
    /// 只載入框架並解析符號，不會送出任何指令（絕不呼叫 `send`）。
    #[cfg(target_os = "macos")]
    #[test]
    fn constructs_against_real_framework() {
        let control = MediaRemoteControl::load();
        assert!(control.is_some(), "MediaRemote 無法載入");
    }
}
