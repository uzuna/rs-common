//! Stream Deck マルチメトリクスモニター
//!
//! サブコマンド:
//! * (なし)    — 4セクション LCD ダッシュボードを表示し続ける
//! * `list`    — 接続中の Stream Deck デバイスを一覧表示する
//! * `diagnose` — 接続環境を段階的に診断する

mod cmd;
mod config;
mod device;
mod error;
mod metrics;
mod notifications;
mod plugin;
mod renderer;
mod section;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ab_glyph::{FontVec, PxScale};
use clap::{Parser, Subcommand};
use elgato_streamdeck::DeviceStateUpdate;
use image::{DynamicImage, Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use tracing::{debug, error, info, warn};

use metrics::MetricsSource;
use notifications::{
    NotificationPressAction, NotificationSource, NotificationState, SlotCompactionMode, SlotState,
};
use plugin::{BrightnessPlugin, CpuPlugin, DataPlugin, LoadPlugin, MemPlugin, NotifPlugin};
use renderer::SectionSpec;
use section::Section;

#[cfg(test)]
use notifications::NotificationItem;

#[cfg(feature = "file-watch")]
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(feature = "file-watch")]
use std::sync::mpsc;

/// セクションのデフォルト履歴長 (バー本数 = 秒数)
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

/// Stream Deck+ 物理ボタン数
const BUTTON_COUNT: usize = 8;

/// 通知受信用の UNIX ドメインソケット
const NOTIFICATION_SOCKET_PATH: &str = "/tmp/rs-common-stream-deck-notify.sock";

/// 設定ファイルリロードのデバウンス間隔
#[cfg(feature = "file-watch")]
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);

/// デフォルト設定ファイル
const DEFAULT_LAYOUT_CONFIG_PATH: &str = "crates/stream-deck/config/layout.toml";

/// ボタン画像用フォント (NotoSans-Regular, 120x120 ボタン用)
const BUTTON_FONT_SIZE: f32 = 18.0;
const BUTTON_FONT_DATA: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");

/// Stream Deck CPU モニター
#[derive(Parser)]
#[command(
    name = "stream-deck",
    about = "Elgato Stream Deck+ の LCD に CPU・メモリ・ロードアベレージを表示する開発ツール",
    version
)]
struct Cli {
    /// レイアウト設定ファイルのパス
    #[arg(long, default_value = DEFAULT_LAYOUT_CONFIG_PATH)]
    config: String,

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
        None => run_monitor(cli.notif_compaction, &cli.config),
    }
}

/// デフォルト動作: メトリクスダッシュボードを LCD に表示し続ける
fn run_monitor(compaction_mode: SlotCompactionMode, config_path: &str) -> anyhow::Result<()> {
    info!("Stream Deck マルチメトリクスモニター 起動");
    info!(mode = %compaction_mode, "通知既読時のスロット挙動");

    let initial_config = load_runtime_config(config_path)?;
    let mut config_manager = ConfigManager::new(config_path, initial_config);
    info!(
        path = config_path,
        ?config_manager.active.sections,
        "レイアウト設定を読み込みました"
    );

    let renderer = renderer::Renderer::new()?;

    // 共有データソース
    let sys_source: Rc<RefCell<MetricsSource>> = Rc::new(RefCell::new(MetricsSource::new()));
    let brightness: Rc<Cell<u8>> = Rc::new(Cell::new(DEFAULT_BRIGHTNESS));
    let notification_state: Rc<RefCell<NotificationState>> =
        Rc::new(RefCell::new(NotificationState::new(compaction_mode)));

    let mut sections = build_sections(
        config_manager.current(),
        &sys_source,
        &brightness,
        &notification_state,
    );

    let mut notification_source = NotificationSource::bind(NOTIFICATION_SOCKET_PATH)?;

    loop {
        info!("Stream Deck+ への接続を試みています...");

        match device::HardwareManager::connect() {
            Ok(hw) => {
                info!("接続完了。ダッシュボード表示開始");
                if let Err(e) = run_loop(
                    &hw,
                    &renderer,
                    &mut config_manager,
                    &sys_source,
                    &mut sections,
                    &brightness,
                    &mut notification_source,
                    &notification_state,
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

fn load_runtime_config(config_path: &str) -> anyhow::Result<RuntimeConfig> {
    let bundle = config::AppConfigBundle::load(config_path, terminal_title_capable())?;
    if bundle.report.has_errors() {
        warn!(errors = ?bundle.report.errors, "新設定モデル検証でエラーを検出しました");
    }
    if !bundle.report.warnings.is_empty() {
        info!(warnings = ?bundle.report.warnings, "新設定モデル検証の警告");
    }
    info!(watch_targets = ?bundle.watch_targets, "新設定モデルの監視対象を解決しました");
    if let Some(reason) = &bundle.report.ssh_disabled_reason {
        info!(reason = %reason, "新設定モデル: SSH ページを無効化します");
    }

    Ok(RuntimeConfig {
        sections: bundle.app.dashboard.sections,
        home_page_id: bundle.app.app.home,
        pages: bundle.app.pages,
        watch_targets: bundle.watch_targets,
    })
}

fn terminal_title_capable() -> bool {
    match std::env::var("STREAM_DECK_TERMINAL_TITLE_CAPABLE") {
        Ok(v) if v.eq_ignore_ascii_case("0") || v.eq_ignore_ascii_case("false") => false,
        Ok(v) if v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true") => true,
        Ok(_) => true,
        Err(_) => true,
    }
}

/// 設定からセクション列を構築する
fn build_sections(
    config: &RuntimeConfig,
    sys_source: &Rc<RefCell<MetricsSource>>,
    brightness: &Rc<Cell<u8>>,
    notification_state: &Rc<RefCell<NotificationState>>,
) -> Vec<Section> {
    config
        .sections
        .iter()
        .map(|sec| {
            let plugin: Box<dyn DataPlugin> = match sec.kind {
                config::DashboardSectionKind::Cpu => {
                    Box::new(CpuPlugin::new(Rc::clone(sys_source)))
                }
                config::DashboardSectionKind::Mem => {
                    Box::new(MemPlugin::new(Rc::clone(sys_source)))
                }
                config::DashboardSectionKind::Load => {
                    Box::new(LoadPlugin::new(Rc::clone(sys_source)))
                }
                config::DashboardSectionKind::Bright => {
                    Box::new(BrightnessPlugin::new(Rc::clone(brightness)))
                }
                config::DashboardSectionKind::Notif => {
                    Box::new(NotifPlugin::new(Rc::clone(notification_state)))
                }
            };
            Section::new(plugin, sec.capacity)
        })
        .collect()
}

/// デバイスが接続されている間、定期的に LCD を更新する
#[allow(clippy::too_many_arguments)]
fn run_loop(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    config_manager: &mut ConfigManager,
    sys_source: &Rc<RefCell<MetricsSource>>,
    sections: &mut Vec<Section>,
    brightness: &Rc<Cell<u8>>,
    notification_source: &mut NotificationSource,
    notification_state: &Rc<RefCell<NotificationState>>,
) -> anyhow::Result<()> {
    let reader = hw.get_reader();
    hw.set_brightness(brightness.get())?;
    let mut page_state = PageState::new(config_manager.current());

    // 接続時: 全ボタンをクリアしてから通知状態を復元する
    hw.clear_buttons()?;
    refresh_button_display(
        hw,
        &notification_state.borrow(),
        config_manager.current(),
        &page_state,
    )?;

    loop {
        if config_manager.poll_reload() {
            info!("レイアウトをリロードしました");
            *sections = build_sections(
                config_manager.current(),
                sys_source,
                brightness,
                notification_state,
            );
            page_state.reconcile(config_manager.current());
            refresh_button_display(
                hw,
                &notification_state.borrow(),
                config_manager.current(),
                &page_state,
            )?
        }

        let notification_state_changed =
            poll_notification_state(notification_source, notification_state)?;
        if notification_state_changed {
            refresh_button_display(
                hw,
                &notification_state.borrow(),
                config_manager.current(),
                &page_state,
            )?;
        }

        // メトリクスを一度だけ refresh してから各セクションを更新
        sys_source.borrow_mut().refresh();
        for section in sections.iter_mut() {
            section.tick();
        }

        let snap_log = sys_source.borrow();
        if let Some(snap) = snap_log.snapshot() {
            tracing::debug!(
                cpu = %format!("{:.1}%", snap.cpu_pct),
                mem = %format!("{:.1}%", snap.mem_pct),
                load = %format!("{:.2}", snap.load_one),
                bright = brightness.get(),
                "LCD 更新",
            );
        }

        // LCD はメトリクス表示を維持
        let frame = build_legacy_display_frame(sections);
        let image = render_display_frame(renderer, frame);

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

            let mut notification_state_changed =
                poll_notification_state(notification_source, notification_state)?;
            let input_outcome = handle_device_updates(
                updates,
                brightness,
                notification_state,
                config_manager.current(),
                &mut page_state,
            );
            let brightness_changed = input_outcome.brightness_changed;
            notification_state_changed |= input_outcome.notification_state_changed;
            let page_state_changed = input_outcome.page_state_changed;

            if notification_state_changed || page_state_changed {
                refresh_button_display(
                    hw,
                    &notification_state.borrow(),
                    config_manager.current(),
                    &page_state,
                )?;
            }

            if brightness_changed {
                debug!(bright = brightness.get(), "輝度変更");
                if let Err(e) = hw.set_brightness(brightness.get()) {
                    error!("輝度設定エラー: {e:#}");
                    return Err(anyhow::anyhow!("{e}"));
                }
            }
        }
    }
}

/// 描画入力の互換レイヤ。
/// 現行のセクション描画と将来のページ描画を同じ入口で扱うための列挙型。
enum DisplayFrame {
    LegacySections(Vec<SectionSpec>),
    #[allow(dead_code)]
    Page(PageDisplayModel),
}

#[allow(dead_code)]
/// ページ描画へ移行するための中間表示モデル。
/// 現段階では互換レイヤで `SectionSpec` に変換して描画する。
struct PageDisplayModel {
    title: String,
    overlay_text: Option<String>,
    tiles: Vec<PageTile>,
}

#[allow(dead_code)]
/// ページ内の1タイル分の表示情報。
/// `emphasis` は互換描画では簡易バー強度として扱う。
struct PageTile {
    label: String,
    value_text: String,
    emphasis: f32,
}

fn build_legacy_display_frame(sections: &[Section]) -> DisplayFrame {
    DisplayFrame::LegacySections(sections.iter().map(Section::as_spec).collect())
}

fn build_page_display_frame(config: &RuntimeConfig, page_state: &PageState) -> DisplayFrame {
    let page = config
        .pages
        .iter()
        .find(|p| p.id == page_state.current_page_id)
        .map(|p| {
            let tiles: Vec<PageTile> = p
                .items
                .iter()
                .map(|item| PageTile {
                    label: match item {
                        config::PageItemConfig::Nav { label, .. } => label.clone(),
                        config::PageItemConfig::Command { label, .. } => label.clone(),
                        config::PageItemConfig::Back { label } => {
                            if label.is_empty() {
                                "Back".to_string()
                            } else {
                                label.clone()
                            }
                        }
                    },
                    value_text: String::new(),
                    emphasis: 0.0,
                })
                .collect();

            PageDisplayModel {
                title: p.title.clone(),
                overlay_text: None,
                tiles,
            }
        })
        .unwrap_or_else(|| PageDisplayModel {
            title: "---".to_string(),
            overlay_text: None,
            tiles: Vec::new(),
        });

    DisplayFrame::Page(page)
}

fn render_display_frame(renderer: &renderer::Renderer, frame: DisplayFrame) -> image::DynamicImage {
    match frame {
        DisplayFrame::LegacySections(specs) => renderer.render(&specs),
        DisplayFrame::Page(page) => {
            let specs = convert_page_model_to_sections(&page);
            renderer.render(&specs)
        }
    }
}

fn convert_page_model_to_sections(page: &PageDisplayModel) -> Vec<SectionSpec> {
    let mut tiles = Vec::new();
    for tile in &page.tiles {
        let mut value_text = tile.value_text.clone();
        if value_text.is_empty() && page.overlay_text.is_some() {
            value_text = page.overlay_text.clone().unwrap_or_default();
        }

        let history = emphasis_to_history(tile.emphasis, HISTORY_LEN);
        tiles.push(SectionSpec {
            label: tile.label.clone(),
            value_text,
            history,
        });
    }

    if tiles.is_empty() {
        return vec![SectionSpec {
            label: page.title.clone(),
            value_text: page.overlay_text.clone().unwrap_or_default(),
            history: vec![0.0; HISTORY_LEN],
        }];
    }

    tiles
}

fn emphasis_to_history(emphasis: f32, len: usize) -> Vec<f32> {
    std::iter::repeat_n(emphasis.clamp(0.0, 1.0), len).collect()
}

/// 1回の入力ポーリングで発生した副作用の集約結果。
/// 呼び出し側はこのフラグを使ってデバイス反映を最小化する。
struct InputUpdateOutcome {
    brightness_changed: bool,
    notification_state_changed: bool,
    page_state_changed: bool,
}

/// ボタン入力の優先順位ポリシーに基づくルーティング結果。
enum ButtonRoutingDecision {
    Notification(NotificationPressAction),
    BrightnessReset,
    Noop,
}

/// エンコーダ入力のルーティング結果。
/// 現状は輝度エンコーダのみを受理する。
enum EncoderRoutingDecision {
    BrightnessDelta(i8),
    Noop,
}

/// 現在ページ上のボタン入力解釈結果。
enum PageButtonDecision {
    Navigate(String),
    Back,
    Command(Vec<String>),
    Noop,
}

/// 機能割り当てボタンを視認しやすくするための色。
enum AssignedButtonColor {
    Nav,
    Back,
    Command,
}

impl AssignedButtonColor {
    fn to_rgb(self) -> image::Rgb<u8> {
        match self {
            AssignedButtonColor::Nav => image::Rgb([0, 80, 255]),
            AssignedButtonColor::Back => image::Rgb([255, 120, 0]),
            AssignedButtonColor::Command => image::Rgb([0, 180, 40]),
        }
    }
}

/// 最小のページ状態機械。
/// Step2 で `current_page_id` と履歴管理を導入する。
struct PageState {
    current_page_id: String,
    history: Vec<String>,
}

impl PageState {
    fn new(config: &RuntimeConfig) -> Self {
        let current_page_id = fallback_page_id(config).unwrap_or_default();
        Self {
            current_page_id,
            history: Vec::new(),
        }
    }

    fn reconcile(&mut self, config: &RuntimeConfig) {
        if page_exists(config, &self.current_page_id) {
            return;
        }
        self.current_page_id = fallback_page_id(config).unwrap_or_default();
        self.history.clear();
    }

    fn navigate_to(&mut self, target: String) {
        self.history.push(self.current_page_id.clone());
        self.current_page_id = target;
    }

    fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.current_page_id = prev;
        }
    }
}

fn fallback_page_id(config: &RuntimeConfig) -> Option<String> {
    if page_exists(config, &config.home_page_id) {
        return Some(config.home_page_id.clone());
    }
    config.pages.first().map(|p| p.id.clone())
}

fn page_exists(config: &RuntimeConfig, page_id: &str) -> bool {
    config.pages.iter().any(|p| p.id == page_id)
}

fn current_page_items<'a>(
    config: &'a RuntimeConfig,
    page_state: &PageState,
) -> &'a [config::PageItemConfig] {
    config
        .pages
        .iter()
        .find(|p| p.id == page_state.current_page_id)
        .map(|p| p.items.as_slice())
        .unwrap_or(&[])
}

fn resolve_page_button_decision(
    config: &RuntimeConfig,
    page_state: &PageState,
    idx: u8,
) -> PageButtonDecision {
    let items = current_page_items(config, page_state);
    let Some(item) = items.get(idx as usize) else {
        return PageButtonDecision::Noop;
    };

    match item {
        config::PageItemConfig::Nav { target, .. } => {
            if page_exists(config, target) {
                PageButtonDecision::Navigate(target.clone())
            } else {
                PageButtonDecision::Noop
            }
        }
        config::PageItemConfig::Back { .. } => PageButtonDecision::Back,
        config::PageItemConfig::Command { command, .. } => {
            if command.is_empty() {
                PageButtonDecision::Noop
            } else {
                PageButtonDecision::Command(command.clone())
            }
        }
    }
}

fn assigned_button_color(
    config: &RuntimeConfig,
    page_state: &PageState,
    idx: usize,
) -> Option<AssignedButtonColor> {
    let items = current_page_items(config, page_state);
    let item = items.get(idx)?;
    match item {
        config::PageItemConfig::Nav { .. } => Some(AssignedButtonColor::Nav),
        config::PageItemConfig::Back { .. } => Some(AssignedButtonColor::Back),
        config::PageItemConfig::Command { .. } => Some(AssignedButtonColor::Command),
    }
}

fn resolve_encoder_twist_policy(idx: u8, delta: i8) -> EncoderRoutingDecision {
    if idx == ENCODER_BRIGHTNESS {
        EncoderRoutingDecision::BrightnessDelta(delta)
    } else {
        EncoderRoutingDecision::Noop
    }
}

fn resolve_button_down_policy(
    idx: u8,
    brightness: u8,
    notification_state: &mut NotificationState,
    now: Instant,
) -> ButtonRoutingDecision {
    // 0-5 ポリシー: 物理ボタン競合時は「通知操作」を最優先し、
    // 通知スロットとして使われていない場合のみ輝度リセットを扱う。
    if let Some(action) = notification_state.on_button_down(idx as usize, now) {
        return ButtonRoutingDecision::Notification(action);
    }

    if idx == BUTTON_BRIGHTNESS_RESET && brightness != DEFAULT_BRIGHTNESS {
        return ButtonRoutingDecision::BrightnessReset;
    }

    ButtonRoutingDecision::Noop
}

fn handle_device_updates(
    updates: Vec<DeviceStateUpdate>,
    brightness: &Rc<Cell<u8>>,
    notification_state: &Rc<RefCell<NotificationState>>,
    runtime_config: &RuntimeConfig,
    page_state: &mut PageState,
) -> InputUpdateOutcome {
    let mut brightness_changed = false;
    let mut notification_state_changed = false;
    let mut page_state_changed = false;

    for update in updates {
        match update {
            DeviceStateUpdate::EncoderTwist(idx, delta) => {
                if let EncoderRoutingDecision::BrightnessDelta(delta) =
                    resolve_encoder_twist_policy(idx, delta)
                {
                    let new_b = apply_brightness_delta(brightness.get(), delta);
                    if new_b != brightness.get() {
                        brightness.set(new_b);
                        brightness_changed = true;
                    }
                }
            }
            DeviceStateUpdate::ButtonDown(idx) => {
                let decision = {
                    let mut state = notification_state.borrow_mut();
                    resolve_button_down_policy(idx, brightness.get(), &mut state, Instant::now())
                };

                match decision {
                    ButtonRoutingDecision::Notification(action) => {
                        notification_state_changed = true;
                        if let NotificationPressAction::Execute(payload) = action {
                            if let Err(e) = execute_payload(&payload) {
                                warn!(%payload, "通知アクション実行失敗: {e:#}");
                            }
                        }
                    }
                    ButtonRoutingDecision::BrightnessReset => {
                        brightness.set(DEFAULT_BRIGHTNESS);
                        brightness_changed = true;
                    }
                    ButtonRoutingDecision::Noop => {
                        match resolve_page_button_decision(runtime_config, page_state, idx) {
                            PageButtonDecision::Navigate(target) => {
                                page_state.navigate_to(target);
                                page_state_changed = true;
                            }
                            PageButtonDecision::Back => {
                                let before = page_state.current_page_id.clone();
                                page_state.back();
                                page_state_changed = page_state.current_page_id != before;
                            }
                            PageButtonDecision::Command(command) => {
                                let action = ActionRequest::Command {
                                    program: command[0].clone(),
                                    args: command[1..].to_vec(),
                                };
                                if let Err(e) = execute_action(action) {
                                    warn!("ページ command 実行失敗: {e:#}");
                                }
                            }
                            PageButtonDecision::Noop => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    InputUpdateOutcome {
        brightness_changed,
        notification_state_changed,
        page_state_changed,
    }
}

fn poll_notification_state(
    source: &mut NotificationSource,
    state: &Rc<RefCell<NotificationState>>,
) -> anyhow::Result<bool> {
    let mut changed = false;
    if drain_notifications(source, &mut state.borrow_mut())? {
        changed = true;
    }
    if state.borrow_mut().expire_pending(Instant::now()) {
        changed = true;
    }
    Ok(changed)
}

// ── ヘルパー関数 ──────────────────────────────────────────────────

/// エンコーダのデルタを輝度に適用し、範囲内にクランプして返す
fn apply_brightness_delta(current: u8, delta: i8) -> u8 {
    (current as i16 + delta as i16 * BRIGHTNESS_STEP as i16)
        .clamp(BRIGHTNESS_MIN as i16, BRIGHTNESS_MAX as i16) as u8
}

#[allow(dead_code)]
/// 実行境界で扱うアクション要求。
/// 通知由来の open と将来の command/ssh-connect を同じ入口に載せる。
enum ActionRequest {
    OpenTarget(String),
    Command { program: String, args: Vec<String> },
    SshConnect { host: String, terminal: String },
}

fn execute_action(action: ActionRequest) -> anyhow::Result<()> {
    match action {
        ActionRequest::OpenTarget(target) => {
            let status = Command::new("xdg-open").arg(&target).status()?;
            if !status.success() {
                anyhow::bail!("xdg-open が失敗しました: status={status}");
            }
            Ok(())
        }
        ActionRequest::Command { program, args } => {
            if program.trim().is_empty() {
                anyhow::bail!("実行プログラム名が空です");
            }
            let status = Command::new(&program).args(args).status()?;
            if !status.success() {
                anyhow::bail!("コマンド実行が失敗しました: status={status}, program={program}");
            }
            Ok(())
        }
        ActionRequest::SshConnect { host, terminal } => {
            anyhow::bail!("SSH 接続アクションは未実装です: host={host}, terminal={terminal}")
        }
    }
}

fn execute_payload(payload: &str) -> anyhow::Result<()> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        anyhow::bail!("action_payload が空です");
    }
    execute_action(ActionRequest::OpenTarget(trimmed.to_owned()))
}

/// ボタン画像にラベルテキストを描画して返す
fn draw_button_with_label(label: &str, bg_color: image::Rgb<u8>) -> anyhow::Result<DynamicImage> {
    const BUTTON_SIZE: u32 = 120;
    const TEXT_COLOR: Rgb<u8> = Rgb([255, 255, 255]);

    let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, bg_color);

    if !label.trim().is_empty() {
        let font = FontVec::try_from_vec(BUTTON_FONT_DATA.to_vec())
            .map_err(|e| anyhow::anyhow!("ボタンフォントロード失敗: {e}"))?;
        let scale = PxScale::from(BUTTON_FONT_SIZE);

        let text_y = (BUTTON_SIZE as i32 - BUTTON_FONT_SIZE as i32) / 2;
        let text_x = 10_i32;

        draw_text_mut(&mut img, TEXT_COLOR, text_x, text_y, scale, &font, label);
    }

    Ok(DynamicImage::ImageRgb8(img))
}

/// ボタンにラベルテキスト画像を設定する
fn refresh_button_display(
    hw: &device::HardwareManager,
    state: &NotificationState,
    runtime_config: &RuntimeConfig,
    page_state: &PageState,
) -> anyhow::Result<()> {
    let items = current_page_items(runtime_config, page_state);

    for idx in 0..BUTTON_COUNT {
        if matches!(state.slots[idx], SlotState::Empty) {
            // 割り当てあり: ラベル画像を描画
            if let Some(color_kind) = assigned_button_color(runtime_config, page_state, idx) {
                let label = items
                    .get(idx)
                    .and_then(|item| match item {
                        config::PageItemConfig::Nav { label, .. } => Some(label.as_str()),
                        config::PageItemConfig::Command { label, .. } => Some(label.as_str()),
                        config::PageItemConfig::Back { label } => Some(label.as_str()),
                    })
                    .unwrap_or("");

                let bg_color = color_kind.to_rgb();
                let button_img = draw_button_with_label(label, bg_color)?;
                hw.set_button_image(idx as u8, button_img)?;
            } else {
                // 割り当てなし: 黒
                hw.set_button_color(idx as u8, image::Rgb([0, 0, 0]))?;
            }
        } else {
            // 通知スロット: 通知色
            hw.set_button_color(idx as u8, state.slots[idx].color().to_rgb())?;
        }
    }
    hw.flush_buttons()?;
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

/// 実行時に利用する統合設定。
/// Step1-4b では新設定モデルで検証・監視対象を解決し、
/// 表示セクションだけを legacy 設定からブリッジして保持する。
struct RuntimeConfig {
    sections: Vec<config::DashboardSectionConfig>,
    home_page_id: String,
    pages: Vec<config::PageConfig>,
    watch_targets: Vec<PathBuf>,
}

// ── ConfigManager ─────────────────────────────────────────────────

/// 設定のアクティブ値と監視状態を管理するコンポーネント。
/// ファイル変更通知を受けてデバウンス付きで設定を再読込する。
struct ConfigManager {
    config_path: String,
    active: RuntimeConfig,
    reload_err_count: u32,
    last_reload_at: Instant,
    watch_targets: Vec<PathBuf>,
    #[cfg(feature = "file-watch")]
    watcher: Option<FileWatcher>,
}

impl ConfigManager {
    fn new(config_path: &str, initial: RuntimeConfig) -> Self {
        let watch_targets = initial.watch_targets.clone();

        #[cfg(feature = "file-watch")]
        let watcher = create_file_watcher(&watch_targets);

        Self {
            config_path: config_path.to_owned(),
            active: initial,
            reload_err_count: 0,
            last_reload_at: Instant::now(),
            watch_targets,
            #[cfg(feature = "file-watch")]
            watcher,
        }
    }

    fn current(&self) -> &RuntimeConfig {
        &self.active
    }

    fn poll_reload(&mut self) -> bool {
        #[cfg(feature = "file-watch")]
        {
            let has_signal = self.watcher.as_ref().is_some_and(|w| w.has_pending());
            if has_signal && self.last_reload_at.elapsed() >= RELOAD_DEBOUNCE {
                return self.do_reload();
            }
        }
        false
    }

    pub(crate) fn do_reload(&mut self) -> bool {
        self.last_reload_at = Instant::now();
        match load_runtime_config(&self.config_path) {
            Ok(new_cfg) => {
                let new_watch_targets = new_cfg.watch_targets.clone();
                self.active = new_cfg;
                self.reload_err_count = 0;

                #[cfg(feature = "file-watch")]
                if self.watch_targets != new_watch_targets {
                    info!(path = %self.config_path, "監視対象ファイルが変更されたため監視を再初期化します");
                    self.watcher = create_file_watcher(&new_watch_targets);
                }

                self.watch_targets = new_watch_targets;
                info!(path = %self.config_path, "設定ファイルをリロードしました");
                true
            }
            Err(e) => {
                self.reload_err_count += 1;
                warn!(
                    path = %self.config_path,
                    err_count = self.reload_err_count,
                    "設定ファイルのリロードに失敗（旧設定を維持）: {e:#}"
                );
                false
            }
        }
    }
}

// ── FileWatcher ──────────────────────────────────────────────────

#[cfg(feature = "file-watch")]
fn create_file_watcher(targets: &[PathBuf]) -> Option<FileWatcher> {
    match FileWatcher::new(targets) {
        Ok(w) => {
            info!(targets = ?targets, "設定ファイル監視を開始");
            Some(w)
        }
        Err(e) => {
            warn!("設定ファイル監視の初期化に失敗（無効化）: {e:#}");
            None
        }
    }
}

#[cfg(feature = "file-watch")]
struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<()>,
}

#[cfg(feature = "file-watch")]
impl FileWatcher {
    fn new(targets: &[PathBuf]) -> anyhow::Result<Self> {
        if targets.is_empty() {
            anyhow::bail!("監視対象が空です");
        }

        let (tx, rx) = mpsc::channel::<()>();
        let target_set: BTreeSet<PathBuf> = targets.iter().cloned().collect();
        let watch_dirs: BTreeSet<PathBuf> = targets
            .iter()
            .map(|target| {
                target.parent().map(Path::to_path_buf).ok_or_else(|| {
                    anyhow::anyhow!(
                        "監視対象の親ディレクトリが取得できません: {}",
                        target.display()
                    )
                })
            })
            .collect::<anyhow::Result<_>>()?;

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let relevant = event.paths.is_empty()
                        || event
                            .paths
                            .iter()
                            .map(|p| normalize_path(p))
                            .any(|p| target_set.contains(&p));
                    if relevant {
                        let _ = tx.send(());
                    }
                }
            })?;

        for dir in watch_dirs {
            watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        }

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    fn has_pending(&self) -> bool {
        let mut any = false;
        while self.receiver.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

// ── テスト ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 値域確認: apply_brightness_delta は範囲外をクランプし、ステップを正しく適用する
    #[test]
    fn test_brightness_adjustment() {
        let cases: &[(u8, i8, u8)] = &[
            (70, 1, 75),
            (70, -1, 65),
            (100, 1, 100),
            (5, -1, 5),
            (70, 0, 70),
            (70, 2, 80),
        ];
        for &(current, delta, expected) in cases {
            let got = apply_brightness_delta(current, delta);
            assert_eq!(got, expected, "current={current} delta={delta}");
        }
    }

    /// 正常系: brightness_changed フラグ: 同値では変化しない
    #[test]
    fn test_brightness_no_change_at_limits() {
        assert_eq!(apply_brightness_delta(BRIGHTNESS_MAX, 1), BRIGHTNESS_MAX);
        assert_eq!(apply_brightness_delta(BRIGHTNESS_MIN, -1), BRIGHTNESS_MIN);
    }

    #[test]
    fn test_convert_page_model_to_sections_fallback_when_tiles_empty() {
        let page = PageDisplayModel {
            title: "Home".to_string(),
            overlay_text: Some("overlay".to_string()),
            tiles: vec![],
        };

        let sections = convert_page_model_to_sections(&page);
        assert_eq!(
            sections.len(),
            1,
            "空ページは1セクションへフォールバックする"
        );
        assert_eq!(sections[0].label, "Home");
        assert_eq!(sections[0].value_text, "overlay");
        assert_eq!(sections[0].history.len(), HISTORY_LEN);
    }

    #[test]
    fn test_emphasis_to_history_clamps_range() {
        let cases = [(-1.0_f32, 0.0_f32), (0.5_f32, 0.5_f32), (2.0_f32, 1.0_f32)];
        for (input, expected) in cases {
            let history = emphasis_to_history(input, 4);
            assert_eq!(history.len(), 4);
            for value in history {
                assert!(
                    (value - expected).abs() < 1e-6,
                    "input={input} value={value}"
                );
            }
        }
    }

    #[test]
    fn test_button_policy_prioritizes_notification_over_brightness_reset() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        state.slots[BUTTON_BRIGHTNESS_RESET as usize] = SlotState::Unread {
            item: NotificationItem {
                id: 1,
                summary: "s".to_string(),
                body: "b".to_string(),
                action_payload: "https://example.com".to_string(),
                created_at: Instant::now(),
            },
        };

        let decision = resolve_button_down_policy(
            BUTTON_BRIGHTNESS_RESET,
            DEFAULT_BRIGHTNESS + 5,
            &mut state,
            Instant::now(),
        );

        assert!(matches!(
            decision,
            ButtonRoutingDecision::Notification(NotificationPressAction::Execute(_))
        ));
    }

    #[test]
    fn test_button_policy_uses_brightness_reset_when_no_notification() {
        let mut state = NotificationState::new(SlotCompactionMode::KeepGap);
        let decision = resolve_button_down_policy(
            BUTTON_BRIGHTNESS_RESET,
            DEFAULT_BRIGHTNESS + 10,
            &mut state,
            Instant::now(),
        );

        assert!(matches!(decision, ButtonRoutingDecision::BrightnessReset));
    }

    #[test]
    fn test_encoder_policy_ignores_non_brightness_encoder() {
        let decision = resolve_encoder_twist_policy(0, 1);
        assert!(matches!(decision, EncoderRoutingDecision::Noop));
    }
}
