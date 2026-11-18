//! 現正播放：來源介面、控制介面、多來源監視器與視窗標題解析
//! （移植自 Swift `Modules/Music/NowPlaying/`）。
//!
//! 具體來源（MediaRemote helper、視窗標題輪詢）與控制指令的實作在平台 crate，core 只定義介面。

pub mod monitor;
pub mod window_title;

pub use monitor::NowPlayingMonitor;
pub use window_title::{parse_window_title, ParsedTrack, WindowInfo};

use crate::runtime::Subscription;
use crate::types::TrackSnapshot;

/// 快照訂閱的回呼：等同 `Box<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>`（別名只為了壓低型別複雜度）。
pub type SnapshotCallback = Box<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>;

pub(crate) type SharedSnapshotCallback =
    std::sync::Arc<dyn Fn(&Option<TrackSnapshot>) + Send + Sync>;

/// 能回報目前播放曲目的來源。沒有東西在播時發布 `None`。
pub trait NowPlayingSource: Send + Sync {
    fn id(&self) -> &str;

    /// 開始監看。重複呼叫應無害。
    fn start(&self);

    /// 停止並發布 `None`；之後再 `start` 不會重播上次的快照。
    fn stop(&self);

    /// 訂閱快照。訂閱當下立即以目前值呼叫 `f` 一次（尚未有資料時為 `None`），之後每次變動再呼叫。
    /// 回呼可能在任何執行緒上被呼叫；drop 回傳的 [`Subscription`] 即取消訂閱。
    fn subscribe(&self, f: SnapshotCallback) -> Subscription;
}

/// 送給播放器的控制指令。對應的原始指令碼由平台 crate 決定。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PlaybackCommand {
    Play,
    Pause,
    TogglePlayPause,
    Next,
    Previous,
}

/// 播放控制。不可用（例如私有框架載入失敗）時，整個物件以 `Option<Arc<dyn PlaybackControl>>` 的 `None` 表示。
pub trait PlaybackControl: Send + Sync {
    fn send(&self, cmd: PlaybackCommand);
}
