use anyhow::Result;
use rusqlite::params;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db;

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

/// Reader を実行する
///
/// - `channel_id`: 監視対象のチャネル ID
/// - `duration`:   実行時間（`None` の場合は無限）
pub fn run(path: &Path, channel_id: i64, duration: Option<Duration>) -> Result<()> {
    let conn = db::open_reader(path)?;
    tracing::info!("Reader 起動: channel_id={channel_id}");

    // 約 30Hz（33.3ms間隔）
    let interval = Duration::from_nanos(33_333_333);
    let start = Instant::now();
    let mut next = Instant::now() + interval;

    // 統計用バッファ（1秒ウィンドウ）
    let mut window: Vec<(u64, u64)> = Vec::new(); // (q1_nanos, q2_nanos)
    let mut last_report = Instant::now();
    let mut last_count: i64 = 0;

    loop {
        if let Some(d) = duration {
            if start.elapsed() >= d {
                break;
            }
        }

        let now = Instant::now();
        if let Some(rem) = next.checked_duration_since(now) {
            std::thread::sleep(rem);
        }
        next += interval;

        // クエリ1: 最新値取得
        let t1 = Instant::now();
        let _: Option<i64> = conn
            .query_row(
                "SELECT timestamp FROM latest_states WHERE channel_id = ?1",
                params![channel_id],
                |row| row.get(0),
            )
            .ok();
        let q1_nanos = t1.elapsed().as_nanos() as u64;

        // クエリ2: 直近1秒のメッセージ件数
        let cutoff = now_nanos() - 1_000_000_000i64;
        let t2 = Instant::now();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages WHERE log_time > ?1",
                params![cutoff],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let q2_nanos = t2.elapsed().as_nanos() as u64;

        last_count = count;
        window.push((q1_nanos, q2_nanos));

        // 1秒ごとに統計をerrに出力
        if last_report.elapsed() >= Duration::from_secs(1) {
            print_stats(&window, last_count);
            window.clear();
            last_report = Instant::now();
        }
    }

    // 残余統計を出力
    if !window.is_empty() {
        print_stats(&window, last_count);
    }
    Ok(())
}

fn print_stats(window: &[(u64, u64)], msg_count: i64) {
    let n = window.len() as u64;
    if n == 0 {
        return;
    }
    let q1_avg = window.iter().map(|x| x.0).sum::<u64>() / n;
    let q1_max = window.iter().map(|x| x.0).max().unwrap_or(0);
    let q2_avg = window.iter().map(|x| x.1).sum::<u64>() / n;
    let q2_max = window.iter().map(|x| x.1).max().unwrap_or(0);
    tracing::info!(
        "[reader] {n}回/s | latest: avg={:.3}ms max={:.3}ms | count({msg_count}): avg={:.3}ms max={:.3}ms",
        q1_avg as f64 / 1e6,
        q1_max as f64 / 1e6,
        q2_avg as f64 / 1e6,
        q2_max as f64 / 1e6,
    );
}
