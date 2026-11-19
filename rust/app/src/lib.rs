//! musicat-app：把 core 的注入介面接到真實世界（HTTP、時鐘）。
//!
//! 這裡沒有任何 OS 專屬程式碼，Windows 也能編譯。

pub mod config;
pub mod http_ureq;
pub mod menu;
pub mod real_runtime;
pub mod switchable;
