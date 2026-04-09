//! Stream Deck マルチメトリクスモニター
//!
//! サブコマンド:
//! * (なし)    — 4セクション LCD ダッシュボードを表示し続ける
//! * `list`    — 接続中の Stream Deck デバイスを一覧表示する
//! * `diagnose` — 接続環境を段階的に診断する

mod action;
mod button;
mod cmd;
mod config;
mod context;
mod device;
mod display;
mod error;
mod metrics;
mod notifications;
mod paging;
mod plugin;
mod plugin_contract;
mod podman;
mod renderer;
mod runtime;
mod section;
mod state;
mod storage;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use elgato_streamdeck::DeviceStateUpdate;
use signal_hook::consts::{SIGINT, SIGTERM};
use tracing::{debug, error, info, warn};

use action::{execute_action, execute_payload, ActionRequest};
use button::{
    find_page_nav_target, page_decision_name, refresh_button_display, resolve_button_assignments,
    resolve_button_down_policy, resolve_encoder_twist_policy, ButtonRoutingDecision,
    EncoderRoutingDecision, NavDirection, PageButtonDecision,
};
use context::{apply_context_auto_transition, build_window_context_provider};
use display::section_patterns;
use metrics::MetricsSource;
use notifications::{
    NotificationPressAction, NotificationSource, NotificationState, SlotCompactionMode,
};
use plugin::{BrightnessPlugin, CpuPlugin, DataPlugin, LoadPlugin, MemPlugin, NotifPlugin};
use renderer::SectionSpec;
use runtime::{ConfigManager, RuntimeConfig};
use section::Section;
use state::{
    InputUpdateOutcome, PageState, Phase0ActionResultKind, Phase0ActionRuntime, SshSessionRegistry,
};
use storage::MemoryStorage;

#[cfg(test)]
use notifications::{NotificationItem, SlotState};

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

/// 通知受信用の UNIX ドメインソケット
const NOTIFICATION_SOCKET_PATH: &str = "/tmp/rs-common-stream-deck-notify.sock";

/// デフォルト設定ファイル
const DEFAULT_LAYOUT_CONFIG_PATH: &str = "crates/stream-deck/config/layout.toml";

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

    // 設定再読込をまたいで履歴を保持するストレージ（ライフタイムを config_manager より長く保つ）
    let section_storage: Rc<RefCell<MemoryStorage>> =
        Rc::new(RefCell::new(MemoryStorage::new(HISTORY_LEN)));

    let mut sections = build_sections(
        config_manager.current(),
        &sys_source,
        &brightness,
        &notification_state,
        &section_storage,
    );

    let shutdown_flag = install_shutdown_flag()?;

    let mut notification_source = NotificationSource::bind(NOTIFICATION_SOCKET_PATH)?;

    loop {
        if should_shutdown(&shutdown_flag) {
            info!("終了シグナルを受信したためモニターを終了します");
            break;
        }

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
                    &section_storage,
                    &shutdown_flag,
                ) {
                    warn!("デバイスエラー: {e:#} — 再接続します");
                }
            }
            Err(e) => {
                warn!("接続失敗: {e:#} — {RETRY_INTERVAL:?} 後に再試行します");
            }
        }

        if should_shutdown(&shutdown_flag) {
            break;
        }
        std::thread::sleep(RETRY_INTERVAL);
    }

    Ok(())
}

pub fn load_runtime_config(config_path: &str) -> anyhow::Result<RuntimeConfig> {
    let bundle = config::AppConfigBundle::load(config_path, terminal_title_capable())?;
    let display = display::dto::DisplayConfigDto::try_from(&bundle.app.display)
        .map_err(|e| anyhow::anyhow!("display DTO 変換に失敗しました: {e}"))?;
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

    let mut pages = bundle.app.pages.clone();
    if bundle.report.ssh_enabled {
        if let (Some(input), Some(hosts)) = (
            config::build_ssh_page_build_input(&bundle.app),
            bundle.ssh_hosts.as_deref(),
        ) {
            let generated = config::build_dynamic_ssh_pages(&input, hosts);
            if !generated.is_empty() {
                info!(count = generated.len(), "動的 SSH ページを生成しました");
                pages.extend(generated);
            }
        }
    }

    if bundle.report.podman_enabled {
        if let Some(podman_cfg) = bundle.app.dynamic.podman.as_ref() {
            if let Some(input) = config::build_podman_page_build_input(podman_cfg) {
                let socket_path = config::resolve_podman_socket_path(podman_cfg);
                let client = podman::PodmanClient::new(&socket_path);
                match client.load_entries() {
                    Ok(entries) => {
                        let containers: Vec<config::PodmanContainerEntry> = entries
                            .iter()
                            .map(|entry| config::PodmanContainerEntry {
                                id: entry.id.clone(),
                                label: entry.display_label(),
                                state: entry.state.clone(),
                            })
                            .collect();
                        let generated = config::build_dynamic_podman_pages(&input, &containers);
                        if !generated.is_empty() {
                            info!(
                                socket = %socket_path,
                                count = generated.len(),
                                containers = containers.len(),
                                "動的 Podman ページを生成しました"
                            );
                            pages.extend(generated);
                        }
                    }
                    Err(e) => {
                        warn!(
                            socket = %socket_path,
                            err = %e,
                            "Podman コンテナ一覧の取得に失敗したため Podman ページ生成をスキップ"
                        );
                    }
                }
            }
        }
    }

    Ok(RuntimeConfig {
        sections: bundle.app.dashboard.sections,
        home_page_id: bundle.app.app.home,
        pages,
        actions: bundle.app.actions,
        display,
        watch_targets: bundle.watch_targets,
        podman: bundle.app.dynamic.podman,
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
    storage: &Rc<RefCell<MemoryStorage>>,
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
            Section::new(plugin, sec.capacity, Rc::clone(storage))
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
    section_storage: &Rc<RefCell<MemoryStorage>>,
    shutdown_flag: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let reader = hw.get_reader();
    hw.set_brightness(brightness.get())?;
    let mut page_state = PageState::new(config_manager.current());
    let mut window_context_provider = build_window_context_provider();
    let mut ssh_registry = SshSessionRegistry::default();
    let mut phase0_actions = Phase0ActionRuntime::from_config(config_manager.current());
    let mut page_nav_overlay = paging::PageNavOverlay::new();
    let mut podman_cpu_history: HashMap<String, VecDeque<f32>> = HashMap::new();
    let mut podman_last_polled = Instant::now() - Duration::from_secs(60);
    let mut podman_last_list_polled = Instant::now() - Duration::from_secs(60);

    hw.clear_buttons()?;
    refresh_button_display(
        hw,
        &notification_state.borrow(),
        config_manager.current(),
        &page_state,
        &podman_cpu_history,
        &phase0_actions,
    )?;

    loop {
        if should_shutdown(shutdown_flag) {
            render_exit_display(hw, renderer, config_manager.current())?;
            return Ok(());
        }

        if config_manager.poll_reload() {
            info!("レイアウトをリロードしました");
            *sections = build_sections(
                config_manager.current(),
                sys_source,
                brightness,
                notification_state,
                section_storage,
            );
            page_state.reconcile(config_manager.current());
            phase0_actions = Phase0ActionRuntime::from_config(config_manager.current());
            refresh_button_display(
                hw,
                &notification_state.borrow(),
                config_manager.current(),
                &page_state,
                &podman_cpu_history,
                &phase0_actions,
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
                &podman_cpu_history,
                &phase0_actions,
            )?;
        }

        if apply_context_auto_transition(
            config_manager.current(),
            &mut page_state,
            window_context_provider.as_mut(),
        ) {
            refresh_button_display(
                hw,
                &notification_state.borrow(),
                config_manager.current(),
                &page_state,
                &podman_cpu_history,
                &phase0_actions,
            )?;
        }

        // メトリクスを一度だけ refresh してから各セクションを更新
        sys_source.borrow_mut().refresh();
        for section in sections.iter_mut() {
            section.tick();
        }

        let podman_history_changed = update_podman_cpu_history(
            config_manager.current(),
            &mut podman_cpu_history,
            &mut podman_last_polled,
        );
        let podman_list_changed = update_podman_container_list(
            config_manager,
            &mut podman_last_list_polled,
            &mut page_state,
        );
        if podman_history_changed || podman_list_changed {
            refresh_button_display(
                hw,
                &notification_state.borrow(),
                config_manager.current(),
                &page_state,
                &podman_cpu_history,
                &phase0_actions,
            )?;
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

        // LCD は常にメトリクス表示を維持する。
        // ページ遷移操作直後 2 秒間だけ左端セクションをページ位置情報で上書きする。
        render_lcd_display(
            hw,
            renderer,
            sections,
            &page_nav_overlay,
            config_manager.current(),
            &page_state,
            &phase0_actions,
        )?;

        // 次のTickまで入力をポーリングする (INPUT_POLL_INTERVAL 刻み)
        let deadline = Instant::now() + TICK_INTERVAL;
        loop {
            if should_shutdown(shutdown_flag) {
                render_exit_display(hw, renderer, config_manager.current())?;
                return Ok(());
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let poll_timeout = INPUT_POLL_INTERVAL.min(remaining);

            let updates = match reader.read(Some(poll_timeout)) {
                Ok(updates) => updates,
                Err(e) => {
                    if should_shutdown(shutdown_flag) {
                        info!("終了シグナル受信中のため Exit 画面を表示して終了します");
                        render_exit_display(hw, renderer, config_manager.current())?;
                        return Ok(());
                    }
                    return Err(anyhow::anyhow!("入力読み取りエラー: {e}"));
                }
            };

            let mut notification_state_changed =
                poll_notification_state(notification_source, notification_state)?;
            let input_outcome = handle_device_updates(
                updates,
                brightness,
                notification_state,
                config_manager.current(),
                &mut page_state,
                &mut ssh_registry,
                &mut phase0_actions,
                &mut page_nav_overlay,
            );
            let brightness_changed = input_outcome.brightness_changed;
            notification_state_changed |= input_outcome.notification_state_changed;
            let page_state_changed = input_outcome.page_state_changed;
            let context_state_changed = apply_context_auto_transition(
                config_manager.current(),
                &mut page_state,
                window_context_provider.as_mut(),
            );

            if notification_state_changed
                || page_state_changed
                || input_outcome.action_state_changed
                || context_state_changed
            {
                refresh_button_display(
                    hw,
                    &notification_state.borrow(),
                    config_manager.current(),
                    &page_state,
                    &podman_cpu_history,
                    &phase0_actions,
                )?;
            }

            if brightness_changed {
                debug!(bright = brightness.get(), "輝度変更");
                if let Err(e) = hw.set_brightness(brightness.get()) {
                    error!("輝度設定エラー: {e:#}");
                    return Err(anyhow::anyhow!("{e}"));
                }
            }

            // エンコーダ操作によるページ遷移時は LCD を即座に更新
            if input_outcome.lcd_needs_update {
                render_lcd_display(
                    hw,
                    renderer,
                    sections,
                    &page_nav_overlay,
                    config_manager.current(),
                    &page_state,
                    &phase0_actions,
                )?;
            }
        }
    }
}

/// 終了時に LCD へ明示的な終了画面を表示する。
/// デフォルトでは全セクションに "exit" と表示する。
fn render_exit_display(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    config: &RuntimeConfig,
) -> anyhow::Result<()> {
    let section_count = config.sections.len().max(1);
    let specs: Vec<SectionSpec> = (0..section_count)
        .map(|_| SectionSpec {
            label: "exit".to_string(),
            value_text: String::new(),
            history: vec![0.0; HISTORY_LEN],
        })
        .collect();
    let image = renderer.render(&specs);
    hw.set_lcd_strip_image(image)?;
    Ok(())
}

/// 終了シグナル検出用のフラグを初期化し、SIGINT/SIGTERM に紐づける。
fn install_shutdown_flag() -> anyhow::Result<Arc<AtomicBool>> {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&shutdown_flag))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&shutdown_flag))?;
    Ok(shutdown_flag)
}

/// 終了シグナルの受信状態を返す。
fn should_shutdown(shutdown_flag: &Arc<AtomicBool>) -> bool {
    shutdown_flag.load(Ordering::Relaxed)
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
struct PageDisplayModel {
    title: String,
    overlay_text: Option<String>,
    tiles: Vec<PageTile>,
}

#[allow(dead_code)]
/// ページ内の1タイル分の表示情報。
struct PageTile {
    label: String,
    value_text: String,
    emphasis: f32,
}

fn build_legacy_display_frame(sections: &[Section]) -> DisplayFrame {
    DisplayFrame::LegacySections(sections.iter().map(Section::as_spec).collect())
}

fn build_page_display_frame(config: &RuntimeConfig, page_state: &PageState) -> DisplayFrame {
    let assignments = resolve_button_assignments(config, page_state);

    let title = config
        .pages
        .iter()
        .find(|p| p.id == page_state.current_page_id)
        .map(|p| p.title.clone())
        .unwrap_or_else(|| "---".to_string());

    let tiles: Vec<PageTile> = assignments
        .into_iter()
        .map(|assignment| PageTile {
            label: assignment.label,
            value_text: String::new(),
            emphasis: 0.0,
        })
        .collect();

    let page = PageDisplayModel {
        title,
        overlay_text: None,
        tiles,
    };

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

fn handle_device_updates(
    updates: Vec<DeviceStateUpdate>,
    brightness: &Rc<Cell<u8>>,
    notification_state: &Rc<RefCell<NotificationState>>,
    runtime_config: &RuntimeConfig,
    page_state: &mut PageState,
    ssh_registry: &mut SshSessionRegistry,
    phase0_actions: &mut Phase0ActionRuntime,
    page_nav_overlay: &mut paging::PageNavOverlay,
) -> InputUpdateOutcome {
    let mut outcome = InputUpdateOutcome::default();

    for update in updates {
        match update {
            DeviceStateUpdate::EncoderTwist(idx, delta) => {
                let decision = resolve_encoder_twist_policy(idx, delta, runtime_config, page_state);
                match decision {
                    EncoderRoutingDecision::BrightnessDelta(d) => {
                        let new_b = apply_brightness_delta(brightness.get(), d);
                        if new_b != brightness.get() {
                            brightness.set(new_b);
                            outcome.brightness_changed = true;
                        }
                    }
                    EncoderRoutingDecision::PageNavigate(dir) => {
                        let nav_label = match dir {
                            NavDirection::Next => "Next",
                            NavDirection::Prev => "Prev",
                        };
                        if let Some(target) =
                            find_page_nav_target(runtime_config, page_state, nav_label)
                        {
                            let target = target.to_string();
                            info!(encoder = idx, dir = ?dir, to = %target, "エンコーダでページ遷移（グループ内ページング）");
                            page_state.set_context_page(target);
                            outcome.page_state_changed = true;
                            outcome.lcd_needs_update = true;
                            page_nav_overlay.trigger();
                        }
                    }
                    EncoderRoutingDecision::SampleAction { action_id } => {
                        let apply_outcome = phase0_actions.apply(&action_id);
                        if apply_outcome.result == Phase0ActionResultKind::Success {
                            info!(
                                encoder = idx,
                                delta,
                                action_id = %action_id,
                                feedback = %apply_outcome.feedback,
                                "左端ノブで section sample action を適用"
                            );
                            outcome.action_state_changed = true;
                            outcome.lcd_needs_update = true;
                        } else {
                            warn!(
                                encoder = idx,
                                delta,
                                action_id = %action_id,
                                "左端ノブの section sample action が reject されました"
                            );
                        }
                    }
                    EncoderRoutingDecision::Noop => {}
                }
            }
            DeviceStateUpdate::ButtonDown(idx) => {
                let decision = {
                    let mut state = notification_state.borrow_mut();
                    resolve_button_down_policy(
                        idx,
                        brightness.get(),
                        &mut state,
                        Instant::now(),
                        DEFAULT_BRIGHTNESS,
                    )
                };

                match decision {
                    ButtonRoutingDecision::Notification(action) => {
                        debug!(button = idx, "通知ボタン入力を処理");
                        outcome.notification_state_changed = true;
                        if let NotificationPressAction::Execute(payload) = action {
                            if let Err(e) = execute_payload(&payload) {
                                warn!(%payload, "通知アクション実行失敗: {e:#}");
                            }
                        }
                    }
                    ButtonRoutingDecision::BrightnessReset => {
                        info!(button = idx, "輝度リセットを実行");
                        brightness.set(DEFAULT_BRIGHTNESS);
                        outcome.brightness_changed = true;
                    }
                    ButtonRoutingDecision::Noop => {
                        let assignments = resolve_button_assignments(runtime_config, page_state);
                        let assignment = assignments.get(idx as usize);
                        let is_paging_nav = assignment
                            .map(|a| matches!(a.label.as_str(), "Prev" | "Next"))
                            .unwrap_or(false);
                        let page_decision = assignment
                            .map(|a| a.decision.clone())
                            .unwrap_or(PageButtonDecision::Noop);
                        debug!(
                            button = idx,
                            page = %page_state.current_page_id,
                            decision = page_decision_name(&page_decision),
                            "ページ入力を解決"
                        );

                        match page_decision {
                            PageButtonDecision::Navigate(target) => {
                                let from = page_state.current_page_id.clone();
                                if is_paging_nav {
                                    page_state.set_context_page(target);
                                    page_nav_overlay.trigger();
                                } else {
                                    page_state.navigate_to(target);
                                }
                                outcome.page_state_changed = true;
                                info!(
                                    button = idx,
                                    from = %from,
                                    to = %page_state.current_page_id,
                                    is_paging = is_paging_nav,
                                    "ページ遷移"
                                );
                            }
                            PageButtonDecision::Back => {
                                let before = page_state.current_page_id.clone();
                                page_state.back();
                                outcome.page_state_changed = page_state.current_page_id != before;
                                info!(
                                    button = idx,
                                    from = %before,
                                    to = %page_state.current_page_id,
                                    changed = outcome.page_state_changed,
                                    "戻る操作"
                                );
                            }
                            PageButtonDecision::Command(command) => {
                                info!(button = idx, command = ?command, "ページ command を実行");
                                let action = ActionRequest::Command {
                                    program: command[0].clone(),
                                    args: command[1..].to_vec(),
                                };
                                if let Err(e) = execute_action(action) {
                                    warn!("ページ command 実行失敗: {e:#}");
                                }
                            }
                            PageButtonDecision::SshConnect {
                                host,
                                terminal,
                                ssh_template,
                            } => {
                                info!(
                                    button = idx,
                                    host = %host,
                                    terminal = ?terminal,
                                    ssh_template = %ssh_template,
                                    "ページ ssh-connect を実行"
                                );
                                if let Err(e) =
                                    ssh_registry.connect(&host, &terminal, &ssh_template)
                                {
                                    warn!("ページ ssh-connect 実行失敗: {e:#}");
                                }
                            }
                            PageButtonDecision::PodmanLogs {
                                container_id,
                                terminal,
                                log_command,
                            } => {
                                info!(
                                    button = idx,
                                    container_id = %container_id,
                                    "Podman logs を起動"
                                );
                                let (program, mut cmd_args) = if terminal.is_empty() {
                                    (action::resolve_default_terminal_program(), Vec::new())
                                } else {
                                    (terminal[0].clone(), terminal[1..].to_vec())
                                };
                                cmd_args.extend_from_slice(&log_command);
                                cmd_args.push(container_id.clone());
                                let action = ActionRequest::PodmanLogs {
                                    container_id,
                                    program,
                                    args: cmd_args,
                                };
                                if let Err(e) = execute_action(action) {
                                    warn!("Podman logs 起動失敗: {e:#}");
                                }
                            }
                            PageButtonDecision::SampleAction { action_id } => {
                                let apply_outcome = phase0_actions.apply(&action_id);
                                if apply_outcome.result == Phase0ActionResultKind::Success {
                                    info!(
                                        button = idx,
                                        action_id = %action_id,
                                        feedback = %apply_outcome.feedback,
                                        "Phase0 sample action を適用"
                                    );
                                    outcome.action_state_changed = true;
                                } else {
                                    warn!(
                                        button = idx,
                                        action_id = %action_id,
                                        "Phase0 sample action が reject されました"
                                    );
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

    outcome
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

fn apply_brightness_delta(current: u8, delta: i8) -> u8 {
    (current as i16 + delta as i16 * BRIGHTNESS_STEP as i16)
        .clamp(BRIGHTNESS_MIN as i16, BRIGHTNESS_MAX as i16) as u8
}

/// LCD 表示をレンダリングして送信する共通ヘルパー。
fn render_lcd_display(
    hw: &device::HardwareManager,
    renderer: &renderer::Renderer,
    sections: &[Section],
    page_nav_overlay: &paging::PageNavOverlay,
    config: &RuntimeConfig,
    page_state: &PageState,
    phase0_actions: &Phase0ActionRuntime,
) -> anyhow::Result<()> {
    let mut specs: Vec<SectionSpec> = sections.iter().map(Section::as_spec).collect();
    section_patterns::apply_section_pattern_overrides(
        &mut specs,
        config,
        page_state,
        phase0_actions,
    );
    if page_nav_overlay.is_active(Instant::now()) {
        let title = config
            .pages
            .iter()
            .find(|p| p.id == page_state.current_page_id)
            .map(|p| p.title.as_str())
            .unwrap_or("");
        if let Some(overlay) = paging::PageNavOverlay::build_section_spec(title, HISTORY_LEN) {
            if let Some(first) = specs.first_mut() {
                *first = overlay;
            }
        }
    }
    let image = renderer.render(&specs);
    hw.set_lcd_strip_image(image)?;
    Ok(())
}

fn update_podman_cpu_history(
    config: &RuntimeConfig,
    histories: &mut HashMap<String, VecDeque<f32>>,
    last_polled: &mut Instant,
) -> bool {
    let Some(podman_cfg) = config.podman.as_ref() else {
        if histories.is_empty() {
            return false;
        }
        histories.clear();
        return true;
    };

    if !podman_cfg.enabled {
        if histories.is_empty() {
            return false;
        }
        histories.clear();
        return true;
    }

    let poll_interval = Duration::from_millis(podman_cfg.poll_interval_ms.max(100));
    if last_polled.elapsed() < poll_interval {
        return false;
    }
    *last_polled = Instant::now();

    let tracked_ids: BTreeSet<String> = config
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|item| {
            if let config::PageItemConfig::PodmanMonitor { container_id, .. } = item {
                Some(container_id.clone())
            } else {
                None
            }
        })
        .collect();

    let before_len = histories.len();
    histories.retain(|id, _| tracked_ids.contains(id));
    let mut changed = histories.len() != before_len;

    if tracked_ids.is_empty() {
        return changed;
    }

    let socket_path = config::resolve_podman_socket_path(podman_cfg);
    let client = podman::PodmanClient::new(&socket_path);
    let stats = match client.load_stats() {
        Ok(stats) => stats,
        Err(e) => {
            warn!(socket = %socket_path, err = %e, "Podman stats 取得失敗");
            return changed;
        }
    };

    let mut running_ids = BTreeSet::new();
    for stat in stats {
        if !tracked_ids.contains(&stat.id) {
            continue;
        }
        running_ids.insert(stat.id.clone());
        let history = histories.entry(stat.id).or_default();
        history.push_back(stat.cpu_percent);
        while history.len() > podman_cfg.cpu_history_len {
            history.pop_front();
        }
        changed = true;
    }

    for id in tracked_ids {
        if !running_ids.contains(&id) && histories.remove(&id).is_some() {
            changed = true;
        }
    }

    changed
}

/// Podman コンテナ一覧を再取得し、ページを再生成する。
fn update_podman_container_list(
    config_manager: &mut ConfigManager,
    last_polled: &mut Instant,
    page_state: &mut PageState,
) -> bool {
    let (podman_cfg_enabled, podman_cfg_clone, prefix) = {
        let config = config_manager.current();
        let Some(podman_cfg) = config.podman.as_ref() else {
            return false;
        };

        if !podman_cfg.enabled {
            return false;
        }

        (
            podman_cfg.enabled,
            podman_cfg.clone(),
            podman_cfg.page_id_prefix.clone(),
        )
    };

    if !podman_cfg_enabled {
        return false;
    }

    let poll_interval =
        Duration::from_millis(podman_cfg_clone.container_list_poll_interval_ms.max(1000));
    if last_polled.elapsed() < poll_interval {
        return false;
    }
    *last_polled = Instant::now();

    let socket_path = config::resolve_podman_socket_path(&podman_cfg_clone);
    let client = podman::PodmanClient::new(&socket_path);
    let entries = match client.load_entries() {
        Ok(entries) => entries,
        Err(e) => {
            warn!(socket = %socket_path, err = %e, "Podman コンテナ一覧の再取得に失敗");
            return false;
        }
    };

    let Some(input) = config::build_podman_page_build_input(&podman_cfg_clone) else {
        return false;
    };

    let containers: Vec<config::PodmanContainerEntry> = entries
        .iter()
        .map(|entry| config::PodmanContainerEntry {
            id: entry.id.clone(),
            label: entry.display_label(),
            state: entry.state.clone(),
        })
        .collect();

    let new_podman_pages = config::build_dynamic_podman_pages(&input, &containers);
    let before_count = config_manager.current().pages.len();
    config_manager.update_podman_pages(&prefix, new_podman_pages);

    if !containers.is_empty() {
        info!(
            socket = %socket_path,
            old_count = before_count,
            new_count = config_manager.current().pages.len(),
            containers = containers.len(),
            "Podman コンテナ一覧を更新してページを再生成しました"
        );
    }

    page_state.reconcile(config_manager.current());
    true
}

// ── テスト ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{build_ssh_shell_command, compose_terminal_launch};
    use crate::button::{
        resolve_button_assignments, resolve_button_down_policy, resolve_encoder_twist_policy,
        ButtonRoutingDecision, EncoderRoutingDecision, BUTTON_BRIGHTNESS_RESET, ENCODER_BRIGHTNESS,
    };
    use crate::context::{
        apply_context_auto_transition, resolve_ssh_host_target_page, WindowContextProvider,
    };
    use crate::state::PageState;

    struct TestWindowContextProvider {
        host: Option<String>,
    }

    impl WindowContextProvider for TestWindowContextProvider {
        fn poll_active_ssh_host(&mut self) -> Option<String> {
            self.host.take()
        }
    }

    fn runtime_with_items(items: Vec<config::PageItemConfig>) -> RuntimeConfig {
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages: vec![config::PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items,
            }],
            actions: vec![],
            display: display::dto::DisplayConfigDto::default(),
            watch_targets: vec![],
            podman: None,
        }
    }

    fn runtime_with_pages(pages: Vec<config::PageConfig>) -> RuntimeConfig {
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages,
            actions: vec![],
            display: display::dto::DisplayConfigDto::default(),
            watch_targets: vec![],
            podman: None,
        }
    }

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
            DEFAULT_BRIGHTNESS,
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
            DEFAULT_BRIGHTNESS,
        );

        assert!(matches!(decision, ButtonRoutingDecision::BrightnessReset));
    }

    /// 値域確認: Prev/Next なしのページで encoder 0 は Noop
    #[test]
    fn test_encoder_policy_ignores_non_brightness_encoder() {
        let config = runtime_with_items(vec![]);
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };
        let decision = resolve_encoder_twist_policy(0, 1, &config, &page_state);
        assert!(matches!(decision, EncoderRoutingDecision::Noop));
    }

    #[test]
    fn test_encoder_policy_maps_left_knob_to_section_sample_action() {
        let config = RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages: vec![config::PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items: vec![config::PageItemConfig::Sample {
                    sample_id: "sec_mode".to_string(),
                    label: "SecMode".to_string(),
                    action_ref: Some("act_sec_mode".to_string()),
                    priority: Some(10),
                }],
            }],
            actions: vec![],
            display: display::dto::DisplayConfigDto {
                samples: vec![display::dto::DisplaySampleDto {
                    id: "sec_mode".to_string(),
                    target: display::dto::DisplayTargetDto::Section,
                    payload: display::dto::DisplayPayloadDto::LabelValue {
                        title: "Sec".to_string(),
                        value: "0".to_string(),
                        unit: "%".to_string(),
                        severity: config::DisplaySeverity::Normal,
                    },
                }],
            },
            watch_targets: vec![],
            podman: None,
        };
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };

        let decision = resolve_encoder_twist_policy(0, 1, &config, &page_state);
        assert!(matches!(
            decision,
            EncoderRoutingDecision::SampleAction { action_id } if action_id == "act_sec_mode"
        ));
    }

    /// 正常系: encoder 3 (輝度) は Prev/Next の有無によらず BrightnessDelta を返す
    #[test]
    fn test_encoder_policy_accepts_brightness_encoder_only() {
        let config = runtime_with_items(vec![]);
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };
        let cases = [
            (1_u8, false),
            (2_u8, false),
            (ENCODER_BRIGHTNESS, true),
            (9_u8, false),
        ];

        for (idx, should_brightness) in cases {
            let decision = resolve_encoder_twist_policy(idx, 1, &config, &page_state);
            if should_brightness {
                assert!(matches!(
                    decision,
                    EncoderRoutingDecision::BrightnessDelta(1)
                ));
            } else {
                assert!(matches!(decision, EncoderRoutingDecision::Noop));
            }
        }
    }

    #[test]
    fn test_resolve_button_assignments_keeps_source_order_when_priority_absent() {
        let cfg = runtime_with_items(vec![
            config::PageItemConfig::Nav {
                label: "Apps".to_string(),
                target: "home".to_string(),
                priority: None,
                visible: true,
            },
            config::PageItemConfig::Command {
                label: "Terminal".to_string(),
                command: vec!["/usr/bin/true".to_string()],
                priority: None,
            },
            config::PageItemConfig::Back {
                label: "Back".to_string(),
                priority: None,
            },
        ]);
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };

        let assignments = resolve_button_assignments(&cfg, &page_state);
        let labels: Vec<&str> = assignments.iter().map(|a| a.label.as_str()).collect();

        assert_eq!(labels, vec!["Apps", "Terminal", "Back"]);
    }

    #[test]
    fn test_resolve_button_assignments_sorts_by_priority_then_source_order() {
        let cfg = runtime_with_items(vec![
            config::PageItemConfig::Command {
                label: "Late".to_string(),
                command: vec!["/usr/bin/true".to_string()],
                priority: Some(20),
            },
            config::PageItemConfig::Back {
                label: "Back".to_string(),
                priority: Some(-20),
            },
            config::PageItemConfig::Nav {
                label: "Apps".to_string(),
                target: "home".to_string(),
                priority: None,
                visible: true,
            },
            config::PageItemConfig::Command {
                label: "MidA".to_string(),
                command: vec!["/usr/bin/true".to_string()],
                priority: Some(10),
            },
            config::PageItemConfig::Command {
                label: "MidB".to_string(),
                command: vec!["/usr/bin/true".to_string()],
                priority: Some(10),
            },
        ]);
        let page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };

        let assignments = resolve_button_assignments(&cfg, &page_state);
        let labels: Vec<&str> = assignments.iter().map(|a| a.label.as_str()).collect();

        assert_eq!(labels, vec!["Back", "Apps", "MidA", "MidB", "Late"]);
    }

    #[test]
    fn test_build_ssh_shell_command_replaces_host_placeholder() {
        let cmd = build_ssh_shell_command("user@example", "ssh {host}", "title");
        // host は shell_quote で囲まれるため 'user@example' となる
        assert!(cmd.contains("exec ssh 'user@example'"), "cmd={cmd}");
        assert!(cmd.contains("printf"));
    }

    #[test]
    fn test_build_ssh_shell_command_quotes_host_with_special_chars() {
        // セミコロンを含むホスト名がシェルインジェクションにならないことを確認する
        let cmd = build_ssh_shell_command("host;echo hacked", "ssh {host}", "title");
        assert!(cmd.contains("'host;echo hacked'"), "cmd={cmd}");
        assert!(!cmd.contains("; echo hacked"), "cmd={cmd}");
    }

    #[test]
    fn test_compose_terminal_launch_falls_back_to_default_terminal() {
        let (program, args) =
            compose_terminal_launch(&[], "echo hi", "rs-common:ssh:test").expect("compose");
        assert!(!program.trim().is_empty());
        assert!(args.len() >= 4);
    }

    #[test]
    fn test_compose_terminal_launch_for_x_terminal_emulator_uses_e_form() {
        let terminal = vec!["/usr/bin/x-terminal-emulator".to_string()];
        let (_, args) =
            compose_terminal_launch(&terminal, "echo hi", "rs-common:ssh:test").expect("ok");

        assert!(args.iter().any(|a| a == "-e"));
        assert!(!args.iter().any(|a| a == "--"));
    }

    #[test]
    fn test_resolve_ssh_host_target_page_finds_matching_page() {
        let config = runtime_with_pages(vec![
            config::PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items: vec![config::PageItemConfig::Nav {
                    label: "SSH".to_string(),
                    target: "ssh_hosts".to_string(),
                    priority: None,
                    visible: true,
                }],
            },
            config::PageConfig {
                id: "ssh_hosts".to_string(),
                title: "SSH".to_string(),
                items: vec![config::PageItemConfig::SshConnect {
                    label: "VM01".to_string(),
                    host: "ubuntu@10.0.0.10".to_string(),
                    terminal: vec![],
                    ssh_template: "ssh {host}".to_string(),
                    priority: None,
                }],
            },
        ]);

        let page = resolve_ssh_host_target_page(&config, "ubuntu@10.0.0.10");
        assert_eq!(page.as_deref(), Some("ssh_hosts"));
    }

    #[test]
    fn test_apply_context_auto_transition_switches_page_when_host_is_known() {
        let config = runtime_with_pages(vec![
            config::PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items: vec![],
            },
            config::PageConfig {
                id: "ssh_hosts".to_string(),
                title: "SSH".to_string(),
                items: vec![config::PageItemConfig::SshConnect {
                    label: "VM01".to_string(),
                    host: "ubuntu@10.0.0.10".to_string(),
                    terminal: vec![],
                    ssh_template: "ssh {host}".to_string(),
                    priority: None,
                }],
            },
        ]);
        let mut page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec!["apps".to_string()],
        };
        let mut provider = TestWindowContextProvider {
            host: Some("ubuntu@10.0.0.10".to_string()),
        };

        let changed = apply_context_auto_transition(&config, &mut page_state, &mut provider);

        assert!(changed);
        assert_eq!(page_state.current_page_id, "ssh_hosts");
        assert_eq!(page_state.history, vec!["apps".to_string()]);
    }

    #[test]
    fn test_apply_context_auto_transition_ignores_unknown_host() {
        let config = runtime_with_pages(vec![config::PageConfig {
            id: "home".to_string(),
            title: "Home".to_string(),
            items: vec![],
        }]);
        let mut page_state = PageState {
            current_page_id: "home".to_string(),
            history: vec![],
        };
        let mut provider = TestWindowContextProvider {
            host: Some("unknown@10.0.0.99".to_string()),
        };

        let changed = apply_context_auto_transition(&config, &mut page_state, &mut provider);

        assert!(!changed);
        assert_eq!(page_state.current_page_id, "home");
    }
}
