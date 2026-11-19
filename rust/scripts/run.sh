#!/bin/sh
# MusiCat（Rust 版）的執行腳本，等同 Swift 版的 scripts/run.sh：先建置再啟動。
# 用法：sh rust/scripts/run.sh [musicat 的參數...]   （從哪個目錄執行都可以）
#
# - 一律 --release：spike 的 GPUI 透明度是以 release 執行檔驗證的。
# - 環境變數照常傳給程式：MUSICAT_LOG=1、MUSICAT_AUTO_QUIT_SECS=<n>、MUSICAT_CONFIG=<path>。
set -eu

RUST_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RUST_DIR"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/musicat-run.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# cargo 的輸出照常顯示在終端機（不隱藏），同時留一份供失敗時判斷原因。
# POSIX sh 沒有 pipefail／PIPESTATUS，所以把 cargo 的結束碼寫進檔案再讀回來。
{
    rc=0
    cargo build --release -p musicat-app 2>&1 || rc=$?
    echo "$rc" >"$TMP/status"
} | tee "$TMP/build.log"
status="$(cat "$TMP/status")"
LOG="$TMP/build.log"

if [ "$status" -ne 0 ]; then
    if grep -qi -E "metal toolchain|tool 'metal'|xcrun.*metal" "$LOG"; then
        cat >&2 <<'MSG'

musicat: 建置失敗，而且訊息提到 Metal（shader 編譯）。
  GPUI 預設在建置時用 Xcode 的 Metal Toolchain 預先編譯 shader；新版 Xcode 不再內建它。
  本專案的作法：rust/shell/Cargo.toml 啟用 gpui 的 `runtime_shaders` feature，
  shader 改在執行時由系統編譯，不需要 Metal Toolchain。
  如果你看到這個訊息，請確認 rust/shell/Cargo.toml 的 gpui 還帶著 features = ["runtime_shaders"]。
  詳見 rust/README.md 的「macOS：GPUI 的 shader 與 Metal Toolchain（runtime_shaders）」。
MSG
    else
        echo "musicat: 建置失敗（cargo 結束碼 $status），訊息見上方。" >&2
    fi
    exit "$status"
fi

if [ ! -f target/release/libmusicat_mediaremote_helper.dylib ]; then
    cat >&2 <<'MSG'

musicat: 找不到 target/release/libmusicat_mediaremote_helper.dylib（現正播放 helper 動態庫）。
  build script 只在 helper/ 或 build.rs 變更時重跑，所以手動刪掉動態庫後 cargo 不會自己重建。
  請執行：  cd rust && cargo clean -p musicat-platform-macos
  然後重新執行這個腳本。沒有它程式仍會啟動，但只剩視窗標題備援。
MSG
    exit 1
fi

# exec 之後 trap 不會執行，先自己清掉暫存。
rm -rf "$TMP"
exec target/release/musicat "$@"
