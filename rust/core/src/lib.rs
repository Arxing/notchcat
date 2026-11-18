//! musicat-core：與平台無關的純邏輯（平台條件編譯一律禁止，見 scripts/check-core-purity.sh）。
//!
//! 非同步模型見 `docs/adr-0001-async-model.md`：一般 `Future` 加注入的 `Clock`／`Spawner`／`HttpClient`。

pub mod http;
pub mod runtime;
pub mod timing;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
