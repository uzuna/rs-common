#!/usr/bin/env bash
# スケールテスト: リーダー数を段階的に増やして読み出し性能の変化を確認
#
# 使用法: ./bench_scale.sh [ステージ秒数(デフォルト30)]
#
# ステージ構成: 1 → 5 → 10 → 20 → 40 リーダー（累積追加）
# 各ステージで全リーダーの最新 stats を集計して表示する。
set -euo pipefail

STEP_DURATION=${1:-30}
DB_PATH="/dev/shm/sqlite_poc/blackboard.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="/tmp/sqlite_blackboard_scale"
WRITER="$WORKSPACE_ROOT/target/release/writer"
READER="$WORKSPACE_ROOT/target/release/reader"

STAGES=(1 5 10 20 40)
TOTAL_DURATION=$(( ${STAGES[-1]} * STEP_DURATION / 1 + 60 ))  # 余裕を持たせる

WRITER_PID=""
READER_PIDS=()

# ────────────────────────────────────────────────────────
# 関数: リーダーの最新 stats 1行を集計して表示
# 引数: $1=ステージ名, $2=リーダー数
# ────────────────────────────────────────────────────────
show_stats() {
    local label="$1"
    local count="$2"
    echo ""
    echo "  [$label] 各リーダーの最新 stats:"
    printf "  %-6s  %-18s  %-18s  %-12s  %s\n" \
        "ID" "latest avg/max(ms)" "count avg/max(ms)" "stale_max" "errors"
    printf "  %s\n" "------  ------------------  ------------------  ------------  ------"
    local sum_l_avg=0
    local valid=0
    for ((i=1; i<=count; i++)); do
        local log="$LOG_DIR/reader_${i}.log"
        if [[ ! -f "$log" ]]; then
            printf "  %-6s  (ログなし)\n" "r${i}"
            continue
        fi
        local last
        last=$(grep -a 'INFO.*回/s' "$log" 2>/dev/null | tail -1 || true)
        if [[ -z "$last" ]]; then
            printf "  %-6s  (統計データなし)\n" "r${i}"
            continue
        fi
        local l_avg l_max c_avg c_max stale errs
        l_avg=$(echo "$last" | grep -oP 'latest: avg=\K[0-9.]+' || echo "?")
        l_max=$(echo "$last" | grep -oP 'latest:.*?max=\K[0-9.]+' || echo "?")
        c_avg=$(echo "$last" | grep -oP 'count\([0-9]+\): avg=\K[0-9.]+' || echo "?")
        c_max=$(echo "$last" | grep -oP 'count\([0-9]+\):.*?max=\K[0-9.]+' || echo "?")
        stale=$(echo "$last" | grep -oP 'stale_max=\K[0-9]+' || echo "?")
        errs=$(echo "$last" | grep -oP 'errors=\K[0-9]+' || echo "?")
        printf "  %-6s  %8s / %-8s  %8s / %-8s  %-12s  %s\n" \
            "r${i}" "$l_avg" "$l_max" "$c_avg" "$c_max" "${stale}ms" "$errs"
        if [[ "$l_avg" != "?" ]]; then
            sum_l_avg=$(echo "$sum_l_avg + $l_avg" | bc)
            valid=$(( valid + 1 ))
        fi
    done
    if [[ $valid -gt 0 ]]; then
        local avg_avg
        avg_avg=$(echo "scale=3; $sum_l_avg / $valid" | bc)
        echo ""
        echo "  全体平均 latest avg: ${avg_avg}ms (${valid}/${count} リーダー)"
    fi
}

# ────────────────────────────────────────────────────────
# クリーンアップ
# ────────────────────────────────────────────────────────
cleanup() {
    echo ""
    echo "--- クリーンアップ ---"
    [[ -n "$WRITER_PID" ]] && kill "$WRITER_PID" 2>/dev/null || true
    for pid in "${READER_PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT

# ────────────────────────────────────────────────────────
# メイン処理
# ────────────────────────────────────────────────────────
echo "=== スケールテスト: リーダー数と読み出し性能 ==="
echo "ステージ: ${STAGES[*]} リーダー, 各 ${STEP_DURATION}s"
echo "DB: $DB_PATH"
echo ""

echo "--- ビルド ---"
cargo build --release -p sqlite-blackboard 2>&1

rm -f "$DB_PATH"
mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR"/*.log

# Writer を先に起動してスキーマを初期化させる
echo ""
echo "--- Writer 起動 (${TOTAL_DURATION}s) ---"
"$WRITER" --db "$DB_PATH" --channels 1 --duration "$TOTAL_DURATION" \
    > "$LOG_DIR/writer.log" 2>&1 &
WRITER_PID=$!
echo "  PID=$WRITER_PID, DB 初期化待ち..."
sleep 2

# ステージごとにリーダーを累積追加
for STAGE_COUNT in "${STAGES[@]}"; do
    PREV_COUNT=${#READER_PIDS[@]}
    ADDED=$(( STAGE_COUNT - PREV_COUNT ))

    # 不足分のリーダーを追加起動
    while [[ ${#READER_PIDS[@]} -lt $STAGE_COUNT ]]; do
        rid=$(( ${#READER_PIDS[@]} + 1 ))
        "$READER" \
            --db "$DB_PATH" \
            --topic sensor/0 \
            --reader-id "r${rid}" \
            --duration "$TOTAL_DURATION" \
            >> "$LOG_DIR/reader_${rid}.log" 2>&1 &
        READER_PIDS+=($!)
    done

    echo ""
    echo "┌────────────────────────────────────────────────────────────┐"
    printf  "│  ステージ: %2d リーダー (+%d 追加)  %${STEP_DURATION}s 計測中...%*s│\n" \
        "$STAGE_COUNT" "$ADDED" "" 0 ""
    echo "└────────────────────────────────────────────────────────────┘"
    sleep "$STEP_DURATION"

    show_stats "stage=${STAGE_COUNT}" "$STAGE_COUNT"
done

echo ""
echo "=== スケールテスト完了 ==="
echo "詳細ログ: $LOG_DIR/"
echo "Writer ログ: $LOG_DIR/writer.log"
