# 貢獻指南（MusiCat Rust 版）

Rust 工作區在 `rust/` 目錄；下列指令都從 `rust/` 執行。

## core 純淨規則

`musicat-core`（`rust/core`）必須與平台、執行階段無關：

- 不得使用 `target_os`（連註解與字串裡出現這個字樣也會被擋）。
- 不得直接或間接依賴 `tokio`、`async-std`、`smol`、`reqwest`、`hyper`、`ureq`、`gpui`、`objc2*`、`windows`、`core-foundation`。
- 時間、網路、排程、背景工作一律透過注入的 `Clock`／`HttpClient`／`Spawner`。

由 `scripts/check-core-purity.sh` 檢查，CI 在 macOS 與 Windows 都會跑：

```sh
sh scripts/check-core-purity.sh
```

## parity 規則（Swift 測試移植）

Swift 版的測試要逐案移植到 Rust：案例與期望值照抄，Rust 測試名稱為 Swift 測試名的 `snake_case`。

- `parity.tsv` 以 tab 分隔，欄位為 `swift_file  rust_location  swift_case_count  phase(r1|r2)  waiver_reason`；`rust_location` 可用逗號列出多處，腳本會加總 `#[test]` 數。
- 每個任務完成時更新 `parity.tsv`。
- `scripts/parity.sh` 在 Rust `#[test]` 數少於 Swift 案例數且沒有豁免理由時失敗；可用 `scripts/parity.sh r1` 只檢查 R1 的列。

```sh
sh scripts/parity.sh        # 全部
sh scripts/parity.sh r1     # 只檢查 R1
```

## 測試怎麼跑

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p musicat-core --features testing   # CI 在所有平台都跑這個
sh scripts/check-core-purity.sh
```

需要螢幕才能跑的測試要標 `#[ignore]`，不會進 CI；本機用 `cargo test -- --ignored` 執行。

## 其他約定

- 私有 API 只准出現在 `platform-macos/src/mediaremote_control.rs` 與 `platform-macos/helper/`；`objc2` 只准出現在 `platform-macos/src/` 內：`panel.rs`（視窗繞路）、以及只用公開 AppKit／Foundation API 的 `screens.rs`、`tray.rs`、`power.rs`（見 ADR-0002）。
- Windows 程式碼一律標示「未在實機驗證」。
- 授權為 `MIT OR Apache-2.0`，User-Agent 固定為 `MusiCat`。
