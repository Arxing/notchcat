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

第一次建置會編譯 GPUI（`=0.2.2`）與它的整個相依樹，需要好幾分鐘；之後是增量建置。
crates.io 下載逾時時可加 `CARGO_HTTP_TIMEOUT=120` 重試。

### macOS：GPUI 的 shader 與 Metal Toolchain（`runtime_shaders`）

GPUI 預設在**建置時**用 Xcode 的 Metal Toolchain（`xcrun metal`）預先編譯 shader。新版 Xcode 不再內建 Metal Toolchain，
沒有安裝時 gpui 的 build script 會失敗（開發機實測：`cannot execute tool 'metal' due to missing Metal Toolchain`）。所以 `rust/shell/Cargo.toml` 預設開啟 GPUI 的 `runtime_shaders` feature：
shader 改在**執行時**（第一次開視窗時）由系統的 Metal 編譯，不需要 Metal Toolchain，只要有 Xcode 或 Command Line Tools 即可。

- **沒有 Metal Toolchain（預設）**：保留 `runtime_shaders`，直接建置。這是目前唯一實際驗證過的方式。
- **已安裝 Metal Toolchain**（`xcodebuild -downloadComponent MetalToolchain`；`xcrun -sdk macosx metal --version` 能執行）：
  可以把 `rust/shell/Cargo.toml` 裡 gpui 的 `features = ["runtime_shaders"]` 拿掉，改用預先編譯的 shader（啟動略快）。
  **這條路尚未驗證**：開發機沒有 Metal Toolchain，已列入 [`docs/HANDOFF-rust.md`](docs/HANDOFF-rust.md) 待作者確認。

Windows 上這個 feature 沒有作用。

### 面板煙霧範例

確認 GPUI 視窗能變成瀏海面板（層級高於選單列、不搶焦點、不出現在 Dock）：

```sh
cd rust
cargo run -p musicat-shell --example panel_smoke
```

它會在有瀏海的螢幕頂端（沒有瀏海就用主螢幕與 200 × 32 的虛擬瀏海）顯示黑色長條，寫著 `MusiCat` 與 `現正播放`，
並把螢幕、幾何與視窗狀態印到 stdout，8 秒後自動結束。`MUSICAT_SMOKE_SECS=60` 可延長顯示時間（手動驗證用）。

### 疑難排解：helper 動態庫不見了

`musicat-platform-macos` 的 build script 會把 MediaRemote helper 編成 `target/<profile>/libmusicat_mediaremote_helper.dylib`。
若只手動刪掉這個檔案，Cargo 不會自動重建它（build script 只在 `helper/` 或 `build.rs` 變更時重跑）。執行：

```sh
cargo clean -p musicat-platform-macos
```

再重新建置即可。

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
