#!/bin/sh
# Swift → Rust 測試移植對照檢查。
# 用法：parity.sh [r1|r2]   無參數檢查全部列；帶階段則只檢查該 phase 的列。
# parity.tsv 欄位（tab 分隔）：swift_file rust_location swift_case_count phase waiver_reason
#   - rust_location 相對於 rust/，可為檔案或目錄，多處以逗號分隔，腳本加總其 #[test] 數。
#   - Rust #[test] 數少於 swift_case_count 且 waiver_reason 為空 → 印出該列並以非零離開。
set -u

RUST_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RUST_DIR" || exit 2
PHASE="${1:-}"
case "$PHASE" in
"" | r1 | r2) ;;
*)
    echo "用法：$0 [r1|r2]" >&2
    exit 2
    ;;
esac

count_tests() {
    # 逗號分隔的路徑清單 → #[test] 總數（不存在的路徑算 0）
    total=0
    for p in $(printf '%s' "$1" | tr ',' ' '); do
        [ -e "$p" ] || continue
        n=$(grep -rho '#\[test\]' "$p" 2>/dev/null | wc -l | tr -d ' ')
        total=$((total + n))
    done
    echo "$total"
}

fail=0
checked=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
    "" | "#"*) continue ;;
    esac
    swift_file=$(printf '%s' "$line" | cut -f1)
    rust_loc=$(printf '%s' "$line" | cut -f2)
    expected=$(printf '%s' "$line" | cut -f3)
    phase=$(printf '%s' "$line" | cut -f4)
    waiver=$(printf '%s' "$line" | cut -f5)

    if [ -n "$PHASE" ] && [ "$phase" != "$PHASE" ]; then
        continue
    fi
    checked=$((checked + 1))
    [ -n "$waiver" ] && continue

    actual=$(count_tests "$rust_loc")
    if [ "$actual" -lt "$expected" ]; then
        echo "MISSING: $swift_file -> $rust_loc (rust $actual < swift $expected, phase $phase)"
        fail=1
    fi
done <parity.tsv

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK: parity 通過（檢查 $checked 列）"
