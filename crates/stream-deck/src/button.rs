//! ボタン・エンコーダの割り当て解決と描画モジュール。

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use ab_glyph::{FontVec, PxScale};
use image::{DynamicImage, Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::config;
use crate::device;
use crate::display::{button_patterns, catalog};
use crate::notifications::{NotificationPressAction, NotificationState, SlotState};
use crate::runtime::RuntimeConfig;
use crate::state::PageState;

/// Stream Deck+ 物理ボタン数
pub const BUTTON_COUNT: usize = 8;

/// 輝度を制御する右端エンコーダのインデックス (0-3、Plus は 4 個)
pub const ENCODER_BRIGHTNESS: u8 = 3;

/// 輝度をデフォルトにリセットする右下ボタンのインデックス (0-7、2行×4列)
pub const BUTTON_BRIGHTNESS_RESET: u8 = 7;

/// デフォルト優先度
const DEFAULT_ITEM_PRIORITY: i32 = 0;

/// ボタン画像用フォント (NotoSans-Regular, 120x120 ボタン用)
const BUTTON_FONT_SIZE: f32 = 18.0;
const BUTTON_FONT_DATA: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");

/// ボタン描画用フォントキャッシュ。初回呼び出し時にのみパースされる。
static BUTTON_FONT: OnceLock<FontVec> = OnceLock::new();

fn get_button_font() -> &'static FontVec {
    BUTTON_FONT.get_or_init(|| {
        FontVec::try_from_vec(BUTTON_FONT_DATA.to_vec()).expect("ボタンフォントロード失敗")
    })
}

/// 現在ページ上のボタン入力解釈結果。
#[derive(Clone)]
pub enum PageButtonDecision {
    Navigate(String),
    Back,
    Command(Vec<String>),
    SshConnect {
        host: String,
        terminal: Vec<String>,
        ssh_template: String,
    },
    /// Podman コンテナへの logs 表示アクション。
    PodmanLogs {
        container_id: String,
        terminal: Vec<String>,
        log_command: Vec<String>,
    },
    Noop,
}

/// 機能割り当てボタンを視認しやすくするための色。
#[derive(Clone, Copy)]
pub enum AssignedButtonColor {
    Nav,
    Back,
    Command,
    Podman([u8; 3]),
}

impl AssignedButtonColor {
    pub fn to_rgb(self) -> image::Rgb<u8> {
        match self {
            AssignedButtonColor::Nav => image::Rgb([0, 80, 255]),
            AssignedButtonColor::Back => image::Rgb([255, 120, 0]),
            AssignedButtonColor::Command => image::Rgb([0, 180, 40]),
            AssignedButtonColor::Podman(rgb) => image::Rgb(rgb),
        }
    }
}

/// ボタン入力の優先順位ポリシーに基づくルーティング結果。
pub enum ButtonRoutingDecision {
    Notification(NotificationPressAction),
    BrightnessReset,
    Noop,
}

/// エンコーダ入力のルーティング結果。
pub enum EncoderRoutingDecision {
    BrightnessDelta(i8),
    PageNavigate(NavDirection),
    Noop,
}

/// エンコーダの責務割り当て。
pub enum EncoderRole {
    ReservedNoop,
    Brightness,
    /// ページスクロール: エンコーダ 0（左端）がページ遷移を担う。
    PageScroll,
}

/// ページ遷移方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavDirection {
    Prev,
    Next,
}

/// ボタン 1 つ分の解決済み割当。
pub struct ResolvedButtonAssignment {
    pub label: String,
    pub color: AssignedButtonColor,
    pub decision: PageButtonDecision,
    pub sample_id: Option<String>,
    pub podman_is_running: bool,
}

pub fn podman_color_from_state(config: &RuntimeConfig, state: &str) -> AssignedButtonColor {
    let colors = config
        .podman
        .as_ref()
        .map(|cfg| cfg.colors.clone())
        .unwrap_or_default();
    match state.to_ascii_lowercase().as_str() {
        "running" => AssignedButtonColor::Podman(colors.running),
        "exited" | "stopped" => AssignedButtonColor::Podman(colors.exited),
        "paused" => AssignedButtonColor::Podman(colors.paused),
        _ => AssignedButtonColor::Podman(colors.other),
    }
}

pub fn current_page_items<'a>(
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

fn page_item_priority(item: &config::PageItemConfig) -> i32 {
    match item {
        config::PageItemConfig::Nav { priority, .. }
        | config::PageItemConfig::Command { priority, .. }
        | config::PageItemConfig::Back { priority, .. }
        | config::PageItemConfig::SshConnect { priority, .. }
        | config::PageItemConfig::PodmanMonitor { priority, .. }
        | config::PageItemConfig::Sample { priority, .. } => {
            priority.unwrap_or(DEFAULT_ITEM_PRIORITY)
        }
    }
}

pub fn page_item_label(item: &config::PageItemConfig) -> String {
    match item {
        config::PageItemConfig::Nav { label, .. } => label.clone(),
        config::PageItemConfig::Command { label, .. } => label.clone(),
        config::PageItemConfig::SshConnect { label, .. } => label.clone(),
        config::PageItemConfig::PodmanMonitor { label, .. } => label.clone(),
        config::PageItemConfig::Sample {
            label, sample_id, ..
        } => {
            if label.is_empty() {
                sample_id.clone()
            } else {
                label.clone()
            }
        }
        config::PageItemConfig::Back { label, .. } => {
            if label.is_empty() {
                "Back".to_string()
            } else {
                label.clone()
            }
        }
    }
}

fn runtime_podman_log_launch(config: &RuntimeConfig) -> (Vec<String>, Vec<String>) {
    if let Some(podman_cfg) = config.podman.as_ref() {
        return (podman_cfg.terminal.clone(), podman_cfg.log_command.clone());
    }
    (
        Vec::new(),
        vec!["podman".to_string(), "logs".to_string(), "-f".to_string()],
    )
}

pub fn resolve_button_assignments(
    config: &RuntimeConfig,
    page_state: &PageState,
) -> Vec<ResolvedButtonAssignment> {
    let mut indexed_items: Vec<(usize, &config::PageItemConfig)> =
        current_page_items(config, page_state)
            .iter()
            .enumerate()
            .collect();

    // visible=false の Nav アイテム（Prev/Next など）はボタン割り当てから除外する。
    indexed_items
        .retain(|(_, item)| !matches!(item, config::PageItemConfig::Nav { visible: false, .. }));

    // priority 昇順 + 設定記述順で安定化して、先頭からボタンへ割り当てる。
    indexed_items.sort_by_key(|(source_idx, item)| (page_item_priority(item), *source_idx));

    indexed_items
        .into_iter()
        .take(BUTTON_COUNT)
        .map(|(_source_idx, item)| match item {
            config::PageItemConfig::Nav { target, .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Nav,
                decision: if crate::runtime::page_exists(config, target) {
                    PageButtonDecision::Navigate(target.clone())
                } else {
                    PageButtonDecision::Noop
                },
                sample_id: None,
                podman_is_running: false,
            },
            config::PageItemConfig::Back { .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Back,
                decision: PageButtonDecision::Back,
                sample_id: None,
                podman_is_running: false,
            },
            config::PageItemConfig::Command { command, .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Command,
                decision: if command.is_empty() {
                    PageButtonDecision::Noop
                } else {
                    PageButtonDecision::Command(command.clone())
                },
                sample_id: None,
                podman_is_running: false,
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
                sample_id: None,
                podman_is_running: false,
            },
            config::PageItemConfig::PodmanMonitor {
                container_id,
                state,
                ..
            } => {
                let (terminal, log_command) = runtime_podman_log_launch(config);
                ResolvedButtonAssignment {
                    label: page_item_label(item),
                    color: podman_color_from_state(config, state),
                    decision: PageButtonDecision::PodmanLogs {
                        container_id: container_id.clone(),
                        terminal,
                        log_command,
                    },
                    sample_id: None,
                    podman_is_running: state.eq_ignore_ascii_case("running"),
                }
            }
            config::PageItemConfig::Sample { sample_id, .. } => ResolvedButtonAssignment {
                label: page_item_label(item),
                color: AssignedButtonColor::Command,
                decision: PageButtonDecision::Noop,
                sample_id: Some(sample_id.clone()),
                podman_is_running: false,
            },
        })
        .collect()
}

pub fn resolve_page_button_decision(
    config: &RuntimeConfig,
    page_state: &PageState,
    idx: u8,
) -> PageButtonDecision {
    resolve_button_assignments(config, page_state)
        .get(idx as usize)
        .map(|assignment| assignment.decision.clone())
        .unwrap_or(PageButtonDecision::Noop)
}

pub fn page_decision_name(decision: &PageButtonDecision) -> &'static str {
    match decision {
        PageButtonDecision::Navigate(_) => "navigate",
        PageButtonDecision::Back => "back",
        PageButtonDecision::Command(_) => "command",
        PageButtonDecision::SshConnect { .. } => "ssh-connect",
        PageButtonDecision::PodmanLogs { .. } => "podman-logs",
        PageButtonDecision::Noop => "noop",
    }
}

/// コンテキスト対応のエンコーダ責務割り当て。
pub fn resolve_encoder_role_ctx(
    idx: u8,
    config: &RuntimeConfig,
    page_state: &PageState,
) -> Option<EncoderRole> {
    match idx {
        ENCODER_BRIGHTNESS => Some(EncoderRole::Brightness),
        0 => {
            let has_paging = current_page_items(config, page_state).iter().any(|item| {
                matches!(
                    item,
                    config::PageItemConfig::Nav { label, .. }
                    if label == "Prev" || label == "Next"
                )
            });
            if has_paging {
                Some(EncoderRole::PageScroll)
            } else {
                Some(EncoderRole::ReservedNoop)
            }
        }
        1..=2 => Some(EncoderRole::ReservedNoop),
        _ => None,
    }
}

pub fn resolve_encoder_twist_policy(
    idx: u8,
    delta: i8,
    config: &RuntimeConfig,
    page_state: &PageState,
) -> EncoderRoutingDecision {
    match resolve_encoder_role_ctx(idx, config, page_state) {
        Some(EncoderRole::Brightness) => EncoderRoutingDecision::BrightnessDelta(delta),
        Some(EncoderRole::PageScroll) => {
            if delta > 0 {
                EncoderRoutingDecision::PageNavigate(NavDirection::Next)
            } else if delta < 0 {
                EncoderRoutingDecision::PageNavigate(NavDirection::Prev)
            } else {
                EncoderRoutingDecision::Noop
            }
        }
        Some(EncoderRole::ReservedNoop) | None => EncoderRoutingDecision::Noop,
    }
}

/// 現在ページの Nav アイテムから指定ラベルのページ遷移先 ID を返す。
pub fn find_page_nav_target<'a>(
    config: &'a RuntimeConfig,
    page_state: &PageState,
    label: &str,
) -> Option<&'a str> {
    current_page_items(config, page_state)
        .iter()
        .find_map(|item| {
            if let config::PageItemConfig::Nav {
                label: item_label,
                target,
                ..
            } = item
            {
                if item_label == label {
                    return Some(target.as_str());
                }
            }
            None
        })
}

pub fn resolve_button_down_policy(
    idx: u8,
    brightness: u8,
    notification_state: &mut NotificationState,
    now: std::time::Instant,
    default_brightness: u8,
) -> ButtonRoutingDecision {
    if let Some(action) = notification_state.on_button_down(idx as usize, now) {
        return ButtonRoutingDecision::Notification(action);
    }

    if idx == BUTTON_BRIGHTNESS_RESET && brightness != default_brightness {
        return ButtonRoutingDecision::BrightnessReset;
    }

    ButtonRoutingDecision::Noop
}

/// ボタン画像にラベルテキストを描画して返す
pub fn draw_button_with_label(
    label: &str,
    bg_color: image::Rgb<u8>,
    sparkline_history: Option<&VecDeque<f32>>,
) -> anyhow::Result<DynamicImage> {
    const BUTTON_SIZE: u32 = 120;
    const TEXT_COLOR: Rgb<u8> = Rgb([255, 255, 255]);
    const SPARKLINE_BOTTOM_Y: i32 = 116;
    const SPARKLINE_HEIGHT: i32 = 18;
    const SPARKLINE_COLOR: Rgb<u8> = Rgb([240, 240, 240]);

    let mut img = RgbImage::from_pixel(BUTTON_SIZE, BUTTON_SIZE, bg_color);

    if !label.trim().is_empty() {
        let font = get_button_font();
        let scale = PxScale::from(BUTTON_FONT_SIZE);

        let text_y = (BUTTON_SIZE as i32 - BUTTON_FONT_SIZE as i32) / 2;
        let text_x = 10_i32;

        draw_text_mut(&mut img, TEXT_COLOR, text_x, text_y, scale, &font, label);
    }

    if let Some(history) = sparkline_history {
        if !history.is_empty() {
            let len = history.len() as i32;
            for (idx, cpu) in history.iter().enumerate() {
                let idx = idx as i32;
                let x0 = idx * BUTTON_SIZE as i32 / len;
                let x1 = ((idx + 1) * BUTTON_SIZE as i32 / len).max(x0 + 1);
                let normalized = (cpu / 100.0).clamp(0.0, 1.0);
                let bar_h = ((normalized * SPARKLINE_HEIGHT as f32).round() as i32).max(1);
                let y = SPARKLINE_BOTTOM_Y - bar_h;
                draw_filled_rect_mut(
                    &mut img,
                    Rect::at(x0, y).of_size((x1 - x0) as u32, bar_h as u32),
                    SPARKLINE_COLOR,
                );
            }
        }
    }

    Ok(DynamicImage::ImageRgb8(img))
}

/// ボタンにラベルテキスト画像を設定する
pub fn refresh_button_display(
    hw: &device::HardwareManager,
    state: &NotificationState,
    runtime_config: &RuntimeConfig,
    page_state: &PageState,
    podman_cpu_history: &HashMap<String, VecDeque<f32>>,
) -> anyhow::Result<()> {
    let assignments = resolve_button_assignments(runtime_config, page_state);

    for idx in 0..BUTTON_COUNT {
        if matches!(state.slots[idx], SlotState::Empty) {
            if let Some(assignment) = assignments.get(idx) {
                if let Some(sample_id) = assignment.sample_id.as_deref() {
                    if let Some(sample) = catalog::resolve_button_sample(runtime_config, sample_id)
                    {
                        let button_img = button_patterns::render_button_pattern(&sample)?;
                        hw.set_button_image(idx as u8, button_img)?;
                        continue;
                    }
                }

                let bg_color = assignment.color.to_rgb();
                let sparkline = match (&assignment.decision, assignment.color) {
                    (PageButtonDecision::PodmanLogs { container_id, .. }, _)
                        if assignment.podman_is_running =>
                    {
                        podman_cpu_history.get(container_id)
                    }
                    _ => None,
                };
                let button_img = draw_button_with_label(&assignment.label, bg_color, sparkline)?;
                hw.set_button_image(idx as u8, button_img)?;
            } else {
                hw.set_button_color(idx as u8, image::Rgb([0, 0, 0]))?;
            }
        } else {
            hw.set_button_color(idx as u8, state.slots[idx].color().to_rgb())?;
        }
    }
    hw.flush_buttons()?;
    Ok(())
}
