use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db;

struct Message {
    channel_id: i64,
    log_time: i64,
    data: Vec<u8>,
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

/// プロセスIDと起動時刻からGUIDを生成する
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

/// BEGIN IMMEDIATE でバッチ挿入し COMMIT
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

/// 5秒以上経過したメッセージを削除し incremental_vacuum を実行
fn cleanup(conn: &Connection) -> Result<()> {
    let cutoff = now_nanos() - 5_000_000_000i64;
    conn.execute("DELETE FROM messages WHERE log_time < ?1", params![cutoff])?;
    conn.execute_batch("PRAGMA incremental_vacuum(10);")?;
    Ok(())
}

/// Writer を実行する
///
/// - `channel_count`: 生成するチャネル数
/// - `duration`:      実行時間（`None` の場合は無限）
pub fn run(path: &Path, channel_count: usize, duration: Option<Duration>) -> Result<()> {
    let conn = db::open_writer(path)?;

    // 参加者とチャネルの登録
    let guid = generate_guid();
    let participant_id = register_participant(&conn, &guid, "rust-writer")?;
    let mut channel_ids = Vec::new();
    for i in 0..channel_count {
        let topic = format!("sensor/{i}");
        let ch_id = register_channel(&conn, &topic, participant_id)?;
        channel_ids.push(ch_id);
    }
    tracing::info!("参加者 id={participant_id}, チャネル数={channel_count}");

    let buffer: Arc<Mutex<Vec<Message>>> = Arc::new(Mutex::new(Vec::new()));
    let buf_clone = Arc::clone(&buffer);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    // 1000Hz データ生成スレッド
    let write_thread = std::thread::spawn(move || {
        let interval = Duration::from_millis(1);
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
                        data: vec![0u8; 4096],
                    });
                }
            }
            next += interval;
        }
    });

    // 100ms コミットループ（メインスレッド）
    let start = Instant::now();
    let mut next_commit = Instant::now() + Duration::from_millis(100);
    let mut inserted_total = 0u64;
    let mut commit_count = 0u64;

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
        next_commit += Duration::from_millis(100);

        let messages: Vec<Message> = {
            let mut buf = buffer.lock().unwrap();
            std::mem::take(&mut *buf)
        };

        if !messages.is_empty() {
            let count = messages.len();
            commit_batch(&conn, &messages)?;
            cleanup(&conn)?;
            inserted_total += count as u64;
            commit_count += 1;
            tracing::debug!("[commit #{commit_count}] {count} 件, 累計: {inserted_total}");
        }
    }

    stop.store(true, Ordering::Relaxed);
    write_thread.join().unwrap();
    tracing::info!("--- Writer 完了: 総挿入 {inserted_total} 件 ---");
    Ok(())
}
