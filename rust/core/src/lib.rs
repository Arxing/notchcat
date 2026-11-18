//! musicat-core：與平台無關的純邏輯（平台條件編譯一律禁止，見 scripts/check-core-purity.sh）。
//!
//! 非同步模型見 `docs/adr-0001-async-model.md`：一般 `Future` 加注入的 `Clock`／`Spawner`／`HttpClient`。

pub mod artwork_cache;
pub mod estimator;
pub mod http;
pub mod lyrics;
pub mod modules;
pub mod music;
pub mod nowplaying;
pub mod runtime;
pub mod shell_logic;
pub mod timing;
pub mod types;

pub use artwork_cache::ArtworkCache;
pub use estimator::PlaybackEstimator;
pub use types::{ArtworkRef, PositionAnchor, TrackSnapshot};

#[cfg(any(test, feature = "testing"))]
pub mod testing;
