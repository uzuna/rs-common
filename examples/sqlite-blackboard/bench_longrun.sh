#!/usr/bin/env bash
# 長時間負荷テスト: Rust / Go / Python 各 40 プロセス、最大 24 時間
#
# 使用法: ./bench_longrun.sh [計測秒数(デフォルト 86400)] [リーダー数/言語(デフォルト 40)] [サマリー間隔秒(デフォルト 300)]
#
# 出力先: /tmp/sqlite_blackboard_longrun_<開始日時>/
#   reader_r{1..N}.log  : Rust リーダーログ
#   reader_g{1..N}.log  : Go リーダーログ
#   reader_p{1..N}.log  : Python リーダーログ
#   writer.log          : Writer ログ
#   summary.log         : 定期サマリー (CSV ライク)
#   final_report.txt    : 最終レポート
set -euo pipefail

DURATION=${1:-86400}
READERS_PER_LANG=${2:-40}
SUMMARY_INTERVAL=${3:-300}

DB_PATH="/dev/shm/sqlite_poc/blackboard_longrun.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

START_TS=$(date '+%Y%m%d_%H%M%S')
OUTPUT_DIR="/tmp/sqlite_blackboard_longrun_${START_TS}"

WRITER="$WORKSPACE_ROOT/target/release/writer"
READER_RUST="$WORKSPACE_ROOT/target/release/reader"
READER_GO="$SCRIPT_DIR/go/reader_go"
READER_PY="$SCRIPT_DIR/reader.py"

WRITER_PID=""
ALL_PIDS=()

# ────────────────────────────────────────────────────────────
# ユーティリティ
# ────────────────────────────────────────────────────────────
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

# 指定ログファイルの最終統計行を解析して項目を出力する
# 出力: l_avg l_max c_avg c_max stale errs  (いずれかが空なら "N/A")
parse_last_stat() {
    local log="$1"
    [[ -f "$log" ]] || { echo "N/A N/A N/A N/A 0 0"; return; }
    local last
    last=$(grep -a 'INFO.*回/s' "$log" 2>/dev/null | tail -1 || true)
    if [[ -z "$last" ]]; then
        echo "N/A N/A N/A N/A 0 0"
        return
    fi
    local l_avg l_max c_avg c_max stale errs
    l_avg=$(echo "$last" | grep -oP 'latest: avg=\K[0-9.]+' || echo "N/A")
    l_max=$(echo "$last" | grep -oP 'latest:.*?max=\K[0-9.]+' || echo "N/A")
    c_avg=$(echo "$last" | grep -oP 'count\([0-9]+\): avg=\K[0-9.]+' || echo "N/A")
    c_max=$(echo "$last" | grep -oP 'count\([0-9]+\):.*?max=\K[0-9.]+' || echo "N/A")
    stale=$(echo "$last" | grep -oP 'stale_max=\K[0-9]+' || echo "0")
    errs=$(echo "$last" | grep -oP 'errors=\K[0-9]+' || echo "0")
    echo "$l_avg $l_max $c_avg $c_max $stale $errs"
}

# 言語グループ全体を集計して 1 行を summary.log に追記する
# 引数: $1=タイムスタンプ $2=経過秒 $3=言語名 $4=プレフィックス
summarize_lang() {
    local ts="$1" elapsed="$2" lang="$3" prefix="$4"
    local n=$READERS_PER_LANG

    local sum_la=0 sum_ca=0 lm_all=0 cm_all=0 stale_all=0 errs_all=0 valid=0

    for ((i=1; i<=n; i++)); do
        local log="$OUTPUT_DIR/reader_${prefix}${i}.log"
        read -r la lm ca cm st er < <(parse_last_stat "$log")
        [[ "$la" == "N/A" ]] && continue
        sum_la=$(echo "$sum_la + $la" | bc)
        sum_ca=$(echo "$sum_ca + $ca" | bc)
        valid=$(( valid + 1 ))
        local cmp
        cmp=$(echo "$lm > $lm_all" | bc)
        [[ "$cmp" -eq 1 ]] && lm_all=$lm
        cmp=$(echo "$cm > $cm_all" | bc)
        [[ "$cmp" -eq 1 ]] && cm_all=$cm
        [[ "$st" -gt "$stale_all" ]] && stale_all=$st
        errs_all=$(( errs_all + er ))
    done

    if [[ $valid -eq 0 ]]; then
        echo "$ts,$elapsed,$lang,$n,N/A,N/A,N/A,N/A,0,0,0" >> "$OUTPUT_DIR/summary.log"
        return
    fi

    local avg_la avg_ca
    avg_la=$(echo "scale=3; $sum_la / $valid" | bc)
    avg_ca=$(echo "scale=3; $sum_ca / $valid" | bc)

    # CSV: timestamp, elapsed_sec, lang, readers, latest_avg_ms, latest_max_ms,
    #       count_avg_ms, count_max_ms, stale_max_ms, errors, valid_readers
    echo "$ts,$elapsed,$lang,$n,$avg_la,$lm_all,$avg_ca,$cm_all,$stale_all,$errs_all,$valid" \
        >> "$OUTPUT_DIR/summary.log"
}

# 画面とサマリーログに現在の統計を出力する
snapshot() {
    local elapsed=$(( $(date +%s) - START_EPOCH ))
    local ts
    ts=$(date '+%Y-%m-%d %H:%M:%S')

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    printf "  スナップショット  %s  (経過 %ds / %ds)\n" "$ts" "$elapsed" "$DURATION"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    printf "  %-8s  %3s  %-18s  %-18s  %-12s  %s\n" \
        "言語" "N" "latest avg/max(ms)" "count avg/max(ms)" "stale_max" "errors"
    printf "  %s\n" "--------  ---  ------------------  ------------------  ------------  ------"

    for entry in "Rust r" "Go g" "Python p"; do
        local lang=${entry%% *}
        local prefix=${entry##* }
        local n=$READERS_PER_LANG
        local sum_la=0 sum_ca=0 lm_all=0 cm_all=0 stale_all=0 errs_all=0 valid=0

        for ((i=1; i<=n; i++)); do
            local logf="$OUTPUT_DIR/reader_${prefix}${i}.log"
            read -r la lm ca cm st er < <(parse_last_stat "$logf")
            [[ "$la" == "N/A" ]] && continue
            sum_la=$(echo "$sum_la + $la" | bc)
            sum_ca=$(echo "$sum_ca + $ca" | bc)
            valid=$(( valid + 1 ))
            local cmp
            cmp=$(echo "$lm > $lm_all" | bc)
            [[ "$cmp" -eq 1 ]] && lm_all=$lm
            cmp=$(echo "$cm > $cm_all" | bc)
            [[ "$cmp" -eq 1 ]] && cm_all=$cm
            [[ "$st" -gt "$stale_all" ]] && stale_all=$st
            errs_all=$(( errs_all + er ))
        done

        if [[ $valid -eq 0 ]]; then
            printf "  %-8s  %3d  %s\n" "$lang" "$n" "(統計データなし)"
        else
            local avg_la avg_ca
            avg_la=$(echo "scale=3; $sum_la / $valid" | bc)
            avg_ca=$(echo "scale=3; $sum_ca / $valid" | bc)
            printf "  %-8s  %3d  %8s / %-8s  %8s / %-8s  %-12s  %s\n" \
                "$lang" "$valid" \
                "$avg_la" "$lm_all" \
                "$avg_ca" "$cm_all" \
                "${stale_all}ms" \
                "$errs_all"
        fi

        summarize_lang "$ts" "$elapsed" "$lang" "$prefix"
    done
}

# ────────────────────────────────────────────────────────────
# クリーンアップ
# ────────────────────────────────────────────────────────────
cleanup() {
    echo ""
    log "クリーンアップ中..."
    [[ -n "$WRITER_PID" ]] && kill "$WRITER_PID" 2>/dev/null || true
    for pid in "${ALL_PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true

    # 最終レポート作成
    {
        echo "=== 長時間負荷テスト 最終レポート ==="
        echo "開始: $START_TS"
        echo "終了: $(date '+%Y-%m-%d %H:%M:%S')"
        echo "計測時間: ${DURATION}s"
        echo "リーダー数/言語: ${READERS_PER_LANG}"
        echo "DB: $DB_PATH"
        echo ""
        echo "--- 最終スナップショット ---"

        for entry in "Rust r" "Go g" "Python p"; do
            local lang=${entry%% *}
            local prefix=${entry##* }
            local n=$READERS_PER_LANG
            local sum_la=0 sum_ca=0 lm_all=0 cm_all=0 stale_all=0 errs_all=0 valid=0

            for ((i=1; i<=n; i++)); do
                local logf="$OUTPUT_DIR/reader_${prefix}${i}.log"
                read -r la lm ca cm st er < <(parse_last_stat "$logf")
                [[ "$la" == "N/A" ]] && continue
                sum_la=$(echo "$sum_la + $la" | bc)
                sum_ca=$(echo "$sum_ca + $ca" | bc)
                valid=$(( valid + 1 ))
                local cmp
                cmp=$(echo "$lm > $lm_all" | bc)
                [[ "$cmp" -eq 1 ]] && lm_all=$lm
                cmp=$(echo "$cm > $cm_all" | bc)
                [[ "$cmp" -eq 1 ]] && cm_all=$cm
                [[ "$st" -gt "$stale_all" ]] && stale_all=$st
                errs_all=$(( errs_all + er ))
            done

            if [[ $valid -gt 0 ]]; then
                local avg_la avg_ca
                avg_la=$(echo "scale=3; $sum_la / $valid" | bc)
                avg_ca=$(echo "scale=3; $sum_ca / $valid" | bc)
                printf "  %-8s  readers=%d  latest_avg=%sms  latest_max=%sms  count_avg=%sms  count_max=%sms  stale_max=%dms  errors=%d\n" \
                    "$lang" "$valid" "$avg_la" "$lm_all" "$avg_ca" "$cm_all" "$stale_all" "$errs_all"
            fi
        done

        echo ""
        echo "--- プロセス別エラー集計 ---"
        for entry in "Rust r" "Go g" "Python p"; do
            local lang=${entry%% *}
            local prefix=${entry##* }
            echo "  ${lang}:"
            for ((i=1; i<=READERS_PER_LANG; i++)); do
                local logf="$OUTPUT_DIR/reader_${prefix}${i}.log"
                [[ -f "$logf" ]] || continue
                local errs
                errs=$(grep -a 'errors=' "$logf" 2>/dev/null | tail -1 | grep -oP 'errors=\K[0-9]+' || echo "0")
                [[ "$errs" -gt 0 ]] && echo "    reader_${prefix}${i}: errors=$errs"
            done
        done

        echo ""
        echo "--- 出力ファイル ---"
        ls -lh "$OUTPUT_DIR/"
    } > "$OUTPUT_DIR/final_report.txt"

    log "最終レポート: $OUTPUT_DIR/final_report.txt"
    cat "$OUTPUT_DIR/final_report.txt"
}
trap cleanup EXIT

# ────────────────────────────────────────────────────────────
# メイン処理
# ────────────────────────────────────────────────────────────
echo "=== 長時間負荷テスト: Rust / Go / Python 各 ${READERS_PER_LANG} プロセス ==="
echo "計測時間: ${DURATION}s ($(( DURATION / 3600 ))h$(( (DURATION % 3600) / 60 ))m)"
echo "サマリー出力間隔: ${SUMMARY_INTERVAL}s"
echo "出力ディレクトリ: ${OUTPUT_DIR}"
echo "DB: ${DB_PATH}"
echo ""

# ── ビルド ───────────────────────────────────────────────────
log "--- ビルド (Rust) ---"
cargo build --release -p sqlite-blackboard 2>&1

log "--- ビルド (Go) ---"
(cd "$SCRIPT_DIR/go" && go mod tidy && go build -o reader_go . 2>&1)

log "--- Python 確認 ---"
python3 --version

# ── 初期化 ───────────────────────────────────────────────────
rm -f "$DB_PATH" "${DB_PATH}-wal" "${DB_PATH}-shm"
mkdir -p "$OUTPUT_DIR"

# summary.log ヘッダー
echo "timestamp,elapsed_sec,lang,readers,latest_avg_ms,latest_max_ms,count_avg_ms,count_max_ms,stale_max_ms,errors,valid_readers" \
    > "$OUTPUT_DIR/summary.log"

# ── Writer 起動 ──────────────────────────────────────────────
log "Writer 起動 (duration=${DURATION}s)..."
"$WRITER" \
    --db "$DB_PATH" \
    --channels 1 \
    --data-size 256 \
    --hz 100 \
    --commit-ms 50 \
    --retention-secs 300 \
    --cleanup-interval-secs 30 \
    --duration "$DURATION" \
    > "$OUTPUT_DIR/writer.log" 2>&1 &
WRITER_PID=$!
log "Writer PID=$WRITER_PID"

# DB 初期化完了を待つ（最大 30 秒）
log "DB 初期化待ち..."
for ((i=1; i<=30; i++)); do
    [[ -f "$DB_PATH" ]] && break
    sleep 1
done
[[ -f "$DB_PATH" ]] || { log "ERROR: DB が作成されませんでした"; exit 1; }
log "DB 確認: $DB_PATH"
sleep 2  # WAL チャンネル登録を待つ

# ── Reader 起動 ──────────────────────────────────────────────
log "Rust リーダー ${READERS_PER_LANG} プロセス起動中..."
for ((i=1; i<=READERS_PER_LANG; i++)); do
    "$READER_RUST" \
        --db "$DB_PATH" \
        --topic "sensor/0" \
        --reader-id "r${i}" \
        --duration "$DURATION" \
        >> "$OUTPUT_DIR/reader_r${i}.log" 2>&1 &
    ALL_PIDS+=($!)
done
log "  → ${READERS_PER_LANG} プロセス起動完了"

log "Go リーダー ${READERS_PER_LANG} プロセス起動中..."
for ((i=1; i<=READERS_PER_LANG; i++)); do
    "$READER_GO" \
        --db "$DB_PATH" \
        --topic "sensor/0" \
        --reader-id "g${i}" \
        --duration "$DURATION" \
        >> "$OUTPUT_DIR/reader_g${i}.log" 2>&1 &
    ALL_PIDS+=($!)
done
log "  → ${READERS_PER_LANG} プロセス起動完了"

log "Python リーダー ${READERS_PER_LANG} プロセス起動中..."
for ((i=1; i<=READERS_PER_LANG; i++)); do
    python3 "$READER_PY" \
        --db "$DB_PATH" \
        --topic "sensor/0" \
        --reader-id "p${i}" \
        --duration "$DURATION" \
        >> "$OUTPUT_DIR/reader_p${i}.log" 2>&1 &
    ALL_PIDS+=($!)
done
log "  → ${READERS_PER_LANG} プロセス起動完了"

TOTAL_READERS=$(( READERS_PER_LANG * 3 ))
log "全 ${TOTAL_READERS} リーダー起動完了 (Writer + ${TOTAL_READERS} readers)"
log "最初のスナップショットまで ${SUMMARY_INTERVAL}s 待機..."

# ── 定期サマリーループ ───────────────────────────────────────
START_EPOCH=$(date +%s)
END_EPOCH=$(( START_EPOCH + DURATION ))
NEXT_SNAP=$(( START_EPOCH + SUMMARY_INTERVAL ))

while true; do
    NOW=$(date +%s)
    [[ $NOW -ge $END_EPOCH ]] && break

    # 次のスナップショットまで待機（1 秒刻みで終了時刻チェック）
    while true; do
        NOW=$(date +%s)
        [[ $NOW -ge $END_EPOCH ]] && break 2
        [[ $NOW -ge $NEXT_SNAP ]] && break
        sleep 1
    done

    # Writer の生存確認
    if ! kill -0 "$WRITER_PID" 2>/dev/null; then
        log "WARNING: Writer (PID=$WRITER_PID) が終了しています"
    fi

    # 生存リーダー数確認
    alive=0
    for pid in "${ALL_PIDS[@]}"; do
        kill -0 "$pid" 2>/dev/null && alive=$(( alive + 1 )) || true
    done
    log "生存プロセス: Writer=$(kill -0 "$WRITER_PID" 2>/dev/null && echo OK || echo DEAD)  Readers=${alive}/${TOTAL_READERS}"

    snapshot

    NEXT_SNAP=$(( NEXT_SNAP + SUMMARY_INTERVAL ))
done

log "計測時間 ${DURATION}s 経過。終了処理中..."
