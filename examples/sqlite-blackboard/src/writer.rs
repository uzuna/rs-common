use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db;

/// Writer の動作パラメータ
pub struct WriterConfig {
    /// 生成するチャネル数
    pub channel_count: usize,
    /// 1 メッセージのデータサイズ (bytes)
    pub data_size: usize,
    /// 書き込み周波数 (Hz)
    pub hz: u32,
    /// コミット間隔 (ms)
    pub commit_ms: u64,
    /// メッセージ保持期間 (秒)。これより古いレコードを定期的に削除する
    pub retention_secs: u64,
    /// クリーンアップ実行間隔 (秒)
    pub cleanup_interval_secs: u64,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            channel_count: 1,
            data_size: 4096,
            hz: 1000,
            commit_ms: 100,
            retention_secs: 300,
            cleanup_interval_secs: 30,
        }
    }
}

struct Message {
    channel_id: i64,
    log_time: i64,
    data: Vec<u8>,
}

/// コミット 1 回分の計測値
struct CommitStat {
    duration_nanos: u64,
    batch_size: usize,
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

fn generate_guid() -> String {
    let pid = std::process::id();
    let ts = now_nanos();
    format!("{ts:032x}{pid:08x}")
}

fn register_participant(conn: &Connection, guid: &str, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO participants (guid, name) VALUES (?1, ?2)",
        params![guid, name],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM participants WHERE guid = ?1",
        params![guid],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn register_channel(conn: &Connection, topic: &str, participant_id: i64) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO channels (topic_name, participant_id) VALUES (?1, ?2)",
        params![topic, participant_id],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM channels WHERE topic_name = ?1 AND participant_id = ?2",
        params![topic, participant_id],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn commit_batch(conn: &Connection, messages: &[Message]) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    {
        let mut stmt_msg = conn.prepare(
            "INSERT OR REPLACE INTO messages (log_time, channel_id, data) VALUES (?1, ?2, ?3)",
        )?;
        let mut stmt_latest = conn.prepare(
            "INSERT OR REPLACE INTO latest_states (channel_id, timestamp, data_json) \
             VALUES (?1, ?2, ?3)",
        )?;
        for msg in messages {
            stmt_msg.execute(params![msg.log_time, msg.channel_id, msg.data])?;
            stmt_latest.execute(params![msg.channel_id, msg.log_time, "{}"])?;
        }
    }
    conn.execute_batch("COMMIT;")?;
    Ok(())
}

fn cleanup(conn: &Connection, retention_nanos: i64) -> Result<usize> {
    let cutoff = now_nanos() - retention_nanos;
    let deleted = conn.execute("DELETE FROM messages WHERE log_time < ?1", params![cutoff])?;
    conn.execute_batch("PRAGMA incremental_vacuum(10);")?;
    Ok(deleted)
}

/// WAL ファイルサイズを KB 単位で返す（存在しない場合は 0）
fn wal_size_kb(db_path: &Path) -> u64 {
    // SQLite WAL は `<db>-wal` という名前になる
    let mut wal = db_path.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(wal).map(|m| m.len() / 1024).unwrap_or(0)
}

/// Writer を実行する
pub fn run(path: &Path, config: &WriterConfig, duration: Option<Duration>) -> Result<()> {
    let conn = db::open_writer(path)?;

    let guid = generate_guid();
    let participant_id = register_participant(&conn, &guid, "rust-writer")?;
    let mut channel_ids = Vec::new();
    for i in 0..config.channel_count {
        let topic = format!("sensor/{i}");
        let ch_id = register_channel(&conn, &topic, participant_id)?;
        channel_ids.push(ch_id);
    }

    let retention_nanos = config.retention_secs as i64 * 1_000_000_000i64;
    let cleanup_interval = Duration::from_secs(config.cleanup_interval_secs);

    tracing::info!(
        "Writer 起動: channels={} hz={} data_size={}B commit_ms={}ms retention={}s cleanup_every={}s",
        config.channel_count,
        config.hz,
        config.data_size,
        config.commit_ms,
        config.retention_secs,
        config.cleanup_interval_secs,
    );

    let buffer: Arc<Mutex<Vec<Message>>> = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = Arc::clone(&buffer);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let data_size = config.data_size;
    let hz = config.hz;

    // データ生成スレッド（hz 周期）
    let write_thread = std::thread::spawn(move || {
        let interval = Duration::from_nanos(1_000_000_000 / hz as u64);
        let mut next = Instant::now() + interval;
        while !stop_clone.load(Ordering::Relaxed) {
            let now = Instant::now();
            if let Some(rem) = next.checked_duration_since(now) {
                std::thread::sleep(rem);
            }
            let ts = now_nanos();
            {
                let mut buf = buf_clone.lock().unwrap();
                for &ch_id in &channel_ids {
                    buf.push(Message {
                        channel_id: ch_id,
                        log_time: ts,
                        data: vec![0u8; data_size],
                    });
                }
            }
            next += interval;
        }
    });

    // コミットループ（メインスレッド）
    let start = Instant::now();
    let commit_interval = Duration::from_millis(config.commit_ms);
    let mut next_commit = Instant::now() + commit_interval;
    let mut next_cleanup = Instant::now() + cleanup_interval;
    let mut inserted_total = 0u64;

    // 1 秒ウィンドウ統計
    let mut sec_commits: Vec<CommitStat> = Vec::new();
    let mut sec_inserted: u64 = 0;
    let mut last_report = Instant::now();

    loop {
        if let Some(d) = duration {
            if start.elapsed() >= d {
                break;
            }
        }

        let now = Instant::now();
        if let Some(rem) = next_commit.checked_duration_since(now) {
            std::thread::sleep(rem);
        }
        next_commit += commit_interval;

        let messages: Vec<Message> = {
            let mut buf = buffer.lock().unwrap();
            std::mem::take(&mut *buf)
        };

        if !messages.is_empty() {
            let batch_size = messages.len();
            let t = Instant::now();
            commit_batch(&conn, &messages)?;
            let d_nanos = t.elapsed().as_nanos() as u64;

            inserted_total += batch_size as u64;
            sec_inserted += batch_size as u64;
            sec_commits.push(CommitStat {
                duration_nanos: d_nanos,
                batch_size,
            });
        }

        // クリーンアップ（cleanup_interval ごと）
        if Instant::now() >= next_cleanup {
            let deleted = cleanup(&conn, retention_nanos)?;
            let wal_kb = wal_size_kb(path);
            tracing::info!(
                "[cleanup] deleted={deleted} retention={}s wal_kb={wal_kb}",
                config.retention_secs
            );
            next_cleanup += cleanup_interval;
        }

        // 1 秒ごとにスループット・コミット性能・WAL サイズを出力
        if last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed_s = last_report.elapsed().as_secs_f64();
            let tps = (sec_inserted as f64 / elapsed_s) as u64;
            let target_tps = hz as u64 * config.channel_count as u64;
            // 目標 Hz に対する実効達成率 (%)
            let achieve_pct = if target_tps > 0 {
                tps * 100 / target_tps
            } else {
                0
            };
            // 理論バッチサイズ = Hz × commit_ms / 1000
            let theory_batch = hz as u64 * config.commit_ms / 1000;

            let (c_avg_ns, c_max_ns, b_avg, b_max) = if sec_commits.is_empty() {
                (0u64, 0u64, 0usize, 0usize)
            } else {
                let n = sec_commits.len() as u64;
                let c_avg = sec_commits.iter().map(|s| s.duration_nanos).sum::<u64>() / n;
                let c_max = sec_commits
                    .iter()
                    .map(|s| s.duration_nanos)
                    .max()
                    .unwrap_or(0);
                let b_sum: usize = sec_commits.iter().map(|s| s.batch_size).sum();
                let b_avg = b_sum / sec_commits.len();
                let b_max = sec_commits.iter().map(|s| s.batch_size).max().unwrap_or(0);
                (c_avg, c_max, b_avg, b_max)
            };
            let wal_kb = wal_size_kb(path);

            tracing::info!(
                "[writer] tps={tps} ({achieve_pct}%/{target_tps}) | \
                 commit: avg={:.3}ms max={:.3}ms | \
                 batch: avg={b_avg} max={b_max} theory={theory_batch} | \
                 wal_kb={wal_kb} total={inserted_total}",
                c_avg_ns as f64 / 1e6,
                c_max_ns as f64 / 1e6,
            );

            sec_commits.clear();
            sec_inserted = 0;
            last_report = Instant::now();
        }
    }

    stop.store(true, Ordering::Relaxed);
    write_thread.join().unwrap();
    tracing::info!("--- Writer 完了: 総挿入 {inserted_total} 件 ---");
    Ok(())
}
