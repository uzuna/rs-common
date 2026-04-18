#!/usr/bin/env bash
# Writer パラメータスイープベンチマーク
#
# テスト内容:
#   Sweep A: データサイズ  128B〜2KB  (hz=1000, commit_ms=100 固定)
#   Sweep B: 書き込み周波数 30〜1000Hz (size=512B, commit_ms=100 固定)
#   Sweep C: コミット間隔   10〜100ms  (size=512B, hz=1000 固定)
#
# 使用法: ./bench_writer.sh [ステップ秒数(デフォルト15)]
#
# 各ステップの計測指標:
#   tps       実効スループット (INSERT/s)
#   ach%      目標Hz に対する達成率 (%)
#   c_avg/max コミット 1 回の wall time avg/max (ms)
#   b_avg     コミット 1 回あたりのバッチサイズ平均 (件)
#   wal_kb    WAL ファイルサイズ (KB)
set -euo pipefail

STEP_DUR=${1:-15}
DB_PATH="/dev/shm/sqlite_poc/blackboard.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="/tmp/sqlite_blackboard_writer_bench"
WRITER="$WORKSPACE_ROOT/target/release/writer"

# ────────────────────────────────────────────────────────
# ビルド
# ────────────────────────────────────────────────────────
echo "--- ビルド (release) ---"
cargo build --release -p sqlite-blackboard 2>&1
echo ""

mkdir -p "$LOG_DIR"

# ────────────────────────────────────────────────────────
# ログからウォームアップ除外後の平均値を算出する
#   引数: ログファイルパス
#   出力: "tps ach% c_avg c_max b_avg wal_kb" (スペース区切り)
# ────────────────────────────────────────────────────────
avg_metric() {
    local vals="$1"
    echo "$vals" | awk 'NF{s+=$1;n++} END{if(n>0) printf "%.3f",s/n; else print "?"}'
}

parse_log() {
    local log="$1"
    # 先頭 3 行（ウォームアップ）を除いた [writer] 行を使用
    local lines
    lines=$(grep -a '\[writer\]' "$log" 2>/dev/null | tail -n +"4" || true)

    if [[ -z "$lines" ]]; then
        echo "? ? ? ? ? ?"
        return
    fi

    local tps ach cavg cmax bavg wal
    # tps=987
    tps=$(avg_metric "$(echo "$lines" | grep -oP 'tps=\K[0-9]+')")
    # (87%/1000)  → 87
    ach=$(avg_metric "$(echo "$lines" | grep -oP '[0-9]+(?=%/)')")
    # commit: avg=1.234ms
    cavg=$(avg_metric "$(echo "$lines" | grep -oP 'commit: avg=\K[0-9.]+')")
    # commit: avg=...ms max=2.345ms
    cmax=$(avg_metric "$(echo "$lines" | grep -oP 'commit: avg=[0-9.]+ms max=\K[0-9.]+')")
    # batch: avg=98
    bavg=$(avg_metric "$(echo "$lines" | grep -oP 'batch: avg=\K[0-9.]+')")
    # wal_kb=1234
    wal=$(avg_metric "$(echo "$lines" | grep -oP 'wal_kb=\K[0-9]+')")

    echo "$tps $ach $cavg $cmax $bavg $wal"
}

# ────────────────────────────────────────────────────────
# 1 ケース実行
#   引数: log_tag hz data_size commit_ms
# ────────────────────────────────────────────────────────
run_case() {
    local tag="$1" hz="$2" size="$3" cms="$4"
    local log="$LOG_DIR/${tag}.log"
    rm -f "$DB_PATH" "$log"

    "$WRITER" \
        --db "$DB_PATH" \
        --hz "$hz" \
        --data-size "$size" \
        --commit-ms "$cms" \
        --duration "$STEP_DUR" \
        > "$log" 2>&1

    parse_log "$log"
}

# ────────────────────────────────────────────────────────
# テーブルヘッダー出力
# ────────────────────────────────────────────────────────
print_header() {
    printf "  %-12s  %7s  %5s  %10s  %10s  %8s  %7s\n" \
        "$1" "tps" "ach%" "c_avg(ms)" "c_max(ms)" "batch_avg" "wal_kb"
    printf "  %s\n" "------------  -------  -----  ----------  ----------  --------  -------"
}

print_row() {
    local label="$1"; shift
    local vals=($@)  # tps ach cavg cmax bavg wal
    printf "  %-12s  %7s  %5s  %10s  %10s  %8s  %7s\n" \
        "$label" "${vals[0]}" "${vals[1]}" "${vals[2]}" "${vals[3]}" "${vals[4]}" "${vals[5]}"
}

# ════════════════════════════════════════════════════════
# Sweep A: データサイズ変化 (hz=1000, commit_ms=100)
# ════════════════════════════════════════════════════════
echo "════════════════════════════════════════════════════"
echo " Sweep A: データサイズ (hz=1000, commit_ms=100ms)"
echo "════════════════════════════════════════════════════"
print_header "size(B)"

for size in 128 256 512 1024 2048; do
    printf "  %-12s  running...\r" "${size}B"
    vals=$(run_case "sweep_a_${size}" 1000 "$size" 100)
    print_row "${size}B" $vals
done

echo ""

# ════════════════════════════════════════════════════════
# Sweep B: 書き込み周波数変化 (size=512B, commit_ms=100)
# ════════════════════════════════════════════════════════
echo "════════════════════════════════════════════════════"
echo " Sweep B: 書き込み周波数 (size=512B, commit_ms=100ms)"
echo "════════════════════════════════════════════════════"
print_header "hz"

for hz in 30 100 300 1000; do
    printf "  %-12s  running...\r" "${hz}Hz"
    vals=$(run_case "sweep_b_${hz}" "$hz" 512 100)
    print_row "${hz}Hz" $vals
done

echo ""

# ════════════════════════════════════════════════════════
# Sweep C: コミット間隔変化 (size=512B, hz=1000)
# 計測のポイント:
#   - c_avg/c_max: commit_ms が短いほど BEGIN..COMMIT オーバーヘッド比率増大
#   - batch_avg: 理論値 = hz × commit_ms / 1000 と一致するか
#   - wal_kb: commit_ms が短い → WAL チェックポイント頻度増大の影響
# ════════════════════════════════════════════════════════
echo "════════════════════════════════════════════════════"
echo " Sweep C: コミット間隔 (size=512B, hz=1000)"
echo "════════════════════════════════════════════════════"
print_header "commit_ms"

for cms in 10 25 50 100; do
    printf "  %-12s  running...\r" "${cms}ms"
    vals=$(run_case "sweep_c_${cms}" 1000 512 "$cms")
    print_row "${cms}ms" $vals
done

echo ""
echo "詳細ログ: $LOG_DIR/"
echo ""
echo "【指標の読み方】"
echo "  tps     : 実際に INSERT できた件数/秒。目標 hz と差があれば OS ジッターの限界。"
echo "  ach%    : tps / 目標hz の達成率。1000Hz では 95%前後が現実的な上限目安。"
echo "  c_avg   : BEGIN〜COMMIT の平均時間。commit_ms より大きければ詰まりの兆候。"
echo "  batch_avg: コミット1回あたりの件数。理論値(hz×commit_ms/1000)からの乖離に注目。"
echo "  wal_kb  : WAL サイズ。commit_ms が短いと細かく刻まれて小さく保たれる。"
