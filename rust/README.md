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

## 執行（macOS）

```sh
cd rust
cargo run --release -p musicat-app
```

可執行檔是 `target/release/musicat`。它會在有瀏海的螢幕（沒有就用主螢幕與虛擬瀏海）頂端顯示面板，並在選單列放一個「♪」圖示
（選單：顯示螢幕、歌詞來源、歌詞條、perl 宿主、結束）。沒有 Dock 圖示。

- 讀取系統現正播放需要 helper 動態庫 `libmusicat_mediaremote_helper.dylib` 放在**可執行檔旁邊**（建置時自動產生在
  `target/<profile>/`）。找不到時印一行 `system now-playing unavailable` 並只用視窗標題備援，程式照常執行。
- 有歌詞時，Expanded 畫面控制列左側會顯示一個小的歌詞來源標籤（例如 `lrclib.net ↗`），點擊會用預設瀏覽器開啟該歌詞的來源網站（只開 `https://` 網址）。
- 選單列圖示很多時，macOS 會把放不下的圖示藏起來（有瀏海的螢幕特別明顯）；看不到「♪」時請按住 ⌘ 拖曳其他圖示騰出位置。
- **結束與 helper**：從選單「結束」，以及登出、關機與 `NSRunningApplication.terminate()` 這些經由 AppKit 結束通知的路徑，都會先停掉 perl helper 再結束。
  Activity Monitor 的強制結束（`kill -9`）與 `SIGINT`／`SIGTERM`（Ctrl-C、`kill`）會直接殺掉行程，**不會**呼叫停止來源的程式；
  這時 helper 在 stdin 的 pipe 關閉（父行程消失）時自行結束，所以同樣不會留下孤兒行程（`kill -9`、`SIGTERM`、`SIGINT` 都已實測 0.5 秒內 helper 消失）。

開發與測試用的環境變數（日誌一律寫到 stderr）：

| 變數 | 作用 |
|---|---|
| `MUSICAT_LOG=1` | 印出現正播放（**含曲名與演出者**）、歌詞狀態（行數，不含內容）、外殼狀態與選單列狀態。沒設時不印任何與曲目有關的內容。 |
| `MUSICAT_AUTO_QUIT_SECS=<n>` | n 秒後走與選單「結束」相同的路徑結束（自動化驗證用）。 |
| `MUSICAT_CONFIG=<path>` | 改用這個設定檔（測試用，避免動到真正的設定檔）。第一次存檔時，若給定路徑的上層資料夾不存在，程式會把它建起來。 |

例如：`MUSICAT_LOG=1 MUSICAT_AUTO_QUIT_SECS=25 cargo run --release -p musicat-app`。
也可以用 `sh rust/scripts/run.sh`（從任何目錄都行；先 `cargo build --release -p musicat-app` 再啟動，參數與環境變數照常傳給程式）。

長時間量測 CPU 與記憶體：App 執行中另開終端機跑 `sh rust/scripts/measure.sh -d 3600`（CSV 與摘要；判準見 [`docs/HANDOFF-rust.md`](docs/HANDOFF-rust.md)）。
驗收清單（給作者）也在那份文件。

實際使用的設定檔路徑（以及它是否存在）在啟動時一律印到 stderr，不必開 `MUSICAT_LOG`：
`musicat [+  0.000s] config <path> (missing: using defaults)`。

## 設定

設定檔是 TOML，位置：

| 平台 | 路徑 |
|---|---|
| macOS | `~/Library/Application Support/MusiCat/musicat.toml` |
| Windows | `%APPDATA%\MusiCat\musicat.toml` |
| 其他 | `$XDG_CONFIG_HOME/musicat/musicat.toml`，沒有就用 `~/.config/musicat/musicat.toml` |

檔案不存在時全部用預設值。完整範例（每個鍵都列出，值為預設值）：

```toml
mode = "notch"                 # 目前只有 "notch"
screen = "1"                   # 選填：顯示在哪個螢幕（螢幕 id）；省略 = 自動
lyric_strip_visible = true     # 瀏海下方的歌詞條

[sources]                      # 歌詞來源開關；兩個都關 = 不查歌詞、不送任何請求
lrclib = true
netease = true

[macos]
perl_host = true               # false = 不借 /usr/bin/perl 讀現正播放，只剩視窗標題備援

[notch]                        # 沒有實體瀏海的螢幕上畫的虛擬瀏海
virtual_width = 200            # 40 到 2000
virtual_height = 32            # 10 到 400
```

- **關閉歌詞來源**：把 `[sources]` 裡對應的鍵設成 `false`。
- **關閉 perl 宿主**：`[macos] perl_host = false`。
- **壞掉的檔案不會被悄悄覆蓋**：語法錯誤、不是 UTF-8、超過 1 MiB 的檔案一律以預設值啟動並在日誌印警告，程式不會因此結束。
  第一次需要存檔時，原檔會先被改名為 `musicat.toml.bak`（蓋掉更舊的 `.bak`）再寫入新檔。
- 未知的鍵與型別錯誤的值只會被警告並忽略（只有那個欄位退回預設值）；日誌只印鍵名，不印值。
- **設定檔只在啟動時讀取一次**：程式執行中手動改檔，下一次選單變更（或歌詞條折疊箭頭的同步）就會用記憶體中的內容把整個檔案寫回、蓋掉你的修改。要手動編輯請先結束 MusiCat。
- 程式存設定時會把檔案重寫成標準格式：**手寫的註解與未知的鍵不會保留**。
- `screen` 記下的螢幕 id 不會因為該螢幕暫時沒接（拔掉、闔上）而被清掉：面板暫時移到預設螢幕，螢幕接回來後（每 2 秒偵測一次）移回去。
- 選單的變更立即生效並寫回設定檔；「結束」與點選目前已是該值的項目不會寫檔。Expanded 畫面右下角的歌詞條折疊箭頭也會被記住（最多 2 秒內寫回）。
- 環境變數 `HOME`（Windows 為 `APPDATA`、其他平台另有 `XDG_CONFIG_HOME`）是空的或相對路徑時一律不採用。
- 設定檔若是符號連結，程式存檔時會把它換成一般檔案（連結的目標不會被更新）。

## 隱私

查歌詞時，正在播放曲目的資料會送到下面兩個主機（曲名與演出者可能是 YouTube 影片或頻道的任意標題）；除此之外不會傳送其他曲目資料或使用者資料：

- **lrclib.net**：曲名、演出者、專輯名稱與取整數秒的曲長（沒有專輯或曲長時省略該欄）。精確查詢沒有結果時，會再以「曲名 演出者」當搜尋字串查一次。
- **music.163.com**（網易雲）：先以「曲名 演出者」搜尋，再用搜尋結果裡找到的歌曲 id 取歌詞。

標準的 HTTP 請求本身會讓對方看到你的 IP 位址與 User-Agent（固定為 `MusiCat`），網易雲的請求另帶固定的 Referer 標頭（`https://music.163.com/`）；這些之外不帶任何其他使用者資料。兩個歌詞來源可以個別在選單或 TOML 設定檔中關閉。

點擊 Expanded 畫面的歌詞來源標籤時，是你的預設瀏覽器自己連線到該網站（與你在瀏覽器裡開那個網址相同）；MusiCat 只把網址交給系統，不會為此另外送出任何請求或資料。

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
