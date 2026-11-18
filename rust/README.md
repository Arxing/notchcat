# MusiCat（Rust 版）

MusiCat 是 NotchCat 的跨平台 Rust 重寫：把螢幕頂端的瀏海（或虛擬瀏海）變成可互動面板，目前的第一個模組是 Music——顯示現正播放的曲目、封面、進度、同步歌詞，並可控制播放。

設計文件見 [`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md`](../docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md)。原本的 Swift 版仍保留在 repo 根目錄，等 Rust 版通過實機驗收後才會移除。

## 狀態

- **macOS**：開發中。
- **Windows**：**未在實機驗證**。目前只有可編譯的骨架，CI 只要求編譯通過加 `musicat-core` 測試。

## 工作區結構

| crate | 路徑 | 說明 |
|---|---|---|
| `musicat-core` | `rust/core` | 平台無關的純邏輯（歌詞、播放估算、顯示規則、外殼幾何）。不得有平台條件編譯與非同步執行階段依賴。 |
| `musicat-platform-macos` | `rust/platform-macos` | macOS 專屬實作（MediaRemote、視窗、選單列圖示）。 |
| `musicat-platform-windows` | `rust/platform-windows` | Windows 骨架（未在實機驗證）。 |
| `musicat-shell` | `rust/shell` | GPUI 外殼與視圖。 |
| `musicat-app` | `rust/app` | 可執行檔 `musicat`，負責組裝、設定檔（TOML）與 HTTP 實作。 |

## 從原始碼建置

需要 Rust stable 工具鏈（`rust/rust-toolchain.toml` 會自動指定，含 `rustfmt`、`clippy`）。

```sh
cd rust
cargo build --workspace
cargo test --workspace
```

macOS 上建置 GPUI 所需的 Metal Toolchain 說明，將於 Task 21 補上。

## 隱私

查歌詞時，正在播放曲目的曲名與演出者（包含 YouTube 影片或頻道的任意標題）會送到 **lrclib.net** 與 **music.163.com**；除此之外不會傳送任何資料。請求的 User-Agent 固定為 `MusiCat`，不帶任何使用者資料。兩個歌詞來源可以個別關閉。

## macOS 風險

macOS 的 MediaRemote 讀取對第三方簽章的 App 是封閉的，因此 MusiCat 借用 Apple 簽章的 `/usr/bin/perl` 當宿主，由它載入一個很小的 C 動態庫來讀取現正播放的資訊。

- 這是借 Apple 簽章宿主的空隙，**不是正式開放的介面**，任何一次系統更新都可能讓它失效。
- 因為使用私有 API，**不適合上架 Mac App Store**，也不建議當作對外發布的產品。
- 可以在選單列選單與 TOML 設定檔中關閉這個讀取方式。

## 授權

雙重授權，擇一適用：

- [MIT](../LICENSE-MIT)
- [Apache License 2.0](../LICENSE-APACHE)

**歌詞資料不在上述授權範圍內。** 歌詞來自第三方服務，內容的版權屬於原權利人。

貢獻方式見 [`CONTRIBUTING.md`](../CONTRIBUTING.md)。
