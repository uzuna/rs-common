#!/usr/bin/env bash
# 多言語スケールテスト: Rust / Go / Python リーダーを並列起動して性能比較
#
# 使用法: ./bench_scale_poly.sh [ステージ秒数(デフォルト30)]
#
# ステージ構成: 1 → 5 → 10 → 20 リーダー（言語ごとに累積追加）
# 各ステージで全言語のレイテンシを集計して比較表示する。
set -euo pipefail

STEP_DURATION=${1:-30}
DB_PATH="/dev/shm/sqlite_poc/blackboard.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="/tmp/sqlite_blackboard_poly"

WRITER="$WORKSPACE_ROOT/target/release/writer"
READER_RUST="$WORKSPACE_ROOT/target/release/reader"
READER_GO="$SCRIPT_DIR/go/reader_go"
READER_PY="$SCRIPT_DIR/reader.py"

STAGES=(1 5 10 20)
TOTAL_DURATION=$(( ${STAGES[-1]} * ${#STAGES[@]} * STEP_DURATION + 60 ))

WRITER_PID=""
RUST_PIDS=()
GO_PIDS=()
PYTHON_PIDS=()

# ────────────────────────────────────────────────────────────
# 各言語リーダーのログを集計して 1 行に出力する
# 引数: $1=言語名 $2=プレフィックス文字 $3=リーダー数
# ────────────────────────────────────────────────────────────
show_lang_stats() {
    local lang="$1"
    local prefix="$2"
    local count="$3"

    local sum_l_avg=0 valid=0
    local l_max_all=0 c_avg_all=0 c_max_all=0 stale_max_all=0 total_errors=0

    for ((i=1; i<=count; i++)); do
        local log="$LOG_DIR/reader_${prefix}${i}.log"
        [[ -f "$log" ]] || continue
        local last
        last=$(grep -a 'INFO.*回/s' "$log" 2>/dev/null | tail -1 || true)
        [[ -z "$last" ]] && continue

        local l_avg l_max c_avg c_max stale errs
        l_avg=$(echo "$last" | grep -oP 'latest: avg=\K[0-9.]+' || echo "")
        l_max=$(echo "$last" | grep -oP 'latest:.*?max=\K[0-9.]+' || echo "")
        c_avg=$(echo "$last" | grep -oP 'count\([0-9]+\): avg=\K[0-9.]+' || echo "")
        c_max=$(echo "$last" | grep -oP 'count\([0-9]+\):.*?max=\K[0-9.]+' || echo "")
        stale=$(echo "$last" | grep -oP 'stale_max=\K[0-9]+' || echo "0")
        errs=$(echo "$last" | grep -oP 'errors=\K[0-9]+' || echo "0")

        [[ -z "$l_avg" ]] && continue
        sum_l_avg=$(echo "$sum_l_avg + $l_avg" | bc)
        valid=$(( valid + 1 ))
        if [[ -n "$l_max" ]]; then
            local cmp
            cmp=$(echo "$l_max > $l_max_all" | bc)
            [[ "$cmp" -eq 1 ]] && l_max_all=$l_max
        fi
        if [[ -n "$c_avg" ]]; then
            c_avg_all=$(echo "$c_avg_all + $c_avg" | bc)
        fi
        if [[ -n "$c_max" ]]; then
            local cmp2
            cmp2=$(echo "$c_max > $c_max_all" | bc)
            [[ "$cmp2" -eq 1 ]] && c_max_all=$c_max
        fi
        if [[ "$stale" -gt "$stale_max_all" ]]; then stale_max_all=$stale; fi
        total_errors=$(( total_errors + errs ))
    done

    if [[ $valid -eq 0 ]]; then
        printf "  %-8s  %3d  %s\n" "$lang" "$count" "(統計データなし)"
        return
    fi

    local avg_l_avg avg_c_avg
    avg_l_avg=$(echo "scale=3; $sum_l_avg / $valid" | bc)
    avg_c_avg=$(echo "scale=3; $c_avg_all / $valid" | bc)

    printf "  %-8s  %3d  %8s / %-8s  %8s / %-8s  %-12s  %s\n" \
        "$lang" "$valid" \
        "$avg_l_avg" "$l_max_all" \
        "$avg_c_avg" "$c_max_all" \
        "${stale_max_all}ms" \
        "$total_errors"
}

# ────────────────────────────────────────────────────────────
# 全言語の集計表示
# 引数: $1=リーダー数/言語
# ────────────────────────────────────────────────────────────
show_stats_poly() {
    local count="$1"
    local total=$(( count * 3 ))
    echo ""
    echo "  [stage: ${count} readers/言語, 合計 ${total} readers] 言語別パフォーマンス比較:"
    printf "  %-8s  %-3s  %-18s  %-18s  %-12s  %s\n" \
        "言語" "N" "latest avg/max(ms)" "count avg/max(ms)" "stale_max" "errors"
    printf "  %s\n" \
        "--------  ---  ------------------  ------------------  ------------  ------"
    show_lang_stats "Rust"   "r" "$count"
    show_lang_stats "Go"     "g" "$count"
    show_lang_stats "Python" "p" "$count"
}

# ────────────────────────────────────────────────────────────
# クリーンアップ
# ────────────────────────────────────────────────────────────
cleanup() {
    echo ""
    echo "--- クリーンアップ ---"
    [[ -n "$WRITER_PID" ]] && kill "$WRITER_PID" 2>/dev/null || true
    for pid in "${RUST_PIDS[@]}" "${GO_PIDS[@]}" "${PYTHON_PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT

# ────────────────────────────────────────────────────────────
# メイン処理
# ────────────────────────────────────────────────────────────
echo "=== 多言語スケールテスト: Rust / Go / Python リーダー性能比較 ==="
echo "ステージ: ${STAGES[*]} readers/言語, 各 ${STEP_DURATION}s"
echo "DB: $DB_PATH"
echo ""

# ── ビルド ───────────────────────────────────────────────────
echo "--- ビルド (Rust) ---"
cargo build --release -p sqlite-blackboard 2>&1

echo "--- ビルド (Go) ---"
(cd "$SCRIPT_DIR/go" && go mod tidy && go build -o reader_go . 2>&1)

echo "--- Python 確認 ---"
python3 --version

# ── 初期化 ───────────────────────────────────────────────────
rm -f "$DB_PATH"
mkdir -p "$LOG_DIR"
rm -f "$LOG_DIR"/*.log

# Writer 起動
echo ""
echo "--- Writer 起動 (${TOTAL_DURATION}s) ---"
"$WRITER" --db "$DB_PATH" --channels 1 --duration "$TOTAL_DURATION" \
    > "$LOG_DIR/writer.log" 2>&1 &
WRITER_PID=$!
echo "  PID=$WRITER_PID, DB 初期化待ち..."
sleep 2

# ── ステージループ ───────────────────────────────────────────
for STAGE_COUNT in "${STAGES[@]}"; do
    PREV_RUST=${#RUST_PIDS[@]}
    PREV_GO=${#GO_PIDS[@]}
    PREV_PYTHON=${#PYTHON_PIDS[@]}
    ADDED=$(( STAGE_COUNT - PREV_RUST ))

    # Rust リーダーを追加起動
    while [[ ${#RUST_PIDS[@]} -lt $STAGE_COUNT ]]; do
        rid=$(( ${#RUST_PIDS[@]} + 1 ))
        "$READER_RUST" \
            --db "$DB_PATH" \
            --topic "sensor/0" \
            --reader-id "r${rid}" \
            --duration "$TOTAL_DURATION" \
            >> "$LOG_DIR/reader_r${rid}.log" 2>&1 &
        RUST_PIDS+=($!)
    done

    # Go リーダーを追加起動
    while [[ ${#GO_PIDS[@]} -lt $STAGE_COUNT ]]; do
        gid=$(( ${#GO_PIDS[@]} + 1 ))
        "$READER_GO" \
            --db "$DB_PATH" \
            --topic "sensor/0" \
            --reader-id "g${gid}" \
            --duration "$TOTAL_DURATION" \
            >> "$LOG_DIR/reader_g${gid}.log" 2>&1 &
        GO_PIDS+=($!)
    done

    # Python リーダーを追加起動
    while [[ ${#PYTHON_PIDS[@]} -lt $STAGE_COUNT ]]; do
        pid=$(( ${#PYTHON_PIDS[@]} + 1 ))
        python3 "$READER_PY" \
            --db "$DB_PATH" \
            --topic "sensor/0" \
            --reader-id "p${pid}" \
            --duration "$TOTAL_DURATION" \
            >> "$LOG_DIR/reader_p${pid}.log" 2>&1 &
        PYTHON_PIDS+=($!)
    done

    TOTAL=$(( STAGE_COUNT * 3 ))
    echo ""
    echo "┌──────────────────────────────────────────────────────────────┐"
    printf "│  ステージ: %2d readers/lang (計 %3d) +%d 追加  %ds 計測中...%*s│\n" \
        "$STAGE_COUNT" "$TOTAL" "$ADDED" "$STEP_DURATION" 0 ""
    echo "└──────────────────────────────────────────────────────────────┘"
    sleep "$STEP_DURATION"

    show_stats_poly "$STAGE_COUNT"
done

echo ""
echo "=== 多言語スケールテスト完了 ==="
echo "詳細ログ: $LOG_DIR/"
echo "  Rust  : $LOG_DIR/reader_r*.log"
echo "  Go    : $LOG_DIR/reader_g*.log"
echo "  Python: $LOG_DIR/reader_p*.log"
echo "  Writer: $LOG_DIR/writer.log"
