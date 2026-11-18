#!/bin/sh
# 檢查 musicat-core 的純淨規則（CI 與本機皆跑）：
#   (a) core/src 內不得出現 target_os 字樣（含註解，避免繞過）。
#   (b) musicat-core 的 normal 依賴樹不得含禁用 crate。
set -u

RUST_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RUST_DIR" || exit 2

FORBIDDEN="tokio async-std smol reqwest hyper ureq gpui windows core-foundation"
# 以 objc2 開頭的 crate（objc2、objc2-foundation…）一律禁用。
FORBIDDEN_PREFIX="objc2"

fail=0

if hits=$(grep -rn 'target_os' core/src); then
    echo "FAIL: musicat-core 不得使用 target_os："
    echo "$hits"
    fail=1
fi

if ! tree=$(cargo tree -p musicat-core --edges normal --prefix none); then
    echo "FAIL: 無法執行 cargo tree -p musicat-core"
    exit 2
fi

crates=$(printf '%s\n' "$tree" | awk '{print $1}' | sort -u)
for name in $crates; do
    for bad in $FORBIDDEN; do
        if [ "$name" = "$bad" ]; then
            echo "FAIL: musicat-core 依賴了禁用 crate：$name"
            fail=1
        fi
    done
    case "$name" in
    "$FORBIDDEN_PREFIX"*)
        echo "FAIL: musicat-core 依賴了禁用 crate：$name"
        fail=1
        ;;
    esac
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK: musicat-core 純淨檢查通過"
