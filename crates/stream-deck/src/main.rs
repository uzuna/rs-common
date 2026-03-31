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
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use elgato_streamdeck::DeviceStateUpdate;
use tracing::{debug, error, info, warn};

use metrics::{MetricsProvider, MetricsSnapshot};
use renderer::SectionSpec;

/// Stream Deck に表示するメトリクスの履歴 (最大 HISTORY_LEN サンプル)
const HISTORY_LEN: usize = 10;

/// 更新間隔 (1秒 = 1バー/サンプル)
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// 再接続待機時間
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// 入力ポーリング間隔 (タック内で輝度操作を素早く反映するための刻み幅)
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// デフォルト輝度 (%)
const DEFAULT_BRIGHTNESS: u8 = 70;

/// 輝度変化の最小・最大・1ステップ (エンコーダ 1 click)
const BRIGHTNESS_MIN: u8 = 5;
const BRIGHTNESS_MAX: u8 = 100;
const BRIGHTNESS_STEP: u8 = 5;

/// 輝度を制御する右端エンコーダのインデックス (0-3、Plus は 4 個)
const ENCODER_BRIGHTNESS: u8 = 3;

/// 輝度をデフォルトにリセットする右下ボタンのインデックス (0-7、2行×4列)
const BUTTON_BRIGHTNESS_RESET: u8 = 7;

/// 色サイクルを担当する左上ボタンのインデックス
const BUTTON_COLOR_CYCLE: u8 = 0;

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
    let mut brightness = DEFAULT_BRIGHTNESS;
    let mut button_states = ButtonStates::new();

    loop {
        info!("Stream Deck+ への接続を試みています...");

        match device::HardwareManager::connect() {
            Ok(hw) => {
                info!("接続完了。ダッシュボード表示開始");
                if let Err(e) = run_loop(
                    &hw,
                    &renderer,
                    &mut metrics,
                    &mut history,
                    &mut brightness,
                    &mut button_states,
                ) {
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
///
/// `brightness`・`button_states` は再接続をまたいで保持されるため、呼び出し元が所有する。
fn run_loop(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    metrics: &mut MetricsProvider,
    history: &mut MetricsHistory,
    brightness: &mut u8,
    button_states: &mut ButtonStates,
) -> anyhow::Result<()> {
    let reader = hw.get_reader();
    hw.set_brightness(*brightness)?;

    // 接続時: 全ボタンをクリアしてから保存済み状態を復元する
    hw.clear_buttons()?;
    for (key, &color) in button_states.colors.iter().enumerate() {
        if color != ButtonColor::Black {
            hw.set_button_color(key as u8, color.to_rgb())?;
        }
    }

    loop {
        // メトリクス取得 & 履歴に追加
        let snap = metrics.sample();
        history.push(&snap);

        tracing::debug!(
            cpu = %format!("{:.1}%", snap.cpu_pct),
            mem = %format!("{:.1}%", snap.mem_pct),
            load = %format!("{:.2}", snap.load_one),
            bright = *brightness,
            "LCD 更新",
        );

        // セクションデータ構築
        let cpu_norm = history.cpu_normalized();
        let mem_norm = history.mem_normalized();
        let load_norm = history.load_normalized(snap.cpu_count);
        let bright_norm = vec![*brightness as f32 / 100.0; HISTORY_LEN];

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
                label: "BRIGHT",
                value_text: format!("{}%", brightness),
                history: &bright_norm,
            },
        ];

        let image = renderer.render(&sections);

        if let Err(e) = hw.set_lcd_strip_image(image) {
            error!("LCD 書き込みエラー: {e:#}");
            return Err(anyhow::anyhow!("{e}"));
        }

        // 次のTickまで入力をポーリングする (INPUT_POLL_INTERVAL 刻み)
        let deadline = Instant::now() + TICK_INTERVAL;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let poll_timeout = INPUT_POLL_INTERVAL.min(remaining);

            let updates = reader
                .read(Some(poll_timeout))
                .map_err(|e| anyhow::anyhow!("入力読み取りエラー: {e}"))?;

            let mut brightness_changed = false;
            for update in updates {
                match update {
                    DeviceStateUpdate::EncoderTwist(idx, delta) if idx == ENCODER_BRIGHTNESS => {
                        let new_b = apply_brightness_delta(*brightness, delta);
                        if new_b != *brightness {
                            *brightness = new_b;
                            brightness_changed = true;
                        }
                    }
                    DeviceStateUpdate::ButtonDown(idx) if idx == BUTTON_BRIGHTNESS_RESET => {
                        if *brightness != DEFAULT_BRIGHTNESS {
                            *brightness = DEFAULT_BRIGHTNESS;
                            brightness_changed = true;
                        }
                    }
                    DeviceStateUpdate::ButtonDown(idx) if idx == BUTTON_COLOR_CYCLE => {
                        let color = button_states.cycle(idx as usize);
                        debug!(key = idx, ?color, "ボタン色サイクル");
                        if let Err(e) = hw.set_button_color(idx, color.to_rgb()) {
                            error!("ボタン色設定エラー: {e:#}");
                            return Err(anyhow::anyhow!("{e}"));
                        }
                    }
                    _ => {}
                }
            }

            if brightness_changed {
                debug!(bright = *brightness, "輝度変更");
                if let Err(e) = hw.set_brightness(*brightness) {
                    error!("輝度設定エラー: {e:#}");
                    return Err(anyhow::anyhow!("{e}"));
                }
            }
        }
    }
}

// ── MetricsHistory ────────────────────────────────────────────────

/// エンコーダのデルタを輝度に適用し、範囲内にクランプして返す
fn apply_brightness_delta(current: u8, delta: i8) -> u8 {
    (current as i16 + delta as i16 * BRIGHTNESS_STEP as i16)
        .clamp(BRIGHTNESS_MIN as i16, BRIGHTNESS_MAX as i16) as u8
}

// ── ButtonColor / ButtonStates ─────────────────────────────────────

/// ボタンが取りうる表示色 (Black → R → G → B → Black 循環)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonColor {
    Black,
    Red,
    Green,
    Blue,
}

impl ButtonColor {
    /// 次の色に遷移する
    fn next(self) -> Self {
        match self {
            ButtonColor::Black => ButtonColor::Red,
            ButtonColor::Red => ButtonColor::Green,
            ButtonColor::Green => ButtonColor::Blue,
            ButtonColor::Blue => ButtonColor::Black,
        }
    }

    /// `image::Rgb<u8>` に変換する
    fn to_rgb(self) -> image::Rgb<u8> {
        match self {
            ButtonColor::Black => image::Rgb([0, 0, 0]),
            ButtonColor::Red => image::Rgb([255, 0, 0]),
            ButtonColor::Green => image::Rgb([0, 255, 0]),
            ButtonColor::Blue => image::Rgb([0, 0, 255]),
        }
    }
}

/// 全ボタン (8個) の現在色を保持する状態
struct ButtonStates {
    colors: [ButtonColor; 8],
}

impl ButtonStates {
    fn new() -> Self {
        Self {
            colors: [ButtonColor::Black; 8],
        }
    }

    /// ボタン `key` の色を次に進めて新しい色を返す
    fn cycle(&mut self, key: usize) -> ButtonColor {
        self.colors[key] = self.colors[key].next();
        self.colors[key]
    }
}

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

    /// 正常系: ButtonColor::next は Black→R→G→B→Black と循環する
    #[test]
    fn test_button_color_cycle() {
        let cases: &[(ButtonColor, ButtonColor)] = &[
            (ButtonColor::Black, ButtonColor::Red),
            (ButtonColor::Red, ButtonColor::Green),
            (ButtonColor::Green, ButtonColor::Blue),
            (ButtonColor::Blue, ButtonColor::Black),
        ];
        for &(input, expected) in cases {
            assert_eq!(input.next(), expected, "input={input:?}");
        }
    }

    /// 正常系: ButtonStates::cycle は state を更新して返す、全ボタン独立
    #[test]
    fn test_button_states_independent() {
        let mut states = ButtonStates::new();
        // ボタン 0 を 1 回サイクル → Red
        assert_eq!(states.cycle(0), ButtonColor::Red);
        // ボタン 1 はまだ Black のまま
        assert_eq!(states.colors[1], ButtonColor::Black);
        // ボタン 0 をさらに 3 回 → Green → Blue → Black
        states.cycle(0);
        states.cycle(0);
        assert_eq!(states.cycle(0), ButtonColor::Black);
    }

    /// 値域確認: apply_brightness_delta は範囲外をクランプし、ステップを正しく適用する
    #[test]
    fn test_brightness_adjustment() {
        let cases: &[(u8, i8, u8)] = &[
            (70, 1, 75),   // 正常増加 (1 click CW = +5%)
            (70, -1, 65),  // 正常減少 (1 click CCW = -5%)
            (100, 1, 100), // 上限クランプ (100% 超にならない)
            (5, -1, 5),    // 下限クランプ (5% 未満にならない)
            (70, 0, 70),   // デルタ 0 は変化なし
            (70, 2, 80),   // 2 click で +10%
        ];
        for &(current, delta, expected) in cases {
            let got = apply_brightness_delta(current, delta);
            assert_eq!(got, expected, "current={current} delta={delta}");
        }
    }

    /// 正常系: brightness_changed フラグ: 同値では変化しない
    #[test]
    fn test_brightness_no_change_at_limits() {
        // 上限でさらに増やしても変化なし
        assert_eq!(apply_brightness_delta(BRIGHTNESS_MAX, 1), BRIGHTNESS_MAX);
        // 下限でさらに減らしても変化なし
        assert_eq!(apply_brightness_delta(BRIGHTNESS_MIN, -1), BRIGHTNESS_MIN);
    }
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
