// SQLite ブラックボード PoC — Go Reader (30Hz 読み出し)
//
// Rust Reader / Python Reader と同じクエリ・出力フォーマットで動作し、
// CGO 経由の go-sqlite3 ライブラリのオーバーヘッドを計測する。
//
// ビルド: cd go && go mod tidy && go build -o reader_go .
package main

import (
	"database/sql"
	"flag"
	"fmt"
	"os"
	"time"

	_ "github.com/mattn/go-sqlite3"
)

// ────────────────────────────────────────────────────────────
// ログ出力（tracing と同じフォーマット: bench_scale.sh が grep できるよう合わせる）
// ────────────────────────────────────────────────────────────

func logInfo(format string, args ...any) {
	msg := fmt.Sprintf(format, args...)
	fmt.Fprintf(os.Stderr, "%s  INFO sqlite_blackboard::reader_go: %s\n",
		time.Now().UTC().Format("2006-01-02T15:04:05.000000Z"), msg)
}

// ────────────────────────────────────────────────────────────
// DB 接続
// ────────────────────────────────────────────────────────────

func openReadOnly(dbPath string) (*sql.DB, error) {
	// URI 形式で読み取り専用接続 + WAL / mmap
	dsn := fmt.Sprintf(
		"file:%s?mode=ro&_journal_mode=WAL&_mmap_size=268435456",
		dbPath,
	)
	db, err := sql.Open("sqlite3", dsn)
	if err != nil {
		return nil, err
	}
	// コネクションプール無効（シングル接続）
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	return db, nil
}

// ────────────────────────────────────────────────────────────
// channel_id の解決
// ────────────────────────────────────────────────────────────

func resolveChannelID(db *sql.DB, topic string, timeoutS float64) (int64, error) {
	deadline := time.Now().Add(time.Duration(timeoutS * float64(time.Second)))
	for {
		var id int64
		err := db.QueryRow(
			"SELECT id FROM channels WHERE topic_name = ? ORDER BY id DESC LIMIT 1",
			topic,
		).Scan(&id)
		if err == nil {
			return id, nil
		}
		if time.Now().After(deadline) {
			return 0, fmt.Errorf("トピック '%s' が %.0f 秒以内に見つかりませんでした", topic, timeoutS)
		}
		logInfo("トピック '%s' が未登録。2秒後にリトライ...", topic)
		time.Sleep(2 * time.Second)
	}
}

// ────────────────────────────────────────────────────────────
// 統計
// ────────────────────────────────────────────────────────────

type sample struct {
	q1Nanos int64
	q2Nanos int64
	staleNs int64 // -1 = データなし
	q1Error bool
}

type rollEntry struct {
	mono    time.Time
	q1Nanos int64
	q2Nanos int64
}

func printStats(readerID string, window []sample, msgCount int64, rolling []rollEntry) {
	n := len(window)
	if n == 0 {
		return
	}

	var q1Sum, q2Sum, staleMax int64
	q1Max, q2Max := int64(0), int64(0)
	errors := 0

	for _, s := range window {
		q1Sum += s.q1Nanos
		q2Sum += s.q2Nanos
		if s.q1Nanos > q1Max {
			q1Max = s.q1Nanos
		}
		if s.q2Nanos > q2Max {
			q2Max = s.q2Nanos
		}
		if s.staleNs >= 0 && s.staleNs > staleMax {
			staleMax = s.staleNs
		}
		if s.q1Error {
			errors++
		}
	}
	q1Avg := q1Sum / int64(n)
	q2Avg := q2Sum / int64(n)

	rollingStr := ""
	if rn := len(rolling); rn > 0 {
		var r1Sum, r2Sum int64
		r1Max, r2Max := int64(0), int64(0)
		for _, r := range rolling {
			r1Sum += r.q1Nanos
			r2Sum += r.q2Nanos
			if r.q1Nanos > r1Max {
				r1Max = r.q1Nanos
			}
			if r.q2Nanos > r2Max {
				r2Max = r.q2Nanos
			}
		}
		rollingStr = fmt.Sprintf(
			" | 1min(%d): latest avg=%.3fms max=%.3fms count avg=%.3fms max=%.3fms",
			rn,
			float64(r1Sum/int64(rn))/1e6, float64(r1Max)/1e6,
			float64(r2Sum/int64(rn))/1e6, float64(r2Max)/1e6,
		)
	}

	logInfo(
		"[%s] %d回/s | latest: avg=%.3fms max=%.3fms | count(%d): avg=%.3fms max=%.3fms | stale_max=%dms errors=%d%s",
		readerID, n,
		float64(q1Avg)/1e6, float64(q1Max)/1e6,
		msgCount,
		float64(q2Avg)/1e6, float64(q2Max)/1e6,
		staleMax/1_000_000, errors,
		rollingStr,
	)
}

// ────────────────────────────────────────────────────────────
// メインループ
// ────────────────────────────────────────────────────────────

const (
	intervalNs    = 33_333_333 * time.Nanosecond // 約 30Hz
	rollingWindow = 60 * time.Second
	resolveEvery  = 5 * time.Second
)

func run(dbPath string, channelID int64, topic string, readerID string, duration time.Duration) error {
	db, err := openReadOnly(dbPath)
	if err != nil {
		return fmt.Errorf("DB 接続失敗: %w", err)
	}
	defer db.Close()

	chID := channelID
	if topic != "" {
		logInfo("[%s] トピック '%s' の channel_id を解決中...", readerID, topic)
		chID, err = resolveChannelID(db, topic, 30.0)
		if err != nil {
			return err
		}
	}
	logInfo("[%s] Reader 起動: channel_id=%d", readerID, chID)

	startedAt := time.Now()
	nextTick := time.Now().Add(intervalNs)
	lastReport := time.Now()
	lastResolve := time.Now()

	var window []sample
	var rolling []rollEntry
	var lastCount int64

	for {
		if duration > 0 && time.Since(startedAt) >= duration {
			break
		}

		// 次ティックまでスリープ
		if rem := time.Until(nextTick); rem > 0 {
			time.Sleep(rem)
		}
		nextTick = nextTick.Add(intervalNs)

		// トピック指定の場合: 定期的に channel_id を再解決（Writer 再起動追従）
		if topic != "" && time.Since(lastResolve) >= resolveEvery {
			if newID, e := resolveChannelID(db, topic, 1.0); e == nil && newID != chID {
				logInfo("[%s] channel_id 更新: %d → %d", readerID, chID, newID)
				chID = newID
			}
			lastResolve = time.Now()
		}

		// ── クエリ1: latest_states から最新タイムスタンプを取得 ──────────
		t1 := time.Now()
		var ts int64
		q1Err := db.QueryRow(
			"SELECT timestamp FROM latest_states WHERE channel_id = ?", chID,
		).Scan(&ts)
		q1Nanos := time.Since(t1).Nanoseconds()

		staleNs := int64(-1)
		q1Error := q1Err != nil
		if q1Err == nil {
			if s := time.Now().UnixNano() - ts; s > 0 {
				staleNs = s
			} else {
				staleNs = 0
			}
		}

		// ── クエリ2: 直近 1 秒のメッセージ件数（集計クエリ） ─────────────
		cutoffNs := time.Now().UnixNano() - 1_000_000_000
		t2 := time.Now()
		var cnt int64
		if e := db.QueryRow(
			"SELECT count(*) FROM messages WHERE log_time > ?", cutoffNs,
		).Scan(&cnt); e == nil {
			lastCount = cnt
		}
		q2Nanos := time.Since(t2).Nanoseconds()

		nowMono := time.Now()
		window = append(window, sample{q1Nanos, q2Nanos, staleNs, q1Error})
		rolling = append(rolling, rollEntry{nowMono, q1Nanos, q2Nanos})

		// 1 秒ごとに統計を出力
		if time.Since(lastReport) >= time.Second {
			// 60 秒より古いエントリを刈り取る
			cutoff := nowMono.Add(-rollingWindow)
			for len(rolling) > 0 && rolling[0].mono.Before(cutoff) {
				rolling = rolling[1:]
			}
			printStats(readerID, window, lastCount, rolling)
			window = window[:0]
			lastReport = time.Now()
		}
	}

	if len(window) > 0 {
		printStats(readerID, window, lastCount, rolling)
	}
	return nil
}

// ────────────────────────────────────────────────────────────
// CLI エントリポイント
// ────────────────────────────────────────────────────────────

func main() {
	dbFlag := flag.String("db", "/dev/shm/sqlite_poc/blackboard.db", "DB ファイルパス")
	channelIDFlag := flag.Int64("channel-id", 1, "監視対象の channel_id (--topic と排他)")
	topicFlag := flag.String("topic", "", "監視対象のトピック名 (--channel-id と排他。Writer 再起動後も自動追従)")
	readerIDFlag := flag.String("reader-id", "go0", "ログ出力用の識別子")
	durationFlag := flag.Float64("duration", 0, "実行時間（秒）。0 または省略時は無限ループ")
	flag.Parse()

	topic := *topicFlag
	chID := int64(0)
	if topic == "" {
		chID = *channelIDFlag
	}

	dur := time.Duration(0)
	if *durationFlag > 0 {
		dur = time.Duration(*durationFlag * float64(time.Second))
	}

	if err := run(*dbFlag, chID, topic, *readerIDFlag, dur); err != nil {
		fmt.Fprintf(os.Stderr, "エラー: %v\n", err)
		os.Exit(1)
	}
}
