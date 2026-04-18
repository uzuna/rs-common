#!/usr/bin/env bash
# フェイルオーバーテスト: Writer 異常終了と復帰でリーダーへの影響を確認
#
# 使用法: ./bench_failover.sh [フェーズ秒数(デフォルト15)] [リーダー数(デフォルト5)]
#
# フェーズ構成:
#   1. 通常稼働          → errors=0, stale_max が小さいことを確認
#   2. Writer SIGKILL   → errors=0 のまま, stale_max が増大することを確認
#   3. Writer 再起動     → stale_max が解消し, データが再び流れることを確認
set -euo pipefail

PHASE_DURATION=${1:-15}
NUM_READERS=${2:-5}
TOPIC="sensor/0"
DB_PATH="/dev/shm/sqlite_poc/blackboard.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="/tmp/sqlite_blackboard_failover"
WRITER="$WORKSPACE_ROOT/target/release/writer"
READER="$WORKSPACE_ROOT/target/release/reader"

TOTAL_DURATION=$(( PHASE_DURATION * 4 + 10 ))
WRITER_PID=""
READER_PIDS=()

# ────────────────────────────────────────────────────────
# 関数: リーダーの最新 stats を集計して表示
# 引数: $1=フェーズ名
# ────────────────────────────────────────────────────────
show_stats() {
    local label="$1"
    echo ""
    echo "  [$label] 各リーダーの最新 stats (errors=0 かつ stale_max に注目):"
    printf "  %-6s  %-18s  %-18s  %-12s  %s\n" \
        "ID" "latest avg/max(ms)" "count avg/max(ms)" "stale_max" "errors"
    printf "  %s\n" "------  ------------------  ------------------  ------------  ------"

    local total_errors=0
    local max_stale=0
    local valid=0

    for ((i=1; i<=NUM_READERS; i++)); do
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
        stale=$(echo "$last" | grep -oP 'stale_max=\K[0-9]+' || echo "0")
        errs=$(echo "$last" | grep -oP 'errors=\K[0-9]+' || echo "0")
        printf "  %-6s  %8s / %-8s  %8s / %-8s  %-12s  %s\n" \
            "r${i}" "$l_avg" "$l_max" "$c_avg" "$c_max" "${stale}ms" "$errs"

        if [[ "$stale" != "?" ]]; then
            if (( stale > max_stale )); then max_stale=$stale; fi
        fi
        if [[ "$errs" != "?" ]]; then
            total_errors=$(( total_errors + errs ))
        fi
        valid=$(( valid + 1 ))
    done

    echo ""
    if [[ $total_errors -eq 0 ]]; then
        echo "  ✓ errors 合計: 0 (リーダーにエラーなし)"
    else
        echo "  ✗ errors 合計: $total_errors"
    fi
    echo "  stale_max 最大値: ${max_stale}ms"
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
# メイン
# ────────────────────────────────────────────────────────
echo "=== フェイルオーバーテスト ==="
echo "リーダー数: $NUM_READERS, 各フェーズ: ${PHASE_DURATION}s, トピック: $TOPIC"
echo "DB: $DB_PATH"
echo ""

echo "--- ビルド ---"
cargo build --release -p sqlite-blackboard 2>&1

rm -f "$DB_PATH"
mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR"/*.log

# ==================== 準備: Writer 起動 + DB 初期化 ====================
echo ""
echo "--- Writer 起動 (初期化) ---"
"$WRITER" --db "$DB_PATH" --channels 1 --duration "$TOTAL_DURATION" \
    > "$LOG_DIR/writer.log" 2>&1 &
WRITER_PID=$!
echo "  Writer PID=$WRITER_PID, DB 初期化待ち..."
sleep 2

# リーダーを全フェーズ通して稼働（--topic で自動追従）
echo ""
echo "--- リーダー ${NUM_READERS} 本を起動 (--topic ${TOPIC}) ---"
for ((i=1; i<=NUM_READERS; i++)); do
    "$READER" \
        --db "$DB_PATH" \
        --topic "$TOPIC" \
        --reader-id "r${i}" \
        --duration "$TOTAL_DURATION" \
        >> "$LOG_DIR/reader_${i}.log" 2>&1 &
    READER_PIDS+=($!)
done
echo "  PID: ${READER_PIDS[*]}"

# ==================== フェーズ1: 通常稼働 ====================
echo ""
echo "┌────────────────────────────────────────────────────────────────┐"
echo "│  フェーズ1: 通常稼働 (${PHASE_DURATION}s)                               │"
echo "│  期待値: errors=0, stale_max が数ms 以下                       │"
echo "└────────────────────────────────────────────────────────────────┘"
sleep "$PHASE_DURATION"
show_stats "フェーズ1: 通常稼働"

# ==================== フェーズ2: Writer 強制終了 ====================
echo ""
echo "┌────────────────────────────────────────────────────────────────┐"
echo "│  フェーズ2: Writer を SIGKILL で強制終了 (${PHASE_DURATION}s)            │"
echo "│  期待値: errors=0 のまま, stale_max が増大する                 │"
echo "└────────────────────────────────────────────────────────────────┘"
echo "  kill -9 $WRITER_PID"
kill -9 "$WRITER_PID" 2>/dev/null || true
WRITER_PID=""
echo "  Writer 強制終了. リーダーは引き続き稼働中..."
sleep "$PHASE_DURATION"
show_stats "フェーズ2: Writer 停止"

# ==================== フェーズ3: Writer 再起動 ====================
echo ""
echo "┌────────────────────────────────────────────────────────────────┐"
echo "│  フェーズ3: Writer 再起動 (${PHASE_DURATION}s)                          │"
echo "│  期待値: stale_max が解消し, count が回復する                   │"
echo "└────────────────────────────────────────────────────────────────┘"
"$WRITER" --db "$DB_PATH" --channels 1 --duration $(( PHASE_DURATION + 10 )) \
    >> "$LOG_DIR/writer.log" 2>&1 &
WRITER_PID=$!
echo "  Writer 再起動 PID=$WRITER_PID"
echo "  ※ リーダーは --topic により新 channel_id を自動検出 (最大5秒)"
sleep "$PHASE_DURATION"
show_stats "フェーズ3: Writer 復帰後"

# ==================== サマリー ====================
echo ""
echo "=== フェイルオーバーテスト完了 ==="
echo ""
echo "確認ポイント:"
echo "  - フェーズ1→2→3 を通じて errors=0 であること"
echo "  - フェーズ2 で stale_max が増大し, フェーズ3 で解消すること"
echo "  - フェーズ3 で count が 0 より大きく回復すること"
echo ""
echo "詳細ログ: $LOG_DIR/"
