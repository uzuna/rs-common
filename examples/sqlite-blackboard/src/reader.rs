use anyhow::Result;
use rusqlite::params;
use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db;

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

/// チャネルの指定方法
pub enum ChannelSpec {
    /// チャネル ID を直接指定
    Id(i64),
    /// トピック名から最新の channel_id を解決（Writer 再起動後も追従する）
    Topic(String),
}

struct Sample {
    q1_nanos: u64,
    q2_nanos: u64,
    /// latest_states の timestamp から現在時刻までのナノ秒（データがない場合は None）
    stale_nanos: Option<u64>,
    q1_error: bool,
}

/// トピック名から最新の channel_id を引く（見つかるまで最大 30 秒リトライ）
fn resolve_channel_id(conn: &rusqlite::Connection, topic: &str) -> Result<i64> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT id FROM channels WHERE topic_name = ?1 ORDER BY id DESC LIMIT 1",
            params![topic],
            |row| row.get(0),
        );
        match result {
            Ok(id) => return Ok(id),
            Err(_) => {
                if Instant::now() >= deadline {
                    anyhow::bail!("トピック '{topic}' が 30 秒以内に見つかりませんでした");
                }
                tracing::warn!("トピック '{topic}' が未登録。2秒後にリトライ...");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Reader を実行する
///
/// - `channel_spec`: 監視対象チャネルの指定方法
/// - `reader_id`:    ログ出力用の識別子
/// - `duration`:     実行時間（`None` の場合は無限）
pub fn run(
    path: &Path,
    channel_spec: ChannelSpec,
    reader_id: &str,
    duration: Option<Duration>,
) -> Result<()> {
    let conn = db::open_reader(path)?;

    // channel_id を解決
    let mut channel_id = match &channel_spec {
        ChannelSpec::Id(id) => *id,
        ChannelSpec::Topic(topic) => {
            tracing::info!("[{reader_id}] トピック '{topic}' の channel_id を解決中...");
            resolve_channel_id(&conn, topic)?
        }
    };
    tracing::info!("[{reader_id}] Reader 起動: channel_id={channel_id}");

    // 約 30Hz（33.3ms間隔）
    let interval = Duration::from_nanos(33_333_333);
    let start = Instant::now();
    let mut next = Instant::now() + interval;

    // 統計用バッファ（1秒ウィンドウ）
    let mut window: Vec<Sample> = Vec::new();
    let mut last_report = Instant::now();
    let mut last_count: i64 = 0;

    // 最大 60 秒のローリングウィンドウ（(取得時刻, q1_ns, q2_ns)）
    let mut rolling: VecDeque<(Instant, u64, u64)> = VecDeque::new();
    const ROLLING_WINDOW: Duration = Duration::from_secs(60);

    // トピック指定の場合、channel_id を定期的に再解決（Writer 再起動追従）
    let mut last_channel_resolve = Instant::now();
    let channel_resolve_interval = Duration::from_secs(5);

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

        // トピック指定の場合: 5秒ごとに channel_id を再解決
        if matches!(channel_spec, ChannelSpec::Topic(_)) {
            if last_channel_resolve.elapsed() >= channel_resolve_interval {
                if let ChannelSpec::Topic(ref topic) = channel_spec {
                    if let Ok(new_id) = resolve_channel_id(&conn, topic) {
                        if new_id != channel_id {
                            tracing::info!(
                                "[{reader_id}] channel_id 更新: {channel_id} → {new_id}"
                            );
                            channel_id = new_id;
                        }
                    }
                }
                last_channel_resolve = Instant::now();
            }
        }

        // クエリ1: 最新値取得（タイムスタンプで鮮度を確認）
        let t1 = Instant::now();
        let q1_result: rusqlite::Result<i64> = conn.query_row(
            "SELECT timestamp FROM latest_states WHERE channel_id = ?1",
            params![channel_id],
            |row| row.get(0),
        );
        let q1_nanos = t1.elapsed().as_nanos() as u64;

        let (q1_error, stale_nanos) = match q1_result {
            Ok(ts) => {
                let now_ns = now_nanos();
                let stale = if now_ns > ts { (now_ns - ts) as u64 } else { 0 };
                (false, Some(stale))
            }
            Err(_) => (true, None),
        };

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
        let now_instant = Instant::now();
        window.push(Sample { q1_nanos, q2_nanos, stale_nanos, q1_error });
        rolling.push_back((now_instant, q1_nanos, q2_nanos));

        // 1秒ごとに統計を出力
        if last_report.elapsed() >= Duration::from_secs(1) {
            // 60 秒より古いエントリを刈り取る
            let cutoff_instant = now_instant
                .checked_sub(ROLLING_WINDOW)
                .unwrap_or(now_instant);
            while rolling.front().is_some_and(|(t, _, _)| *t < cutoff_instant) {
                rolling.pop_front();
            }
            let rolling_stats = compute_rolling_stats(&rolling);
            print_stats(reader_id, &window, last_count, rolling_stats);
            window.clear();
            last_report = Instant::now();
        }
    }

    if !window.is_empty() {
        let rolling_stats = compute_rolling_stats(&rolling);
        print_stats(reader_id, &window, last_count, rolling_stats);
    }
    Ok(())
}

/// ローリングウィンドウから統計を計算する。(q1_avg, q1_max, q2_avg, q2_max, samples) を返す
fn compute_rolling_stats(
    rolling: &VecDeque<(Instant, u64, u64)>,
) -> Option<(u64, u64, u64, u64, usize)> {
    let n = rolling.len();
    if n == 0 {
        return None;
    }
    let q1_avg = rolling.iter().map(|(_, q1, _)| q1).sum::<u64>() / n as u64;
    let q1_max = rolling.iter().map(|(_, q1, _)| *q1).max().unwrap_or(0);
    let q2_avg = rolling.iter().map(|(_, _, q2)| q2).sum::<u64>() / n as u64;
    let q2_max = rolling.iter().map(|(_, _, q2)| *q2).max().unwrap_or(0);
    Some((q1_avg, q1_max, q2_avg, q2_max, n))
}

fn print_stats(
    reader_id: &str,
    window: &[Sample],
    msg_count: i64,
    rolling: Option<(u64, u64, u64, u64, usize)>,
) {
    let n = window.len() as u64;
    if n == 0 {
        return;
    }
    let q1_avg = window.iter().map(|s| s.q1_nanos).sum::<u64>() / n;
    let q1_max = window.iter().map(|s| s.q1_nanos).max().unwrap_or(0);
    let q2_avg = window.iter().map(|s| s.q2_nanos).sum::<u64>() / n;
    let q2_max = window.iter().map(|s| s.q2_nanos).max().unwrap_or(0);
    let errors = window.iter().filter(|s| s.q1_error).count();

    // 鮮度の最大値（Writer が止まっていると大きくなる）
    let max_stale_ms = window
        .iter()
        .filter_map(|s| s.stale_nanos)
        .max()
        .unwrap_or(0)
        / 1_000_000;

    tracing::info!(
        "[{reader_id}] {n}回/s | \
         latest: avg={:.3}ms max={:.3}ms | \
         count({msg_count}): avg={:.3}ms max={:.3}ms | \
         stale_max={max_stale_ms}ms errors={errors}{}",
        q1_avg as f64 / 1e6,
        q1_max as f64 / 1e6,
        q2_avg as f64 / 1e6,
        q2_max as f64 / 1e6,
        rolling.map_or(String::new(), |(r1a, r1x, r2a, r2x, rn)| format!(
            " | 1min({rn}): latest avg={:.3}ms max={:.3}ms count avg={:.3}ms max={:.3}ms",
            r1a as f64 / 1e6,
            r1x as f64 / 1e6,
            r2a as f64 / 1e6,
            r2x as f64 / 1e6,
        )),
    );
}
