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
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ab_glyph::{FontVec, PxScale};
use clap::{Parser, Subcommand};
use dbus::arg::RefArg;
use dbus::blocking::{Connection, Proxy};
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

/// DBus ポーリング間隔 (Python 側の 500ms ポーリングに合わせる)
const DBUS_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
const DBUS_SERVICE_ENV: &str = "STREAM_DECK_DBUS_SERVICE";
const DBUS_PATH_ENV: &str = "STREAM_DECK_DBUS_PATH";
const DBUS_INTERFACE_ENV: &str = "STREAM_DECK_DBUS_INTERFACE";
const DBUS_PROPERTY_ENV: &str = "STREAM_DECK_DBUS_PROPERTY";

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

    Ok(RuntimeConfig {
        sections: bundle.app.dashboard.sections,
        home_page_id: bundle.app.app.home,
        pages,
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
    let mut window_context_provider = build_window_context_provider();
    let mut ssh_registry = SshSessionRegistry::new();

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

        // LCD は常にメトリクス表示を維持する。
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
                &mut ssh_registry,
            );
            let brightness_changed = input_outcome.brightness_changed;
            notification_state_changed |= input_outcome.notification_state_changed;
            let page_state_changed = input_outcome.page_state_changed;
            let context_state_changed = apply_context_auto_transition(
                config_manager.current(),
                &mut page_state,
                window_context_provider.as_mut(),
            );

            if notification_state_changed || page_state_changed || context_state_changed {
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

/// エンコーダの責務割り当て。
/// Step 3-6 ではページ操作を未接続とし、右端のみ輝度操作に固定する。
enum EncoderRole {
    ReservedNoop,
    Brightness,
}

/// 現在ページ上のボタン入力解釈結果。
#[derive(Clone)]
enum PageButtonDecision {
    Navigate(String),
    Back,
    Command(Vec<String>),
    SshConnect {
        host: String,
        terminal: Vec<String>,
        ssh_template: String,
    },
    Noop,
}

/// 機能割り当てボタンを視認しやすくするための色。
#[derive(Clone, Copy)]
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

    /// 自動コンテキスト遷移は履歴へ push せず現在ページのみ差し替える。
    fn set_context_page(&mut self, target: String) {
        self.current_page_id = target;
    }
}

/// Step 7: アクティブウィンドウ情報から SSH ホスト文脈を得る境界。
/// 現段階は no-op 実装で、DBus 実装を後続で差し替える。
trait WindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String>;
}

struct NoopWindowContextProvider;

impl WindowContextProvider for NoopWindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String> {
        None
    }
}

/// 環境差分を吸収するため、取得元 DBus プロパティは環境変数で指定する。
/// プロパティ値に `rs-common:ssh:<host>` が含まれる場合に `<host>` を抽出する。
struct DbusWindowContextProvider {
    connection: Connection,
    service: String,
    path: String,
    interface: String,
    property: String,
    last_host: Option<String>,
    next_poll: Instant,
}

impl DbusWindowContextProvider {
    fn from_env() -> anyhow::Result<Option<Self>> {
        let service = std::env::var(DBUS_SERVICE_ENV).ok();
        let path = std::env::var(DBUS_PATH_ENV).ok();
        let interface = std::env::var(DBUS_INTERFACE_ENV).ok();
        let property = std::env::var(DBUS_PROPERTY_ENV).ok();

        let Some(service) = service else {
            return Ok(None);
        };
        let Some(path) = path else {
            return Ok(None);
        };
        let Some(interface) = interface else {
            return Ok(None);
        };
        let Some(property) = property else {
            return Ok(None);
        };

        let connection = Connection::new_session()
            .map_err(|e| anyhow::anyhow!("DBus セッションバス接続に失敗しました: {e}"))?;

        Ok(Some(Self {
            connection,
            service,
            path,
            interface,
            property,
            last_host: None,
            next_poll: Instant::now(),
        }))
    }

    fn proxy(&self) -> Proxy<'_, &Connection> {
        self.connection.with_proxy(
            self.service.as_str(),
            self.path.as_str(),
            Duration::from_millis(80),
        )
    }

    fn fetch_context_text(&self) -> anyhow::Result<String> {
        let proxy = self.proxy();
        let (value,): (dbus::arg::Variant<Box<dyn RefArg + 'static>>,) = proxy
            .method_call(
                "org.freedesktop.DBus.Properties",
                "Get",
                (self.interface.as_str(), self.property.as_str()),
            )
            .map_err(|e| anyhow::anyhow!("DBus Properties.Get 呼び出しに失敗しました: {e}"))?;

        refarg_to_string(value.0.as_ref())
            .ok_or_else(|| anyhow::anyhow!("DBus プロパティ値を文字列へ変換できませんでした"))
    }
}

impl WindowContextProvider for DbusWindowContextProvider {
    fn poll_active_ssh_host(&mut self) -> Option<String> {
        // ポーリング間隔に達していない場合は DBus 呼び出しをスキップ
        if Instant::now() < self.next_poll {
            return None;
        }
        self.next_poll = Instant::now() + DBUS_POLL_INTERVAL;

        let text = match self.fetch_context_text() {
            Ok(text) => text,
            Err(e) => {
                debug!(err = %e, "DBus コンテキスト取得に失敗");
                return None;
            }
        };

        let host = parse_ssh_host_from_context(&text);

        // 非 SSH ウィンドウに切り替わったら last_host をリセット（次の復帰を検出できるようにする）
        if host.is_none() {
            self.last_host = None;
            return None;
        }

        let host = host.unwrap();
        if self.last_host.as_ref() == Some(&host) {
            return None;
        }

        self.last_host = Some(host.clone());
        Some(host)
    }
}

fn refarg_to_string(value: &dyn RefArg) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(i) = value.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = value.as_u64() {
        return Some(u.to_string());
    }
    None
}

fn parse_ssh_host_from_context(text: &str) -> Option<String> {
    let marker = "rs-common:ssh:";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '\u{7}' || c == '\u{1b}')
        .unwrap_or(rest.len());
    let host = rest[..end].trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn build_window_context_provider() -> Box<dyn WindowContextProvider> {
    match DbusWindowContextProvider::from_env() {
        Ok(Some(provider)) => {
            info!(
                service = %provider.service,
                path = %provider.path,
                interface = %provider.interface,
                property = %provider.property,
                "DBus WindowContextProvider を有効化"
            );
            Box::new(provider)
        }
        Ok(None) => {
            info!(
                "DBus WindowContextProvider は無効（環境変数未設定）: {} {} {} {}",
                DBUS_SERVICE_ENV, DBUS_PATH_ENV, DBUS_INTERFACE_ENV, DBUS_PROPERTY_ENV
            );
            Box::new(NoopWindowContextProvider)
        }
        Err(e) => {
            warn!(err = %e, "DBus WindowContextProvider 初期化失敗のため no-op で継続");
            Box::new(NoopWindowContextProvider)
        }
    }
}

fn resolve_ssh_host_target_page(config: &RuntimeConfig, host: &str) -> Option<String> {
    for page in &config.pages {
        for item in &page.items {
            if let config::PageItemConfig::SshConnect {
                host: item_host, ..
            } = item
            {
                if item_host == host {
                    return Some(page.id.clone());
                }
            }
        }
    }
    None
}

fn apply_context_auto_transition(
    config: &RuntimeConfig,
    page_state: &mut PageState,
    provider: &mut dyn WindowContextProvider,
) -> bool {
    let Some(host) = provider.poll_active_ssh_host() else {
        return false;
    };

    let Some(target_page) = resolve_ssh_host_target_page(config, &host) else {
        debug!(host = %host, "自動遷移対象外のホスト");
        return false;
    };

    if page_state.current_page_id == target_page {
        return false;
    }

    let from = page_state.current_page_id.clone();
    page_state.set_context_page(target_page.clone());
    info!(host = %host, from = %from, to = %target_page, "コンテキスト自動遷移");
    true
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

/// ボタン 1 つ分の解決済み割当。
/// 入力処理と描画処理の両方で同じ結果を参照して、表示と動作の不一致を防ぐ。
struct ResolvedButtonAssignment {
    label: String,
    color: AssignedButtonColor,
    decision: PageButtonDecision,
}

const DEFAULT_ITEM_PRIORITY: i32 = 0;

fn page_item_priority(item: &config::PageItemConfig) -> i32 {
    match item {
        config::PageItemConfig::Nav { priority, .. }
        | config::PageItemConfig::Command { priority, .. }
        | config::PageItemConfig::Back { priority, .. }
        | config::PageItemConfig::SshConnect { priority, .. } => {
            priority.unwrap_or(DEFAULT_ITEM_PRIORITY)
        }
    }
}

fn page_item_label(item: &config::PageItemConfig) -> String {
    match item {
        config::PageItemConfig::Nav { label, .. } => label.clone(),
        config::PageItemConfig::Command { label, .. } => label.clone(),
        config::PageItemConfig::SshConnect { label, .. } => label.clone(),
        config::PageItemConfig::Back { label, .. } => {
            if label.is_empty() {
                "Back".to_string()
            } else {
                label.clone()
            }
        }
    }
}

fn resolve_button_assignments(
    config: &RuntimeConfig,
    page_state: &PageState,
) -> Vec<ResolvedButtonAssignment> {
    let mut indexed_items: Vec<(usize, &config::PageItemConfig)> =
        current_page_items(config, page_state)
            .iter()
            .enumerate()
            .collect();

    // priority 昇順 + 設定記述順で安定化して、先頭からボタンへ割り当てる。
    indexed_items.sort_by_key(|(source_idx, item)| (page_item_priority(item), *source_idx));

    indexed_items
        .into_iter()
        .take(BUTTON_COUNT)
        .map(|(_source_idx, item)| match item {
            config::PageItemConfig::Nav { target, .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Nav,
                decision: if page_exists(config, target) {
                    PageButtonDecision::Navigate(target.clone())
                } else {
                    PageButtonDecision::Noop
                },
            },
            config::PageItemConfig::Back { .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Back,
                decision: PageButtonDecision::Back,
            },
            config::PageItemConfig::Command { command, .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Command,
                decision: if command.is_empty() {
                    PageButtonDecision::Noop
                } else {
                    PageButtonDecision::Command(command.clone())
                },
            },
            config::PageItemConfig::SshConnect {
                host,
                terminal,
                ssh_template,
                ..
            } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Command,
                decision: PageButtonDecision::SshConnect {
                    host: host.clone(),
                    terminal: terminal.clone(),
                    ssh_template: ssh_template.clone(),
                },
            },
        })
        .collect()
}

fn resolve_page_button_decision(
    config: &RuntimeConfig,
    page_state: &PageState,
    idx: u8,
) -> PageButtonDecision {
    resolve_button_assignments(config, page_state)
        .get(idx as usize)
        .map(|assignment| assignment.decision.clone())
        .unwrap_or(PageButtonDecision::Noop)
}

fn page_decision_name(decision: &PageButtonDecision) -> &'static str {
    match decision {
        PageButtonDecision::Navigate(_) => "navigate",
        PageButtonDecision::Back => "back",
        PageButtonDecision::Command(_) => "command",
        PageButtonDecision::SshConnect { .. } => "ssh-connect",
        PageButtonDecision::Noop => "noop",
    }
}

fn resolve_encoder_role(idx: u8) -> Option<EncoderRole> {
    match idx {
        ENCODER_BRIGHTNESS => Some(EncoderRole::Brightness),
        0..=2 => Some(EncoderRole::ReservedNoop),
        _ => None,
    }
}

fn resolve_encoder_twist_policy(idx: u8, delta: i8) -> EncoderRoutingDecision {
    match resolve_encoder_role(idx) {
        Some(EncoderRole::Brightness) => EncoderRoutingDecision::BrightnessDelta(delta),
        Some(EncoderRole::ReservedNoop) | None => EncoderRoutingDecision::Noop,
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
    ssh_registry: &mut SshSessionRegistry,
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
                        debug!(button = idx, "通知ボタン入力を処理");
                        notification_state_changed = true;
                        if let NotificationPressAction::Execute(payload) = action {
                            if let Err(e) = execute_payload(&payload) {
                                warn!(%payload, "通知アクション実行失敗: {e:#}");
                            }
                        }
                    }
                    ButtonRoutingDecision::BrightnessReset => {
                        info!(button = idx, "輝度リセットを実行");
                        brightness.set(DEFAULT_BRIGHTNESS);
                        brightness_changed = true;
                    }
                    ButtonRoutingDecision::Noop => {
                        let page_decision =
                            resolve_page_button_decision(runtime_config, page_state, idx);
                        debug!(
                            button = idx,
                            page = %page_state.current_page_id,
                            decision = page_decision_name(&page_decision),
                            "ページ入力を解決"
                        );

                        match page_decision {
                            PageButtonDecision::Navigate(target) => {
                                let from = page_state.current_page_id.clone();
                                page_state.navigate_to(target);
                                page_state_changed = true;
                                info!(
                                    button = idx,
                                    from = %from,
                                    to = %page_state.current_page_id,
                                    "ページ遷移"
                                );
                            }
                            PageButtonDecision::Back => {
                                let before = page_state.current_page_id.clone();
                                page_state.back();
                                page_state_changed = page_state.current_page_id != before;
                                info!(
                                    button = idx,
                                    from = %before,
                                    to = %page_state.current_page_id,
                                    changed = page_state_changed,
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

/// 実行境界で扱うアクション要求。
/// 通知由来の open と将来の command を同じ入口に載せる。
/// SSH 接続は SshSessionRegistry 経由で実行するため、ここには含まない。
enum ActionRequest {
    OpenTarget(String),
    Command {
        program: String,
        args: Vec<String>,
    },
}

fn resolve_default_terminal_program() -> String {
    let output = Command::new("readlink")
        .arg("-e")
        .arg("/usr/bin/x-terminal-emulator")
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let candidate = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !candidate.is_empty() {
                return candidate;
            }
        }
    }

    "/usr/bin/x-terminal-emulator".to_string()
}

fn shell_escape_single_quoted(raw: &str) -> String {
    raw.replace('\'', "'\"'\"'")
}

fn build_ssh_shell_command(host: &str, ssh_template: &str, title: &str) -> String {
    let ssh_command = ssh_template.replace("{host}", host);
    let escaped_title = shell_escape_single_quoted(title);
    format!(
        "printf '\\033]0;{}\\007'; exec {}",
        escaped_title, ssh_command
    )
}

fn compose_terminal_launch(
    terminal: &[String],
    shell_command: &str,
    title: &str,
) -> anyhow::Result<(String, Vec<String>)> {
    let (program, mut args) = if terminal.is_empty() {
        (resolve_default_terminal_program(), Vec::new())
    } else {
        (terminal[0].clone(), terminal[1..].to_vec())
    };

    if program.trim().is_empty() {
        anyhow::bail!("terminal command の program が空です");
    }

    let bin_name = Path::new(&program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match bin_name {
        "xterm" => {
            args.push("-T".to_string());
            args.push(title.to_string());
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        // Debian/Ubuntu alternatives entry. Many implementations (e.g. terminator)
        // reject "-- bash -lc ..." but accept "-e ...".
        "x-terminal-emulator" => {
            args.push("-T".to_string());
            args.push(title.to_string());
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        "konsole" => {
            args.push("-p".to_string());
            args.push(format!("tabtitle={title}"));
            args.push("-e".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
        _ => {
            args.push("--title".to_string());
            args.push(title.to_string());
            args.push("--".to_string());
            args.push("bash".to_string());
            args.push("-lc".to_string());
            args.push(shell_command.to_string());
        }
    }

    Ok((program, args))
}

/// host -> 起動済みプロセス ID のレジストリ。セッション再利用とウィンドウフォーカスを管理する。
struct SshSessionRegistry {
    sessions: HashMap<String, u32>,
}

impl SshSessionRegistry {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// `/proc/<pid>` の存在でプロセス生存を確認する。
    fn is_alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    /// ウィンドウタイトルで既存セッションへのフォーカスを試みる。
    /// `wmctrl -a` → `xdotool search --name windowactivate` の順にフォールバックする。
    fn try_focus(title: &str) -> bool {
        if Command::new("wmctrl")
            .args(["-a", title])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        Command::new("xdotool")
            .args(["search", "--name", title, "windowactivate"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// 既存セッションがあればフォーカス、なければ新規起動する。
    fn connect(
        &mut self,
        host: &str,
        terminal: &[String],
        ssh_template: &str,
    ) -> anyhow::Result<()> {
        let session_title = format!("rs-common:ssh:{host}");

        if let Some(&pid) = self.sessions.get(host) {
            if Self::is_alive(pid) {
                if Self::try_focus(&session_title) {
                    info!(host, pid, "既存 SSH ウィンドウへフォーカスしました");
                } else {
                    info!(host, pid, "既存 SSH セッションを再利用します（フォーカスは未保証）");
                }
                return Ok(());
            }
            // PID が死んでいる場合はレジストリから削除して新規起動へ
            self.sessions.remove(host);
        }

        let shell_command = build_ssh_shell_command(host, ssh_template, &session_title);
        let (program, args) = compose_terminal_launch(terminal, &shell_command, &session_title)?;
        let child = Command::new(&program).args(&args).spawn().map_err(|e| {
            anyhow::anyhow!("SSH 接続用端末の起動に失敗しました program={program}: {e}")
        })?;

        let pid = child.id();
        self.sessions.insert(host.to_string(), pid);
        info!(host, pid, program = %program, "SSH セッションを新規起動しました");
        Ok(())
    }
}

fn execute_action(action: ActionRequest) -> anyhow::Result<()> {
    match action {
        ActionRequest::OpenTarget(target) => {
            info!(target = %target, "open アクションを実行");
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
            info!(program = %program, args = ?args, "command アクションを実行");
            let child = Command::new(&program).args(args).spawn()?;
            info!(program = %program, pid = child.id(), "command を非同期起動しました");
            Ok(())
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
    let assignments = resolve_button_assignments(runtime_config, page_state);

    for idx in 0..BUTTON_COUNT {
        if matches!(state.slots[idx], SlotState::Empty) {
            // 割り当てあり: ラベル画像を描画
            if let Some(assignment) = assignments.get(idx) {
                let bg_color = assignment.color.to_rgb();
                let button_img = draw_button_with_label(&assignment.label, bg_color)?;
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
            watch_targets: vec![],
        }
    }

    fn runtime_with_pages(pages: Vec<config::PageConfig>) -> RuntimeConfig {
        RuntimeConfig {
            sections: vec![],
            home_page_id: "home".to_string(),
            pages,
            watch_targets: vec![],
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

    #[test]
    fn test_encoder_policy_accepts_brightness_encoder_only() {
        let cases = [
            (0_u8, false),
            (1_u8, false),
            (2_u8, false),
            (ENCODER_BRIGHTNESS, true),
            (9_u8, false),
        ];

        for (idx, should_brightness) in cases {
            let decision = resolve_encoder_twist_policy(idx, 1);
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
        assert!(cmd.contains("exec ssh user@example"));
        assert!(cmd.contains("printf"));
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
            host: Some("ubuntu@10.0.0.99".to_string()),
        };

        let changed = apply_context_auto_transition(&config, &mut page_state, &mut provider);

        assert!(!changed);
        assert_eq!(page_state.current_page_id, "home");
    }

    #[test]
    fn test_parse_ssh_host_from_context_extracts_marker_value() {
        let text = "active=rs-common:ssh:ubuntu@10.149.39.52 title=terminal";
        let host = parse_ssh_host_from_context(text);
        assert_eq!(host.as_deref(), Some("ubuntu@10.149.39.52"));
    }

    #[test]
    fn test_parse_ssh_host_from_context_returns_none_without_marker() {
        let text = "active=Terminal title=home";
        let host = parse_ssh_host_from_context(text);
        assert_eq!(host, None);
    }
}
