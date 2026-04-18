#!/usr/bin/env python3
"""SQLite ブラックボード PoC — Python Reader (30Hz 読み出し)

Rust Reader と同じクエリ・出力フォーマットで動作し、
Python 標準ライブラリ(sqlite3)のオーバーヘッドを計測する。

依存: Python 3.8 以上、外部ライブラリ不要
"""
import argparse
import collections
import logging
import sqlite3
import sys
import time

# bench_scale.sh の grep パターン "INFO.*回/s" に合わせたフォーマット
_LOG_FMT = "%(asctime)s.%(msecs)03d  INFO %(name)s: %(message)s"
logging.basicConfig(
    format=_LOG_FMT,
    datefmt="%Y-%m-%dT%H:%M:%S",
    stream=sys.stderr,
    level=logging.INFO,
)
logger = logging.getLogger("sqlite_blackboard::reader_py")


def _now_nanos():
    """UNIX エポックからのナノ秒を返す"""
    return time.time_ns()


def _open_readonly(db_path):
    """読み取り専用で SQLite 接続を開く"""
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.execute("PRAGMA journal_mode = WAL;")
    conn.execute("PRAGMA mmap_size = 268435456;")
    return conn


def _resolve_channel_id(conn, topic, timeout_s=30.0):
    """トピック名から最新の channel_id を解決する（最大 timeout_s 秒リトライ）"""
    deadline = time.monotonic() + timeout_s
    while True:
        row = conn.execute(
            "SELECT id FROM channels WHERE topic_name = ? ORDER BY id DESC LIMIT 1",
            (topic,),
        ).fetchone()
        if row:
            return row[0]
        if time.monotonic() >= deadline:
            raise RuntimeError(
                f"トピック '{topic}' が {timeout_s:.0f} 秒以内に見つかりませんでした"
            )
        logger.warning("[?] トピック '%s' が未登録。2秒後にリトライ...", topic)
        time.sleep(2.0)


def _print_stats(reader_id, window, msg_count, rolling):
    """1秒ウィンドウ + 1分ローリング統計をログ出力する

    window  : list of (q1_ns, q2_ns, stale_ns_or_None, q1_error: bool)
    rolling : deque of (monotonic, q1_ns, q2_ns)
    """
    n = len(window)
    if n == 0:
        return

    q1_vals = [s[0] for s in window]
    q2_vals = [s[1] for s in window]
    stale_vals = [s[2] for s in window if s[2] is not None]
    errors = sum(1 for s in window if s[3])

    q1_avg = sum(q1_vals) / n
    q1_max = max(q1_vals)
    q2_avg = sum(q2_vals) / n
    q2_max = max(q2_vals)
    max_stale_ms = max(stale_vals) // 1_000_000 if stale_vals else 0

    # 1分ローリング統計
    rolling_str = ""
    rn = len(rolling)
    if rn > 0:
        r1v = [r[1] for r in rolling]
        r2v = [r[2] for r in rolling]
        r1a = sum(r1v) / rn
        r1x = max(r1v)
        r2a = sum(r2v) / rn
        r2x = max(r2v)
        rolling_str = (
            f" | 1min({rn}):"
            f" latest avg={r1a/1e6:.3f}ms max={r1x/1e6:.3f}ms"
            f" count avg={r2a/1e6:.3f}ms max={r2x/1e6:.3f}ms"
        )

    logger.info(
        "[%s] %d回/s"
        " | latest: avg=%.3fms max=%.3fms"
        " | count(%d): avg=%.3fms max=%.3fms"
        " | stale_max=%dms errors=%d%s",
        reader_id, n,
        q1_avg / 1e6, q1_max / 1e6,
        msg_count,
        q2_avg / 1e6, q2_max / 1e6,
        max_stale_ms, errors,
        rolling_str,
    )


def run(db_path, channel_id, topic, reader_id, duration_s):
    """Reader メインループ

    channel_id : int または None（topic 指定時）
    topic      : str または None（channel_id 指定時）
    duration_s : float または None（無限ループ）
    """
    conn = _open_readonly(db_path)

    # channel_id の解決
    if topic is not None:
        logger.info("[%s] トピック '%s' の channel_id を解決中...", reader_id, topic)
        ch_id = _resolve_channel_id(conn, topic)
    else:
        ch_id = channel_id
    logger.info("[%s] Reader 起動: channel_id=%d", reader_id, ch_id)

    INTERVAL      = 1.0 / 30.0   # 約 33.3ms（30Hz）
    ROLLING_WIN   = 60.0          # ローリングウィンドウ幅（秒）
    RESOLVE_EVERY = 5.0           # channel_id 再解決間隔（秒）

    start_mono         = time.monotonic()
    next_tick          = time.monotonic() + INTERVAL
    last_report        = time.monotonic()
    last_resolve       = time.monotonic()

    # (q1_ns, q2_ns, stale_ns_or_None, q1_error)
    window  = []
    # (monotonic_time, q1_ns, q2_ns)
    rolling = collections.deque()
    last_count = 0

    while True:
        if duration_s is not None and (time.monotonic() - start_mono) >= duration_s:
            break

        # 次ティックまでスリープ
        rem = next_tick - time.monotonic()
        if rem > 0:
            time.sleep(rem)
        next_tick += INTERVAL

        # トピック指定の場合: 定期的に channel_id を再解決（Writer 再起動追従）
        if topic is not None and (time.monotonic() - last_resolve) >= RESOLVE_EVERY:
            try:
                new_id = _resolve_channel_id(conn, topic, timeout_s=1.0)
                if new_id != ch_id:
                    logger.info(
                        "[%s] channel_id 更新: %d → %d", reader_id, ch_id, new_id
                    )
                    ch_id = new_id
            except RuntimeError:
                pass
            last_resolve = time.monotonic()

        # ── クエリ1: latest_states から最新タイムスタンプを取得 ──────────
        t1 = time.monotonic_ns()
        try:
            row = conn.execute(
                "SELECT timestamp FROM latest_states WHERE channel_id = ?",
                (ch_id,),
            ).fetchone()
            q1_error = False
            stale_ns = max(0, _now_nanos() - row[0]) if row else None
        except sqlite3.OperationalError:
            q1_error = True
            stale_ns = None
        q1_ns = time.monotonic_ns() - t1

        # ── クエリ2: 直近 1 秒のメッセージ件数（集計クエリ） ─────────────
        cutoff_ns = _now_nanos() - 1_000_000_000
        t2 = time.monotonic_ns()
        try:
            row2 = conn.execute(
                "SELECT count(*) FROM messages WHERE log_time > ?",
                (cutoff_ns,),
            ).fetchone()
            last_count = row2[0] if row2 else 0
        except sqlite3.OperationalError:
            last_count = 0
        q2_ns = time.monotonic_ns() - t2

        now_mono = time.monotonic()
        window.append((q1_ns, q2_ns, stale_ns, q1_error))
        rolling.append((now_mono, q1_ns, q2_ns))

        # 1 秒ごとに統計を出力
        if (now_mono - last_report) >= 1.0:
            # 60 秒より古いエントリを刈り取る
            cutoff_mono = now_mono - ROLLING_WIN
            while rolling and rolling[0][0] < cutoff_mono:
                rolling.popleft()

            _print_stats(reader_id, window, last_count, rolling)
            window.clear()
            last_report = now_mono

    # 残余統計を出力
    if window:
        _print_stats(reader_id, window, last_count, rolling)

    conn.close()


def main():
    parser = argparse.ArgumentParser(
        description="SQLite ブラックボード PoC — Python Reader (30Hz 読み出し)"
    )
    parser.add_argument(
        "--db",
        default="/dev/shm/sqlite_poc/blackboard.db",
        help="DB ファイルパス (デフォルト: %(default)s)",
    )

    grp = parser.add_mutually_exclusive_group()
    grp.add_argument(
        "--channel-id",
        type=int,
        default=1,
        dest="channel_id",
        help="監視対象の channel_id。--topic と排他 (デフォルト: %(default)s)",
    )
    grp.add_argument(
        "--topic",
        help="監視対象のトピック名。--channel-id と排他。Writer 再起動後も自動追従",
    )

    parser.add_argument(
        "--reader-id",
        default="py0",
        dest="reader_id",
        help="ログ出力用の識別子 (デフォルト: %(default)s)",
    )
    parser.add_argument(
        "--duration",
        type=float,
        default=None,
        help="実行時間（秒）。省略時は無限ループ",
    )

    args = parser.parse_args()
    ch_id = None if args.topic else args.channel_id

    run(
        db_path=args.db,
        channel_id=ch_id,
        topic=args.topic,
        reader_id=args.reader_id,
        duration_s=args.duration,
    )


if __name__ == "__main__":
    main()
