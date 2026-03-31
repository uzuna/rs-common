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
use std::fmt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use elgato_streamdeck::DeviceStateUpdate;
use serde::Deserialize;
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

/// Pending 状態を既読にするまでの猶予
const NOTIFICATION_PENDING_TIMEOUT: Duration = Duration::from_secs(2);

/// 通知受信用の UNIX ドメインソケット
const NOTIFICATION_SOCKET_PATH: &str = "/tmp/rs-common-stream-deck-notify.sock";

/// 受信する通知 1 件の最大サイズ
const NOTIFICATION_MAX_BYTES: usize = 4096;

/// LCD に表示する通知サマリ最大文字数
const NOTIFICATION_SUMMARY_MAX_CHARS: usize = 14;

/// Stream Deck CPU モニター
#[derive(Parser)]
#[command(
    name = "stream-deck",
    about = "Elgato Stream Deck+ の LCD に CPU・メモリ・ロードアベレージを表示する開発ツール",
    version
)]
struct Cli {
    /// 通知が既読になった時のスロット挙動
    #[arg(long, value_enum, default_value_t = SlotCompactionMode::KeepGap)]
    notif_compaction: SlotCompactionMode,

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
        None => run_monitor(cli.notif_compaction),
    }
}

/// デフォルト動作: 4セクションのメトリクスダッシュボードを LCD に表示し続ける
fn run_monitor(compaction_mode: SlotCompactionMode) -> anyhow::Result<()> {
    info!("Stream Deck マルチメトリクスモニター 起動");
    info!(mode = %compaction_mode, "通知既読時のスロット挙動");

    let renderer = renderer::Renderer::new()?;
    let mut metrics = MetricsProvider::new();
    let mut history = MetricsHistory::new(HISTORY_LEN);
    let mut brightness = DEFAULT_BRIGHTNESS;
    let mut notification_source = NotificationSource::bind(NOTIFICATION_SOCKET_PATH)?;
    let mut notification_state = NotificationState::new(compaction_mode);

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
                    &mut notification_source,
                    &mut notification_state,
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
/// `brightness`・通知状態は再接続をまたいで保持されるため、呼び出し元が所有する。
fn run_loop(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    metrics: &mut MetricsProvider,
    history: &mut MetricsHistory,
    brightness: &mut u8,
    notification_source: &mut NotificationSource,
    notification_state: &mut NotificationState,
) -> anyhow::Result<()> {
    let reader = hw.get_reader();
    hw.set_brightness(*brightness)?;

    // 接続時: 全ボタンをクリアしてから通知状態を復元する
    hw.clear_buttons()?;
    refresh_notification_buttons(hw, notification_state)?;

    loop {
        let mut notification_state_changed = false;

        if drain_notifications(notification_source, notification_state)? {
            notification_state_changed = true;
        }
        if notification_state.expire_pending(Instant::now()) {
            notification_state_changed = true;
        }
        if notification_state_changed {
            refresh_notification_buttons(hw, notification_state)?;
        }

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
        let notif_norm = notification_state.notification_history(HISTORY_LEN);

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
                label: "NOTIF",
                value_text: notification_state.overlay_text(),
                history: &notif_norm,
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
            let mut notification_state_changed = false;

            if drain_notifications(notification_source, notification_state)? {
                notification_state_changed = true;
            }
            if notification_state.expire_pending(Instant::now()) {
                notification_state_changed = true;
            }

            for update in updates {
                match update {
                    DeviceStateUpdate::EncoderTwist(idx, delta) if idx == ENCODER_BRIGHTNESS => {
                        let new_b = apply_brightness_delta(*brightness, delta);
                        if new_b != *brightness {
                            *brightness = new_b;
                            brightness_changed = true;
                        }
                    }
                    DeviceStateUpdate::ButtonDown(idx) => {
                        if let Some(action) =
                            notification_state.on_button_down(idx as usize, Instant::now())
                        {
                            notification_state_changed = true;
                            if let NotificationPressAction::Execute(payload) = action {
                                if let Err(e) = execute_payload(&payload) {
                                    warn!(%payload, "通知アクション実行失敗: {e:#}");
                                }
                            }
                            continue;
                        }

                        if idx == BUTTON_BRIGHTNESS_RESET && *brightness != DEFAULT_BRIGHTNESS {
                            *brightness = DEFAULT_BRIGHTNESS;
                            brightness_changed = true;
                        }
                    }
                    _ => {}
                }
            }

            if notification_state_changed {
                refresh_notification_buttons(hw, notification_state)?;
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

fn execute_payload(payload: &str) -> anyhow::Result<()> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        anyhow::bail!("action_payload が空です");
    }

    let status = Command::new("xdg-open").arg(trimmed).status()?;
    if !status.success() {
        anyhow::bail!("xdg-open が失敗しました: status={status}");
    }
    Ok(())
}

fn refresh_notification_buttons(
    hw: &device::HardwareManager,
    state: &NotificationState,
) -> anyhow::Result<()> {
    for idx in 0..state.slots.len() {
        hw.set_button_color(idx as u8, state.slots[idx].color().to_rgb())?;
    }
    Ok(())
}

fn drain_notifications(
    source: &mut NotificationSource,
    state: &mut NotificationState,
) -> anyhow::Result<bool> {
    let mut changed = false;
    loop {
        let Some(item) = source.try_recv()? else {
            break;
        };
        if let Some(slot) = state.insert(item, Instant::now()) {
            info!(slot, unread = state.unread_count(), "通知受信");
            changed = true;
        }
    }
    Ok(changed)
}

// ── Notification Types ──────────────────────────────────────────────

#[derive(Debug)]
struct NotificationSource {
    socket: UnixDatagram,
    recv_buf: [u8; NOTIFICATION_MAX_BYTES],
}

impl NotificationSource {
    fn bind(path: &str) -> anyhow::Result<Self> {
        let socket_path = Path::new(path);
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }

        let socket = UnixDatagram::bind(socket_path)?;
        socket.set_nonblocking(true)?;
        info!(path, "通知ソケット待受を開始");

        Ok(Self {
            socket,
            recv_buf: [0; NOTIFICATION_MAX_BYTES],
        })
    }

    fn try_recv(&mut self) -> anyhow::Result<Option<IncomingNotification>> {
        match self.socket.recv(&mut self.recv_buf) {
            Ok(size) => {
                let payload = std::str::from_utf8(&self.recv_buf[..size])?;
                let packet: NotificationPacket = serde_json::from_str(payload)?;
                let item = IncomingNotification {
                    summary: packet.summary,
                    body: packet.body.unwrap_or_default(),
                    action_payload: packet.action_payload,
                };
                Ok(Some(item))
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct NotificationPacket {
    summary: String,
    #[serde(default)]
    body: Option<String>,
    action_payload: String,
}

#[derive(Debug)]
struct IncomingNotification {
    summary: String,
    body: String,
    action_payload: String,
}

#[derive(Debug, Clone)]
struct NotificationItem {
    id: u64,
    summary: String,
    body: String,
    action_payload: String,
    created_at: Instant,
}

#[derive(Debug, Clone)]
enum SlotState {
    Empty,
    Unread {
        item: NotificationItem,
    },
    Pending {
        item: NotificationItem,
        deadline: Instant,
    },
}

impl SlotState {
    fn color(&self) -> ButtonColor {
        match self {
            SlotState::Empty => ButtonColor::Black,
            SlotState::Unread { .. } => ButtonColor::Blue,
            SlotState::Pending { .. } => ButtonColor::Yellow,
        }
    }
}

#[derive(Debug)]
enum NotificationPressAction {
    Execute(String),
    RevertToUnread,
}

struct NotificationState {
    slots: [SlotState; 8],
    next_id: u64,
    compaction_mode: SlotCompactionMode,
}

impl NotificationState {
    fn new(compaction_mode: SlotCompactionMode) -> Self {
        Self {
            slots: std::array::from_fn(|_| SlotState::Empty),
            next_id: 1,
            compaction_mode,
        }
    }

    fn insert(&mut self, incoming: IncomingNotification, now: Instant) -> Option<usize> {
        let item = NotificationItem {
            id: self.next_id,
            summary: incoming.summary,
            body: incoming.body,
            action_payload: incoming.action_payload,
            created_at: now,
        };
        self.next_id += 1;

        if let Some((idx, _)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| matches!(slot, SlotState::Empty))
        {
            self.slots[idx] = SlotState::Unread { item };
            return Some(idx);
        }

        let target = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                SlotState::Unread { item } => Some((idx, item.created_at)),
                _ => None,
            })
            .min_by_key(|(_, created_at)| *created_at)
            .map(|(idx, _)| idx);

        match target {
            Some(idx) => {
                self.slots[idx] = SlotState::Unread { item };
                Some(idx)
            }
            None => {
                warn!("通知スロットが埋まっているため破棄しました");
                None
            }
        }
    }

    fn unread_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| matches!(slot, SlotState::Unread { .. }))
            .count()
    }

    fn total_active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| !matches!(slot, SlotState::Empty))
            .count()
    }

    fn on_button_down(&mut self, idx: usize, now: Instant) -> Option<NotificationPressAction> {
        if idx >= self.slots.len() {
            return None;
        }

        match &self.slots[idx] {
            SlotState::Unread { item } => {
                let payload = item.action_payload.clone();
                let moved = item.clone();
                debug!(slot = idx, notif_id = item.id, "通知アクションを実行");
                self.slots[idx] = SlotState::Pending {
                    item: moved,
                    deadline: now + NOTIFICATION_PENDING_TIMEOUT,
                };
                Some(NotificationPressAction::Execute(payload))
            }
            SlotState::Pending { item, .. } => {
                debug!(slot = idx, notif_id = item.id, "通知を未読に戻しました");
                self.slots[idx] = SlotState::Unread { item: item.clone() };
                Some(NotificationPressAction::RevertToUnread)
            }
            SlotState::Empty => None,
        }
    }

    fn expire_pending(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for slot in &mut self.slots {
            if let SlotState::Pending { deadline, .. } = slot {
                if *deadline <= now {
                    *slot = SlotState::Empty;
                    changed = true;
                }
            }
        }

        if changed && self.compaction_mode == SlotCompactionMode::CompactLeft {
            self.compact_left();
        }

        changed
    }

    fn compact_left(&mut self) {
        let mut non_empty: Vec<SlotState> = self
            .slots
            .iter()
            .filter(|slot| !matches!(slot, SlotState::Empty))
            .cloned()
            .collect();
        non_empty.resize_with(self.slots.len(), || SlotState::Empty);

        for (idx, slot) in non_empty.into_iter().enumerate() {
            self.slots[idx] = slot;
        }
    }

    fn latest_item(&self) -> Option<&NotificationItem> {
        self.slots
            .iter()
            .filter_map(|slot| match slot {
                SlotState::Unread { item } | SlotState::Pending { item, .. } => Some(item),
                SlotState::Empty => None,
            })
            .max_by_key(|item| item.created_at)
    }

    fn overlay_text(&self) -> String {
        let active = self.total_active_count();
        if active == 0 {
            return "0".to_string();
        }

        let summary = self
            .latest_item()
            .map(|item| {
                let text = if item.summary.trim().is_empty() {
                    item.body.as_str()
                } else {
                    item.summary.as_str()
                };
                format!(
                    "#{} {}",
                    item.id,
                    truncate_chars(text, NOTIFICATION_SUMMARY_MAX_CHARS)
                )
            })
            .unwrap_or_else(|| "-".to_string());

        format!("{active} {summary}")
    }

    fn notification_history(&self, len: usize) -> Vec<f32> {
        let active_ratio = self.total_active_count() as f32 / self.slots.len() as f32;
        vec![active_ratio; len]
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SlotCompactionMode {
    KeepGap,
    CompactLeft,
}

impl fmt::Display for SlotCompactionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            SlotCompactionMode::KeepGap => "keep-gap",
            SlotCompactionMode::CompactLeft => "compact-left",
        };
        write!(f, "{label}")
    }
}

// ── ButtonColor ───────────────────────────────────────────────────

/// 通知状態の表示色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonColor {
    Black,
    Blue,
    Yellow,
}

impl ButtonColor {
    /// `image::Rgb<u8>` に変換する
    fn to_rgb(self) -> image::Rgb<u8> {
        match self {
            ButtonColor::Black => image::Rgb([0, 0, 0]),
            ButtonColor::Blue => image::Rgb([0, 0, 255]),
            ButtonColor::Yellow => image::Rgb([255, 200, 0]),
        }
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

    #[test]
    fn test_notification_transition_unread_pending_unread() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let now = Instant::now();
        state.insert(
            IncomingNotification {
                summary: "summary".to_string(),
                body: "body".to_string(),
                action_payload: "vscode://file/tmp/result.csv".to_string(),
            },
            now,
        );

        let action = state.on_button_down(0, now + Duration::from_millis(1));
        assert!(matches!(action, Some(NotificationPressAction::Execute(_))));
        assert!(matches!(state.slots[0], SlotState::Pending { .. }));

        let action2 = state.on_button_down(0, now + Duration::from_millis(2));
        assert!(matches!(
            action2,
            Some(NotificationPressAction::RevertToUnread)
        ));
        assert!(matches!(state.slots[0], SlotState::Unread { .. }));
    }

    #[test]
    fn test_notification_pending_timeout_to_empty() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let now = Instant::now();
        state.insert(
            IncomingNotification {
                summary: "summary".to_string(),
                body: "body".to_string(),
                action_payload: "vscode://file/tmp/result.csv".to_string(),
            },
            now,
        );
        state.on_button_down(0, now);

        let changed =
            state.expire_pending(now + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(1));
        assert!(changed);
        assert!(matches!(state.slots[0], SlotState::Empty));
    }

    #[test]
    fn test_notification_replaces_oldest_unread_when_full() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let base = Instant::now();

        for i in 0..8 {
            state.insert(
                IncomingNotification {
                    summary: format!("n{i}"),
                    body: String::new(),
                    action_payload: format!("vscode://file/tmp/n{i}.txt"),
                },
                base + Duration::from_millis(i as u64),
            );
        }

        state.insert(
            IncomingNotification {
                summary: "latest".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/latest.txt".to_string(),
            },
            base + Duration::from_secs(1),
        );

        let first_slot_summary = match &state.slots[0] {
            SlotState::Unread { item } => item.summary.clone(),
            _ => String::new(),
        };
        assert_eq!(first_slot_summary, "latest");
    }

    #[test]
    fn test_notification_compact_left_on_expire() {
        let mut state = NotificationState::new(SlotCompactionMode::CompactLeft);
        let base = Instant::now();

        state.insert(
            IncomingNotification {
                summary: "a".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/a".to_string(),
            },
            base,
        );
        state.insert(
            IncomingNotification {
                summary: "b".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/b".to_string(),
            },
            base + Duration::from_millis(1),
        );
        state.insert(
            IncomingNotification {
                summary: "c".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/c".to_string(),
            },
            base + Duration::from_millis(2),
        );

        state.on_button_down(1, base + Duration::from_millis(3));
        state.expire_pending(base + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(10));

        let slot0 = matches!(state.slots[0], SlotState::Unread { .. });
        let slot1 = matches!(state.slots[1], SlotState::Unread { .. });
        let slot2 = matches!(state.slots[2], SlotState::Empty);
        assert!(slot0 && slot1 && slot2);
    }

    #[test]
    fn test_notification_keep_gap_on_expire() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let base = Instant::now();

        state.insert(
            IncomingNotification {
                summary: "a".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/a".to_string(),
            },
            base,
        );
        state.insert(
            IncomingNotification {
                summary: "b".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/b".to_string(),
            },
            base + Duration::from_millis(1),
        );
        state.insert(
            IncomingNotification {
                summary: "c".to_string(),
                body: String::new(),
                action_payload: "vscode://file/tmp/c".to_string(),
            },
            base + Duration::from_millis(2),
        );

        state.on_button_down(1, base + Duration::from_millis(3));
        state.expire_pending(base + NOTIFICATION_PENDING_TIMEOUT + Duration::from_millis(10));

        let slot0 = matches!(state.slots[0], SlotState::Unread { .. });
        let slot1 = matches!(state.slots[1], SlotState::Empty);
        let slot2 = matches!(state.slots[2], SlotState::Unread { .. });
        assert!(slot0 && slot1 && slot2);
    }
}
