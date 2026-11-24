#!/bin/sh
# 私有 API 圍欄（CI 與本機皆跑）：
# `MRMediaRemote`、`dlopen`、`dlsym` 只准出現在
#   (a) rust/platform-macos/helper/                    （C，讀取，只在 perl 行程內跑）
#   (b) rust/platform-macos/src/mediaremote_control.rs （只送控制指令）
# 其餘 rust/ 下的檔案一律不得出現。略過 rust/target、rust/docs、rust/scripts
# （文件與本腳本本身會提到這些字）。
set -u

RUST_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RUST_DIR" || exit 2

PATTERN='MRMediaRemote|dlopen|dlsym'

# find 先剪掉不掃的目錄與允許清單，再讓 grep 掃剩下的檔案（-I 略過二進位檔）。
# 路徑以 ./ 開頭做完整比對，所以其他目錄裡同名的 helper/ 或 mediaremote_control.rs 仍會被掃。
hits=$(find . \
    \( -path ./target -o -path ./docs -o -path ./scripts -o -path ./.git \
       -o -path ./platform-macos/helper \) -prune -o \
    -type f ! -path ./platform-macos/src/mediaremote_control.rs \
    -exec grep -HnIE "$PATTERN" {} +)
status=$?
# find 本身出錯（exec 的 grep 沒找到時回傳 1，不算 find 失敗）。
[ "$status" -gt 1 ] && { echo "FAIL: 無法掃描 rust/"; exit 2; }

if [ -n "$hits" ]; then
    echo "FAIL: 私有 API 字樣（$PATTERN）出現在允許清單之外："
    echo "$hits"
    exit 1
fi
echo "OK: 私有 API 圍欄檢查通過"
