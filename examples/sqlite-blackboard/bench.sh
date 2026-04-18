#!/usr/bin/env bash
# SQLite ブラックボード PoC ベンチマーク実行スクリプト
# 使用法: ./bench.sh [秒数(デフォルト30)]
set -euo pipefail

DURATION=${1:-30}
DB_PATH="/dev/shm/sqlite_poc/blackboard.db"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== SQLite ブラックボード PoC ==="
echo "  DB   : $DB_PATH"
echo "  時間 : ${DURATION}s"
echo ""

# ビルド
echo "--- ビルド ---"
cargo build --release -p sqlite-blackboard 2>&1

WRITER="$WORKSPACE_ROOT/target/release/writer"
READER="$WORKSPACE_ROOT/target/release/reader"

# 古いDBを削除
rm -f "$DB_PATH"

# Writer を起動（バックグラウンド）
echo "--- Writer 起動 ---"
"$WRITER" --db "$DB_PATH" --channels 1 --duration "$DURATION" &
WRITER_PID=$!

# Writer が DB を初期化するまで待機
sleep 1

# Reader のチャネル ID を確認（channels テーブルの最初の id）
CHANNEL_ID=1

# Reader を起動（バックグラウンド）
echo "--- Reader 起動 (channel_id=$CHANNEL_ID) ---"
"$READER" --db "$DB_PATH" --channel-id "$CHANNEL_ID" --duration "$DURATION" &
READER_PID=$!

# 完了待ち
wait "$WRITER_PID" && echo "Writer 正常終了"
wait "$READER_PID" && echo "Reader 正常終了"

# DB サイズ確認
echo ""
echo "--- DB ファイルサイズ ---"
ls -lh "$DB_PATH" 2>/dev/null || echo "(DBファイルなし)"

# メモリ使用量 (/dev/shm)
echo ""
echo "--- /dev/shm 使用量 ---"
df -h /dev/shm

echo ""
echo "=== 完了 ==="
