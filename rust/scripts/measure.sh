#!/bin/sh
# 長時間量測 MusiCat 與它的 perl helper 的 CPU 與記憶體（驗收清單第 12、13 項，spec 第 10 節「記憶體與 CPU」）。
#
# 用法：measure.sh [-d 秒數=3600] [-i 秒數=10] [-o 檔案=musicat-measure.csv] [-p PID]
#   -d  總量測時間（秒）。app 先結束時提早收尾。
#   -i  取樣間隔（秒）。
#   -o  CSV 輸出檔（預設在目前目錄）；只寫這個檔案，不寫任何其他地方，也不連網。
#   -p  指定 app 的 PID；省略時以 `pgrep -x musicat` 找正在執行的 musicat。
#
# CSV 欄位：elapsed_s, musicat_cpu_pct, musicat_rss_kb, helper_cpu_pct, helper_rss_kb
# helper = app 的子行程中、命令列含 musicat_helper_main 的 perl（沒有時這兩欄留空）。
# CPU 與 RSS 都來自 `ps -o %cpu=,rss= -p <pid>`（%cpu 是 ps 回報的近期使用率，RSS 單位 KB）。
#
# 結尾印摘要與判定（寫在 rust/docs/HANDOFF-rust.md 的「量測」一節）：
#   RSS 成長 = 最後十分之一樣本的平均相對於最前十分之一樣本平均的變化百分比，app 與 helper 各算一次，
#   都 <= 10% 才是 PASS；CPU 只印平均與最大值，由人判斷（與 spike 的閒置 1.4% 同一個量級）。
#   樣本少於 10 筆時，「十分之一」取 1 筆；少於 2 筆則不判定。
#
# 結束碼：0 = 完成（含無法判定）、1 = RSS 成長超標（FAIL）、2 = 用法或環境錯誤。
set -u

DURATION=3600
INTERVAL=10
OUT=musicat-measure.csv
PID=
LIMIT_PCT=10

usage() {
    sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

die() {
    echo "measure.sh: $*" >&2
    exit 2
}

is_positive_number() {
    # 正的整數或小數
    printf '%s' "$1" | awk '/^[0-9]+(\.[0-9]+)?$/ && $1 + 0 > 0 { ok = 1 } END { exit ok ? 0 : 1 }'
}

while getopts 'd:i:o:p:h' opt; do
    case "$opt" in
    d) DURATION=$OPTARG ;;
    i) INTERVAL=$OPTARG ;;
    o) OUT=$OPTARG ;;
    p) PID=$OPTARG ;;
    *) usage ;;
    esac
done
shift $((OPTIND - 1))
[ "$#" -eq 0 ] || usage

is_positive_number "$DURATION" || die "-d 要是正數（秒）：$DURATION"
is_positive_number "$INTERVAL" || die "-i 要是正數（秒）：$INTERVAL"
[ -n "$OUT" ] || die "-o 不能是空字串"

if [ -z "$PID" ]; then
    PID=$(pgrep -x musicat | head -n 1)
    [ -n "$PID" ] || die "找不到正在執行的 musicat（pgrep -x musicat）。先啟動 app，或用 -p 指定 PID。"
    COUNT=$(pgrep -x musicat | wc -l | tr -d ' ')
    [ "$COUNT" -le 1 ] || echo "measure.sh: 有 $COUNT 個 musicat 行程，量測第一個（PID $PID）；要量別的請用 -p。" >&2
fi
case "$PID" in
'' | *[!0-9]*) die "PID 要是數字：$PID" ;;
esac
kill -0 "$PID" 2>/dev/null || die "PID $PID 不存在"

# 在 app（PID）的子行程中找命令列含 musicat_helper_main 的行程；沒有就印空字串。
find_helper() {
    pgrep -P "$PID" -f musicat_helper_main 2>/dev/null | head -n 1
}

# 印 "<cpu> <rss>"；行程不存在時什麼都不印。
sample() {
    ps -o %cpu=,rss= -p "$1" 2>/dev/null | awk 'NF >= 2 { print $1, $2 }'
}

echo "elapsed_s,musicat_cpu_pct,musicat_rss_kb,helper_cpu_pct,helper_rss_kb" >"$OUT" || die "無法寫入 $OUT"

echo "measure.sh: 量測 PID $PID，每 ${INTERVAL} 秒一筆，最長 ${DURATION} 秒，輸出 $OUT" >&2

START=$(date +%s)
while :; do
    NOW=$(date +%s)
    ELAPSED=$((NOW - START))

    APP=$(sample "$PID")
    if [ -z "$APP" ]; then
        echo "measure.sh: PID $PID 已結束，收尾。" >&2
        break
    fi
    set -- $APP
    APP_CPU=$1
    APP_RSS=$2

    HELPER_CPU=
    HELPER_RSS=
    HPID=$(find_helper)
    if [ -n "$HPID" ]; then
        H=$(sample "$HPID")
        if [ -n "$H" ]; then
            set -- $H
            HELPER_CPU=$1
            HELPER_RSS=$2
        fi
    fi

    echo "$ELAPSED,$APP_CPU,$APP_RSS,$HELPER_CPU,$HELPER_RSS" >>"$OUT"

    # 以「下一筆的時間」判斷是否超過總時間，確保最後一筆落在 DURATION 附近。
    if awk -v e="$ELAPSED" -v i="$INTERVAL" -v d="$DURATION" 'BEGIN { exit (e + i > d) ? 0 : 1 }'; then
        break
    fi
    sleep "$INTERVAL"
done

# ---------------------------------------------------------------------------- 摘要

awk -F, -v limit="$LIMIT_PCT" -v out="$OUT" '
function trend(rss, n, label,    k, i, first, last, second, change) {
    # rss[1..n]：該行程每個有值樣本的 RSS（KB）
    if (n < 2) {
        printf("%s RSS: %d 筆樣本，不足以判定趨勢\n", label, n)
        return "NA"
    }
    k = int(n / 10)
    if (k < 1) k = 1
    first = 0; last = 0
    for (i = 1; i <= k; i++) { first += rss[i]; last += rss[n - k + i] }
    first /= k; last /= k
    change = (first > 0) ? (last - first) / first * 100 : 0
    printf("%s RSS: 前 %d 筆平均 %.0f KB -> 後 %d 筆平均 %.0f KB，變化 %+.1f%%\n", label, k, first, k, last, change)
    if (n >= 20) {
        # 僅供參考、不計入判定：跳過第一個十分之一（暖機，例如 helper 記憶體在前幾分鐘會爬升後進入平穩）。
        second = 0
        for (i = 1; i <= k; i++) second += rss[k + i]
        second /= k
        if (second > 0) printf("  （參考，不計入判定）第 2 個十分之一平均 %.0f KB -> 後 %d 筆，變化 %+.1f%%\n", second, k, (last - second) / second * 100)
    }
    return (change <= limit) ? "PASS" : "FAIL"
}
function stats(label, sum, n, max) {
    if (n == 0) { printf("%s CPU: 沒有樣本\n", label); return }
    printf("%s CPU: 平均 %.1f%%、最大 %.1f%%（%d 筆）\n", label, sum / n, max, n)
}
NR == 1 { next }   # header
NF >= 3 {
    rows++
    last_elapsed = $1
    if ($2 != "") { ac_n++; ac_sum += $2; if ($2 + 0 > ac_max) ac_max = $2 + 0 }
    if ($3 != "") { ar_n++; ar[ar_n] = $3 + 0 }
    if ($4 != "") { hc_n++; hc_sum += $4; if ($4 + 0 > hc_max) hc_max = $4 + 0 }
    if ($5 != "") { hr_n++; hr[hr_n] = $5 + 0 }
}
END {
    printf("\n=== measure.sh 摘要（%s）===\n", out)
    printf("樣本數 %d，涵蓋 %s 秒\n", rows + 0, last_elapsed == "" ? 0 : last_elapsed)
    if (rows + 0 == 0) { print "沒有樣本，無法判定。"; exit 2 }
    if (rows < 10) print "注意：樣本少於 10 筆，「前後十分之一」各只取 1 筆，趨勢僅供參考。"
    stats("musicat", ac_sum, ac_n, ac_max)
    app = trend(ar, ar_n, "musicat")
    if (hc_n == 0 && hr_n == 0) {
        print "helper: 整段期間都沒有找到（perl_host 關閉、或命令列不含 musicat_helper_main）；helper 欄位為空，不判定。"
        helper = "NA"
    } else {
        stats("helper ", hc_sum, hc_n, hc_max)
        if (hr_n < rows) printf("注意：helper 只在 %d / %d 筆樣本中出現（可能中途被重啟）。\n", hr_n, rows)
        helper = trend(hr, hr_n, "helper ")
    }
    printf("判定（RSS 前後十分之一成長 <= %d%%；CPU 請自己看上面的平均，應與 spike 閒置的 1.4%% 同一個量級）：\n", limit)
    printf("  musicat RSS: %s\n  helper  RSS: %s\n", app, helper)
    if (app == "FAIL" || helper == "FAIL") { print "RESULT: FAIL"; exit 1 }
    if (app == "NA" && helper == "NA") { print "RESULT: 無法判定（樣本不足）"; exit 0 }
    print "RESULT: PASS"
    exit 0
}
' "$OUT"
exit $?
