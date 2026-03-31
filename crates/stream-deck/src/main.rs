//! Stream Deck マルチメトリクスモニター
//!
//! サブコマンド:
//! * (なし)    — 4セクション LCD ダッシュボードを表示し続ける
//! * `list`    — 接続中の Stream Deck デバイスを一覧表示する
//! * `diagnose` — 接続環境を段階的に診断する

mod cmd;
mod device;
mod error;
mod metrics;
mod renderer;

use std::collections::VecDeque;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing::{error, info, warn};

use metrics::{MetricsProvider, MetricsSnapshot};
use renderer::SectionSpec;

/// Stream Deck に表示するメトリクスの履歴 (最大 HISTORY_LEN サンプル)
const HISTORY_LEN: usize = 10;

/// 更新間隔 (1秒 = 1バー/サンプル)
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// 再接続待機時間
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Stream Deck CPU モニター
#[derive(Parser)]
#[command(
    name = "stream-deck",
    about = "Elgato Stream Deck+ の LCD に CPU・メモリ・ロードアベレージを表示する開発ツール",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 接続中の Stream Deck デバイスを一覧表示する
    List,
    /// 接続環境を段階的に診断する (lsusb・hidraw・udev・権限・接続テスト)
    Diagnose,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::List) => cmd::list::run(),
        Some(Commands::Diagnose) => cmd::diagnose::run(),
        None => run_monitor(),
    }
}

/// デフォルト動作: 4セクションのメトリクスダッシュボードを LCD に表示し続ける
fn run_monitor() -> anyhow::Result<()> {
    info!("Stream Deck マルチメトリクスモニター 起動");

    let renderer = renderer::Renderer::new()?;
    let mut metrics = MetricsProvider::new();
    let mut history = MetricsHistory::new(HISTORY_LEN);

    loop {
        info!("Stream Deck+ への接続を試みています...");

        match device::HardwareManager::connect() {
            Ok(hw) => {
                info!("接続完了。ダッシュボード表示開始");
                if let Err(e) = run_loop(&hw, &renderer, &mut metrics, &mut history) {
                    warn!("デバイスエラー: {e:#} — 再接続します");
                }
            }
            Err(e) => {
                warn!("接続失敗: {e:#} — {RETRY_INTERVAL:?} 後に再試行します");
            }
        }

        std::thread::sleep(RETRY_INTERVAL);
    }
}

/// デバイスが接続されている間、定期的に LCD を更新する
fn run_loop(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    metrics: &mut MetricsProvider,
    history: &mut MetricsHistory,
) -> anyhow::Result<()> {
    loop {
        // メトリクス取得 & 履歴に追加
        let snap = metrics.sample();
        history.push(&snap);

        tracing::debug!(
            cpu = %format!("{:.1}%", snap.cpu_pct),
            mem = %format!("{:.1}%", snap.mem_pct),
            load = %format!("{:.2}", snap.load_one),
            "LCD 更新",
        );

        // セクションデータ構築
        let cpu_norm = history.cpu_normalized();
        let mem_norm = history.mem_normalized();
        let load_norm = history.load_normalized(snap.cpu_count);
        let empty: Vec<f32> = Vec::new();

        let sections: [SectionSpec; 4] = [
            SectionSpec {
                label: "CPU",
                value_text: format!("{:.1}%", snap.cpu_pct),
                history: &cpu_norm,
            },
            SectionSpec {
                label: "MEM",
                value_text: format_memory(snap.used_memory_bytes, snap.total_memory_bytes),
                history: &mem_norm,
            },
            SectionSpec {
                label: "LOAD",
                value_text: format!("{:.2}", snap.load_one),
                history: &load_norm,
            },
            SectionSpec {
                label: "---",
                value_text: String::new(),
                history: &empty,
            },
        ];

        let image = renderer.render(&sections);

        if let Err(e) = hw.set_lcd_strip_image(image) {
            error!("LCD 書き込みエラー: {e:#}");
            return Err(anyhow::anyhow!("{e}"));
        }

        std::thread::sleep(TICK_INTERVAL);
    }
}

// ── MetricsHistory ────────────────────────────────────────────────

/// メモリ使用量を "使用量/最大値" 形式の文字列に変換する (例: "6.2/16G")
fn format_memory(used: u64, total: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if total >= GB {
        let used_gb = used as f64 / GB as f64;
        let total_gb = total as f64 / GB as f64;
        format!("{used_gb:.1}/{total_gb:.0}G")
    } else {
        let used_mb = used / MB;
        let total_mb = total / MB;
        format!("{used_mb}/{total_mb}M")
    }
}

/// CPU・MEM・LOAD の直近 N サンプルを保持するリングバッファ
struct MetricsHistory {
    cpu: VecDeque<f32>,
    mem: VecDeque<f32>,
    load: VecDeque<f32>,
    capacity: usize,
}

impl MetricsHistory {
    fn new(capacity: usize) -> Self {
        Self {
            cpu: VecDeque::with_capacity(capacity),
            mem: VecDeque::with_capacity(capacity),
            load: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// スナップショットを追加する (超えた分は先頭から破棄)
    fn push(&mut self, snap: &MetricsSnapshot) {
        Self::push_to(&mut self.cpu, snap.cpu_pct, self.capacity);
        Self::push_to(&mut self.mem, snap.mem_pct, self.capacity);
        Self::push_to(&mut self.load, snap.load_one, self.capacity);
    }

    fn push_to(buf: &mut VecDeque<f32>, value: f32, cap: usize) {
        if buf.len() == cap {
            buf.pop_front();
        }
        buf.push_back(value);
    }

    /// CPU 履歴を 0.0..=1.0 に正規化して返す
    fn cpu_normalized(&self) -> Vec<f32> {
        self.cpu.iter().map(|&v| v / 100.0).collect()
    }

    /// MEM 履歴を 0.0..=1.0 に正規化して返す
    fn mem_normalized(&self) -> Vec<f32> {
        self.mem.iter().map(|&v| v / 100.0).collect()
    }

    /// LOAD 履歴を cpu_count を上限として 0.0..=1.0 に正規化して返す
    fn load_normalized(&self, cpu_count: usize) -> Vec<f32> {
        let max = cpu_count.max(1) as f32;
        self.load
            .iter()
            .map(|&v| (v / max).clamp(0.0, 1.0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 値域確認: push_to はリングバッファが capacity を超えない
    #[test]
    fn test_ring_buffer_capacity() {
        let cap = 5;
        let mut h = MetricsHistory::new(cap);
        for i in 0..10 {
            let snap = MetricsSnapshot {
                cpu_pct: i as f32,
                mem_pct: i as f32,
                used_memory_bytes: i as u64 * (1 << 30),
                total_memory_bytes: 16 * (1 << 30),
                load_one: i as f32 * 0.1,
                cpu_count: 4,
            };
            h.push(&snap);
            assert!(
                h.cpu.len() <= cap,
                "cpu バッファが capacity を超えた: {}",
                h.cpu.len()
            );
        }
        // 最後の cap 個だけ残っているはずなので末尾要素を確認
        assert_eq!(*h.cpu.back().unwrap(), 9.0);
        assert_eq!(*h.cpu.front().unwrap(), 5.0);
    }

    /// 正常系: load_normalized は cpu_count で割って 0..1 にクランプ
    #[test]
    fn test_load_normalized_clamp() {
        let cases: &[(f32, usize, f32)] = &[
            (0.0, 4, 0.0),
            (2.0, 4, 0.5),
            (4.0, 4, 1.0),
            (8.0, 4, 1.0), // 超過はクランプ
        ];
        for &(load, cpu_count, expected) in cases {
            let mut h = MetricsHistory::new(1);
            h.push(&MetricsSnapshot {
                cpu_pct: 0.0,
                mem_pct: 0.0,
                used_memory_bytes: 0,
                total_memory_bytes: 16 * (1 << 30),
                load_one: load,
                cpu_count,
            });
            let norm = h.load_normalized(cpu_count);
            let got = norm[0];
            assert!(
                (got - expected).abs() < 1e-5,
                "load={load} cpu_count={cpu_count}: got={got} expected={expected}"
            );
        }
    }

    /// 正常系・値域確認: format_memory が正しい文字列を返す
    #[test]
    fn test_format_memory() {
        const GB: u64 = 1 << 30;
        const MB: u64 = 1 << 20;
        let cases: &[(u64, u64, &str)] = &[
            (6 * GB + GB / 5, 16 * GB, "6.2/16G"), // 典型的な 16GB システム
            (0, 16 * GB, "0.0/16G"),               // 使用量ゼロ
            (16 * GB, 16 * GB, "16.0/16G"),        // 満杯
            (256 * MB, 512 * MB, "256/512M"),      // GB 未満 (512MB < 1GB)
        ];
        for &(used, total, expected) in cases {
            let got = format_memory(used, total);
            assert_eq!(got, expected, "used={used} total={total}");
        }
    }
}
