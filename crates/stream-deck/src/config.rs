//! Step1 用の新設定モデル。
//! 現行の DashboardConfig と並行して導入し、段階的移行を行う。
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub app: AppMeta,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub pages: Vec<PageConfig>,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub dynamic: DynamicConfig,
    #[serde(default)]
    pub watch: WatchConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DisplayConfig {
    #[serde(default)]
    pub samples: Vec<DisplaySampleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySampleConfig {
    pub id: String,
    pub target: DisplaySampleTarget,
    pub pattern: DisplayPatternKind,
    pub payload: DisplaySamplePayload,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DisplaySampleTarget {
    Button,
    Section,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPatternKind {
    LabelOnly,
    LabelValue,
    IconBadge,
    BarTrend,
    ErrorFallback,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DisplaySamplePayload {
    LabelOnly(LabelOnlyPayload),
    LabelValue(LabelValuePayload),
    IconBadge(IconBadgePayload),
    BarTrend(BarTrendPayload),
    ErrorFallback(ErrorFallbackPayload),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DisplaySeverity {
    Normal,
    Warn,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelOnlyPayload {
    pub label: String,
    pub bg_color: [u8; 3],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelValuePayload {
    pub title: String,
    pub value: String,
    pub unit: String,
    pub severity: DisplaySeverity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconBadgePayload {
    pub icon: String,
    pub badge_count: u32,
    pub status: DisplaySeverity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarTrendPayload {
    pub current: f32,
    pub max: f32,
    pub history: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorFallbackPayload {
    pub error_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppMeta {
    #[serde(default = "default_home_page_id")]
    pub home: String,
}

impl Default for AppMeta {
    fn default() -> Self {
        Self {
            home: default_home_page_id(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    #[serde(default = "default_dashboard_sections")]
    pub sections: Vec<DashboardSectionConfig>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            sections: default_dashboard_sections(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardSectionConfig {
    #[serde(rename = "type")]
    pub kind: DashboardSectionKind,
    #[serde(default = "default_section_capacity")]
    pub capacity: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DashboardSectionKind {
    Cpu,
    Mem,
    Load,
    Bright,
    Notif,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageConfig {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub items: Vec<PageItemConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PageItemConfig {
    Nav {
        label: String,
        target: String,
        #[serde(default)]
        priority: Option<i32>,
        /// ボタンとして表示するかどうか。`false` にするとボタン割り当てから除外される。
        /// エンコーダによるページ遷移には影響しない（`find_page_nav_target` は常に走査する）。
        /// TOML 未指定時は `true`（後方互換性を保つ）。
        #[serde(default = "default_true")]
        visible: bool,
    },
    Command {
        label: String,
        command: Vec<String>,
        #[serde(default)]
        priority: Option<i32>,
    },
    Back {
        #[serde(default)]
        label: String,
        #[serde(default)]
        priority: Option<i32>,
    },
    #[serde(rename = "ssh-connect")]
    SshConnect {
        label: String,
        host: String,
        #[serde(default)]
        terminal: Vec<String>,
        #[serde(default = "default_ssh_template")]
        ssh_template: String,
        #[serde(default)]
        priority: Option<i32>,
    },
    /// Podman コンテナ監視ボタン。動的ページ生成時に programmatic に追加される。
    #[serde(rename = "podman-monitor")]
    PodmanMonitor {
        container_id: String,
        label: String,
        state: String,
        #[serde(default)]
        priority: Option<i32>,
    },
    Sample {
        sample_id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        priority: Option<i32>,
    },
}

/// priority の推奨レンジ。
/// この範囲外もパースは許可するが、設定検証で警告を返す。
pub const PAGE_ITEM_PRIORITY_RECOMMENDED_MIN: i32 = -100;
pub const PAGE_ITEM_PRIORITY_RECOMMENDED_MAX: i32 = 100;

fn page_item_priority(item: &PageItemConfig) -> Option<i32> {
    match item {
        PageItemConfig::Nav { priority, .. }
        | PageItemConfig::Command { priority, .. }
        | PageItemConfig::Back { priority, .. }
        | PageItemConfig::SshConnect { priority, .. }
        | PageItemConfig::PodmanMonitor { priority, .. }
        | PageItemConfig::Sample { priority, .. } => *priority,
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DynamicConfig {
    #[serde(default)]
    pub ssh_hosts: Option<DynamicSshConfig>,
    #[serde(default)]
    pub podman: Option<DynamicPodmanConfig>,
}

/// Podman コンテナ監視の動的ページ設定。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicPodmanConfig {
    /// 監視を有効にするか
    #[serde(default)]
    pub enabled: bool,
    /// 生成ページ ID の共通プレフィックス
    #[serde(default = "default_podman_page_prefix")]
    pub page_id_prefix: String,
    /// 1ページに表示するコンテナ数
    #[serde(default = "default_podman_page_size")]
    pub page_size: usize,
    /// Podman Unix ソケットパス。省略時は XDG_RUNTIME_DIR から自動解決
    #[serde(default)]
    pub socket: Option<String>,
    /// ポーリング間隔 (ミリ秒)
    #[serde(default = "default_podman_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// CPU 使用率の履歴長 (バー本数)
    #[serde(default = "default_podman_cpu_history_len")]
    pub cpu_history_len: usize,
    /// logs 表示コマンド (container_id を末尾に付加して実行)
    #[serde(default = "default_podman_log_command")]
    pub log_command: Vec<String>,
    /// logs を表示する端末コマンド (log_command の前に置く)
    #[serde(default)]
    pub terminal: Vec<String>,
    /// Podman ボタンの状態別表示色
    #[serde(default)]
    pub colors: PodmanButtonColors,
    /// コンテナ一覧を再取得してページを再生成する周期 (ミリ秒)。poll_interval_ms より大きい値が推奨される。
    #[serde(default = "default_podman_container_list_poll_interval_ms")]
    pub container_list_poll_interval_ms: u64,
}

/// Podman ボタン色設定。各色は RGB 3要素で指定する。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PodmanButtonColors {
    #[serde(default = "default_podman_color_running")]
    pub running: [u8; 3],
    #[serde(default = "default_podman_color_exited")]
    pub exited: [u8; 3],
    #[serde(default = "default_podman_color_paused")]
    pub paused: [u8; 3],
    #[serde(default = "default_podman_color_other")]
    pub other: [u8; 3],
}

impl Default for PodmanButtonColors {
    fn default() -> Self {
        Self {
            running: default_podman_color_running(),
            exited: default_podman_color_exited(),
            paused: default_podman_color_paused(),
            other: default_podman_color_other(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicSshConfig {
    pub source: String,
    #[serde(default = "default_ssh_page_size")]
    pub page_size: usize,
    #[serde(default = "default_ssh_page_prefix")]
    pub page_id_prefix: String,
    #[serde(default)]
    pub terminal: Vec<String>,
    #[serde(default = "default_ssh_template")]
    pub ssh_template: String,
}

impl Default for DynamicSshConfig {
    fn default() -> Self {
        Self {
            source: String::new(),
            page_size: default_ssh_page_size(),
            page_id_prefix: default_ssh_page_prefix(),
            terminal: Vec::new(),
            ssh_template: default_ssh_template(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WatchConfig {
    #[serde(default)]
    pub includes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SshHostsConfig {
    #[serde(default)]
    pub hosts: Vec<SshHostEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshHostEntry {
    pub id: String,
    pub label: String,
    pub host: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Step 5 でページ生成前に扱う正規化済みホストエントリ。
/// TOML パース用の構造体とは分離し、動的ページ生成入力の中間表現として使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEntry {
    pub id: String,
    pub label: String,
    pub host: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshPageBuildInput {
    pub page_size: usize,
    pub page_id_prefix: String,
    pub terminal: Vec<String>,
    pub ssh_template: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub ssh_enabled: bool,
    pub ssh_disabled_reason: Option<String>,
    pub podman_enabled: bool,
    pub podman_disabled_reason: Option<String>,
}

impl ConfigValidationReport {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct AppConfigBundle {
    pub app: AppConfig,
    pub ssh_hosts: Option<Vec<HostEntry>>,
    pub report: ConfigValidationReport,
    pub watch_targets: Vec<PathBuf>,
}

impl AppConfigBundle {
    pub fn load(config_path: &str, terminal_title_capable: bool) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(config_path).map_err(|e| {
            anyhow::anyhow!("設定ファイルを読めませんでした path={config_path}: {e}")
        })?;
        let app: AppConfig = toml::from_str(&text).map_err(|e| {
            anyhow::anyhow!("設定ファイルの TOML パースに失敗しました path={config_path}: {e}")
        })?;

        let watch_targets = resolve_watch_targets(config_path, &app);

        let mut ssh_hosts = None;
        if let Some(ssh) = &app.dynamic.ssh_hosts {
            if !ssh.source.trim().is_empty() {
                let entries = load_ssh_hosts_entries(config_path, &ssh.source)?;
                ssh_hosts = Some(entries);
            }
        }

        let report = validate_app_config(&app, ssh_hosts.as_deref(), terminal_title_capable);

        Ok(Self {
            app,
            ssh_hosts,
            report,
            watch_targets,
        })
    }
}

pub fn validate_app_config(
    app: &AppConfig,
    ssh_hosts: Option<&[HostEntry]>,
    terminal_title_capable: bool,
) -> ConfigValidationReport {
    let mut report = ConfigValidationReport::default();

    let mut display_samples: BTreeMap<String, DisplaySampleTarget> = BTreeMap::new();
    for sample in &app.display.samples {
        let sample_id = sample.id.trim();
        if sample_id.is_empty() {
            report
                .errors
                .push("display.samples id が空です".to_string());
            continue;
        }
        if display_samples
            .insert(sample_id.to_string(), sample.target)
            .is_some()
        {
            report
                .errors
                .push(format!("重複した display sample id です: {sample_id}"));
        }

        let pattern_matches_payload = matches!(
            (&sample.pattern, &sample.payload),
            (
                DisplayPatternKind::LabelOnly,
                DisplaySamplePayload::LabelOnly(_)
            ) | (
                DisplayPatternKind::LabelValue,
                DisplaySamplePayload::LabelValue(_)
            ) | (
                DisplayPatternKind::IconBadge,
                DisplaySamplePayload::IconBadge(_)
            ) | (
                DisplayPatternKind::BarTrend,
                DisplaySamplePayload::BarTrend(_)
            ) | (
                DisplayPatternKind::ErrorFallback,
                DisplaySamplePayload::ErrorFallback(_)
            )
        );
        if !pattern_matches_payload {
            report.errors.push(format!(
                "display sample={sample_id}: pattern と payload の組み合わせが不正です"
            ));
        }

        if let DisplaySamplePayload::BarTrend(payload) = &sample.payload {
            if payload.max <= 0.0 {
                report.errors.push(format!(
                    "display sample={sample_id}: bar_trend.max は 0 より大きい値が必要です"
                ));
            }
            if payload.history.is_empty() {
                report.warnings.push(format!(
                    "display sample={sample_id}: bar_trend.history が空です"
                ));
            }
        }
    }

    if app.dashboard.sections.is_empty() {
        report
            .errors
            .push("dashboard.sections が空です".to_string());
    }
    for (idx, section) in app.dashboard.sections.iter().enumerate() {
        if section.capacity == 0 {
            report.errors.push(format!(
                "dashboard.sections[{idx}].capacity は 1 以上が必要です"
            ));
        }
    }

    let mut page_ids = BTreeSet::new();
    for page in &app.pages {
        let id = page.id.trim();
        if id.is_empty() {
            report.errors.push("page id が空です".to_string());
            continue;
        }
        if !page_ids.insert(id.to_string()) {
            report.errors.push(format!("重複した page id です: {id}"));
        }

        for (item_idx, item) in page.items.iter().enumerate() {
            match item {
                PageItemConfig::Nav { target, .. } => {
                    if target.trim().is_empty() {
                        report
                            .errors
                            .push(format!("page={id}: nav target が空です"));
                    }
                }
                PageItemConfig::Command { command, .. } => {
                    if command.is_empty() || command[0].trim().is_empty() {
                        report.errors.push(format!("page={id}: command が空です"));
                    }
                }
                PageItemConfig::Back { .. } => {}
                PageItemConfig::SshConnect { host, terminal, .. } => {
                    if host.trim().is_empty() {
                        report
                            .errors
                            .push(format!("page={id}: ssh-connect host が空です"));
                    }
                    if terminal.is_empty() || terminal[0].trim().is_empty() {
                        report.warnings.push(format!(
                            "page={id}: ssh-connect terminal が未指定のため実行時解決に依存します"
                        ));
                    }
                }
                PageItemConfig::PodmanMonitor {
                    container_id,
                    label,
                    ..
                } => {
                    if container_id.trim().is_empty() {
                        report
                            .errors
                            .push(format!("page={id}: podman-monitor container_id が空です"));
                    }
                    if label.trim().is_empty() {
                        report
                            .warnings
                            .push(format!("page={id}: podman-monitor label が空です"));
                    }
                }
                PageItemConfig::Sample { sample_id, .. } => {
                    let sample_id = sample_id.trim();
                    if sample_id.is_empty() {
                        report
                            .errors
                            .push(format!("page={id}: sample sample_id が空です"));
                    } else {
                        match display_samples.get(sample_id) {
                            Some(DisplaySampleTarget::Button | DisplaySampleTarget::Section) => {}
                            None => {
                                report.errors.push(format!(
                                    "page={id}: sample_id が存在しません: {sample_id}"
                                ));
                            }
                        }
                    }
                }
            }

            if let Some(priority) = page_item_priority(item) {
                if !(PAGE_ITEM_PRIORITY_RECOMMENDED_MIN..=PAGE_ITEM_PRIORITY_RECOMMENDED_MAX)
                    .contains(&priority)
                {
                    report.warnings.push(format!(
                        "page={id} item[{item_idx}] の priority={} は推奨レンジ外です ({}..={})",
                        priority,
                        PAGE_ITEM_PRIORITY_RECOMMENDED_MIN,
                        PAGE_ITEM_PRIORITY_RECOMMENDED_MAX
                    ));
                }
            }
        }
    }

    if !app.pages.is_empty()
        && !app.app.home.trim().is_empty()
        && !page_ids.contains(app.app.home.trim())
    {
        report
            .errors
            .push(format!("home ページが存在しません: {}", app.app.home));
    }

    for page in &app.pages {
        for item in &page.items {
            if let PageItemConfig::Nav { target, .. } = item {
                let target = target.trim();
                if !target.is_empty() && !page_ids.contains(target) {
                    report.errors.push(format!(
                        "page={} の nav target が存在しません: {target}",
                        page.id
                    ));
                }
            }
        }
    }

    if let Some(ssh) = &app.dynamic.ssh_hosts {
        // 動的 SSH ページ先頭への nav を静的検証で許容するため、
        // nav target 検証より前にプレフィックスを page_ids へ追加する。
        if !ssh.page_id_prefix.trim().is_empty() {
            page_ids.insert(ssh.page_id_prefix.trim().to_string());
        }
    }
    if let Some(podman) = &app.dynamic.podman {
        if podman.enabled && !podman.page_id_prefix.trim().is_empty() {
            page_ids.insert(podman.page_id_prefix.trim().to_string());
        }
    }

    for page in &app.pages {
        for item in &page.items {
            if let PageItemConfig::Nav { target, .. } = item {
                let target = target.trim();
                if !target.is_empty() && !page_ids.contains(target) {
                    report.errors.push(format!(
                        "page={} の nav target が存在しません: {target}",
                        page.id
                    ));
                }
            }
        }
    }

    if let Some(ssh) = &app.dynamic.ssh_hosts {
        if ssh.page_size == 0 {
            report
                .errors
                .push("dynamic.ssh_hosts.page_size は 1 以上が必要です".to_string());
        }
        if ssh.source.trim().is_empty() {
            report
                .errors
                .push("dynamic.ssh_hosts.source が空です".to_string());
        }
        if ssh.page_id_prefix.trim().is_empty() {
            report
                .errors
                .push("dynamic.ssh_hosts.page_id_prefix が空です".to_string());
        }

        if terminal_title_capable {
            report.ssh_enabled = true;
        } else {
            report.ssh_enabled = false;
            report.ssh_disabled_reason =
                Some("端末タイトル設定に未対応のため SSH ページを無効化します".to_string());
            report
                .warnings
                .push("SSH ページを無効化しました（端末タイトル設定不可）".to_string());
        }

        if let Some(hosts) = ssh_hosts {
            let mut host_ids = BTreeSet::new();
            for host in hosts {
                let id = host.id.trim();
                if id.is_empty() {
                    report.errors.push("SSH host id が空です".to_string());
                    continue;
                }
                if !host_ids.insert(id.to_string()) {
                    report
                        .errors
                        .push(format!("重複した SSH host id です: {id}"));
                }
                if host.host.trim().is_empty() {
                    report
                        .errors
                        .push(format!("SSH host の接続先が空です: id={id}"));
                }
            }
        }
    }

    if let Some(podman) = &app.dynamic.podman {
        if podman.enabled {
            let mut podman_errors = false;
            if podman.page_size == 0 {
                report
                    .errors
                    .push("dynamic.podman.page_size は 1 以上が必要です".to_string());
                podman_errors = true;
            }
            if podman.poll_interval_ms < 100 {
                report
                    .errors
                    .push("dynamic.podman.poll_interval_ms は 100 以上が必要です".to_string());
                podman_errors = true;
            }
            if podman.cpu_history_len < 2 {
                report
                    .errors
                    .push("dynamic.podman.cpu_history_len は 2 以上が必要です".to_string());
                podman_errors = true;
            }
            if podman.log_command.is_empty() {
                report
                    .errors
                    .push("dynamic.podman.log_command が空です".to_string());
                podman_errors = true;
            }
            if podman.terminal.is_empty() {
                report.warnings.push(
                    "dynamic.podman.terminal が未指定のため logs 実行時解決に依存します"
                        .to_string(),
                );
            }
            if !podman_errors {
                report.podman_enabled = true;
            }
        }
    }

    report
}

/// `ssh_hosts.toml` を読み込み、動的ページ生成で使う中間表現へ正規化する。
pub fn load_ssh_hosts_entries(config_path: &str, source: &str) -> anyhow::Result<Vec<HostEntry>> {
    let hosts_path = resolve_child_path(config_path, source);
    let hosts_text = std::fs::read_to_string(&hosts_path).map_err(|e| {
        anyhow::anyhow!(
            "SSH ホスト設定を読めませんでした path={}: {e}",
            hosts_path.display()
        )
    })?;
    let parsed: SshHostsConfig = toml::from_str(&hosts_text).map_err(|e| {
        anyhow::anyhow!(
            "SSH ホスト設定の TOML パースに失敗しました path={}: {e}",
            hosts_path.display()
        )
    })?;

    Ok(normalize_ssh_hosts(parsed))
}

fn normalize_ssh_hosts(raw: SshHostsConfig) -> Vec<HostEntry> {
    raw.hosts
        .into_iter()
        .map(|host| HostEntry {
            id: host.id.trim().to_string(),
            label: {
                let label = host.label.trim();
                if label.is_empty() {
                    host.id.trim().to_string()
                } else {
                    label.to_string()
                }
            },
            host: host.host.trim().to_string(),
            tags: {
                let mut seen = BTreeSet::new();
                let mut tags = Vec::new();
                for tag in host.tags {
                    let normalized = tag.trim().to_string();
                    if normalized.is_empty() {
                        continue;
                    }
                    if seen.insert(normalized.clone()) {
                        tags.push(normalized);
                    }
                }
                tags
            },
        })
        .collect()
}

pub fn build_ssh_page_build_input(app: &AppConfig) -> Option<SshPageBuildInput> {
    let ssh = app.dynamic.ssh_hosts.as_ref()?;
    if ssh.source.trim().is_empty() {
        return None;
    }
    let page_id_prefix = ssh.page_id_prefix.trim();
    if page_id_prefix.is_empty() {
        return None;
    }

    Some(SshPageBuildInput {
        page_size: ssh.page_size,
        page_id_prefix: page_id_prefix.to_string(),
        terminal: ssh.terminal.clone(),
        ssh_template: ssh.ssh_template.clone(),
    })
}

pub fn build_dynamic_ssh_pages(input: &SshPageBuildInput, hosts: &[HostEntry]) -> Vec<PageConfig> {
    if input.page_size == 0 || hosts.is_empty() {
        return Vec::new();
    }

    let chunk_count = hosts.len().div_ceil(input.page_size);
    let page_ids: Vec<String> = (0..chunk_count)
        .map(|idx| {
            if idx == 0 {
                input.page_id_prefix.clone()
            } else {
                format!("{}_{}", input.page_id_prefix, idx + 1)
            }
        })
        .collect();

    let mut pages = Vec::with_capacity(chunk_count);
    for (page_idx, chunk) in hosts.chunks(input.page_size).enumerate() {
        let mut items: Vec<PageItemConfig> = Vec::new();

        items.push(PageItemConfig::Back {
            label: "Back".to_string(),
            priority: Some(-100),
        });
        if page_idx > 0 {
            items.push(PageItemConfig::Nav {
                label: "Prev".to_string(),
                target: page_ids[page_idx - 1].clone(),
                priority: Some(-90),
                visible: false,
            });
        }
        if page_idx + 1 < page_ids.len() {
            items.push(PageItemConfig::Nav {
                label: "Next".to_string(),
                target: page_ids[page_idx + 1].clone(),
                priority: Some(-80),
                visible: false,
            });
        }

        for host in chunk {
            items.push(PageItemConfig::SshConnect {
                label: host.label.clone(),
                host: host.host.clone(),
                terminal: input.terminal.clone(),
                ssh_template: input.ssh_template.clone(),
                priority: Some(0),
            });
        }

        pages.push(PageConfig {
            id: page_ids[page_idx].clone(),
            title: format!("SSH {} / {}", page_idx + 1, page_ids.len()),
            items,
        });
    }

    pages
}

pub fn resolve_watch_targets(config_path: &str, app: &AppConfig) -> Vec<PathBuf> {
    let base_path = normalize_path(Path::new(config_path));
    let base_dir = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut dedup = BTreeSet::new();
    dedup.insert(base_path);

    for include in &app.watch.includes {
        let include = include.trim();
        if include.is_empty() {
            continue;
        }
        let include_path = Path::new(include);
        let resolved = if include_path.is_absolute() {
            normalize_path(include_path)
        } else {
            normalize_path(&base_dir.join(include_path))
        };
        dedup.insert(resolved);
    }

    if let Some(ssh) = &app.dynamic.ssh_hosts {
        if !ssh.source.trim().is_empty() {
            dedup.insert(resolve_child_path(config_path, &ssh.source));
        }
    }

    dedup.into_iter().collect()
}

fn resolve_child_path(config_path: &str, child: &str) -> PathBuf {
    let base = normalize_path(Path::new(config_path));
    let base_dir = base
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let path = Path::new(child);
    if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&base_dir.join(path))
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

fn default_true() -> bool {
    true
}

fn default_home_page_id() -> String {
    "home".to_string()
}

fn default_section_capacity() -> usize {
    10
}

fn default_dashboard_sections() -> Vec<DashboardSectionConfig> {
    vec![
        DashboardSectionConfig {
            kind: DashboardSectionKind::Cpu,
            capacity: default_section_capacity(),
        },
        DashboardSectionConfig {
            kind: DashboardSectionKind::Mem,
            capacity: default_section_capacity(),
        },
        DashboardSectionConfig {
            kind: DashboardSectionKind::Load,
            capacity: default_section_capacity(),
        },
        DashboardSectionConfig {
            kind: DashboardSectionKind::Notif,
            capacity: default_section_capacity(),
        },
    ]
}

fn default_ssh_page_size() -> usize {
    6
}

fn default_ssh_page_prefix() -> String {
    "ssh_hosts".to_string()
}

fn default_ssh_template() -> String {
    "ssh {host}".to_string()
}

fn default_podman_page_size() -> usize {
    6
}

fn default_podman_page_prefix() -> String {
    "podman".to_string()
}

fn default_podman_poll_interval_ms() -> u64 {
    500
}

fn default_podman_cpu_history_len() -> usize {
    30
}

fn default_podman_log_command() -> Vec<String> {
    vec!["podman".to_string(), "logs".to_string(), "-f".to_string()]
}

fn default_podman_color_running() -> [u8; 3] {
    [0, 180, 40]
}

fn default_podman_color_exited() -> [u8; 3] {
    [200, 30, 30]
}

fn default_podman_color_paused() -> [u8; 3] {
    [200, 160, 0]
}

fn default_podman_color_other() -> [u8; 3] {
    [80, 80, 80]
}

fn default_podman_container_list_poll_interval_ms() -> u64 {
    5000
}

// ── Podman ページ生成 ──────────────────────────────────────────────

/// `build_dynamic_podman_pages` への入力パラメータ。
#[derive(Debug, Clone)]
pub struct PodmanPageBuildInput {
    pub page_size: usize,
    pub page_id_prefix: String,
    pub terminal: Vec<String>,
    pub log_command: Vec<String>,
}

/// `DynamicPodmanConfig` から `PodmanPageBuildInput` を組み立てる。
/// `enabled = false` または必須フィールドが不正な場合は `None` を返す。
pub fn build_podman_page_build_input(cfg: &DynamicPodmanConfig) -> Option<PodmanPageBuildInput> {
    if !cfg.enabled {
        return None;
    }
    let prefix = cfg.page_id_prefix.trim();
    if prefix.is_empty() || cfg.page_size == 0 {
        return None;
    }
    Some(PodmanPageBuildInput {
        page_size: cfg.page_size,
        page_id_prefix: prefix.to_string(),
        terminal: cfg.terminal.clone(),
        log_command: cfg.log_command.clone(),
    })
}

/// Podman コンテナエントリ（ページ生成に必要な最小フィールド）。
#[derive(Debug, Clone)]
pub struct PodmanContainerEntry {
    pub id: String,
    pub label: String,
    pub state: String,
}

/// Podman コンテナ一覧から動的ページ群を生成する。
/// SSH ページと同じページング規則（Back 固定 / Prev / Next）を使う。
pub fn build_dynamic_podman_pages(
    input: &PodmanPageBuildInput,
    entries: &[PodmanContainerEntry],
) -> Vec<PageConfig> {
    if input.page_size == 0 || entries.is_empty() {
        return Vec::new();
    }

    let chunk_count = entries.len().div_ceil(input.page_size);
    let page_ids: Vec<String> = (0..chunk_count)
        .map(|idx| {
            if idx == 0 {
                input.page_id_prefix.clone()
            } else {
                format!("{}_{}", input.page_id_prefix, idx + 1)
            }
        })
        .collect();

    let mut pages = Vec::with_capacity(chunk_count);
    for (page_idx, chunk) in entries.chunks(input.page_size).enumerate() {
        let mut items: Vec<PageItemConfig> = Vec::new();

        items.push(PageItemConfig::Back {
            label: "Back".to_string(),
            priority: Some(-100),
        });
        if page_idx > 0 {
            items.push(PageItemConfig::Nav {
                label: "Prev".to_string(),
                target: page_ids[page_idx - 1].clone(),
                priority: Some(-90),
                visible: false,
            });
        }
        if page_idx + 1 < page_ids.len() {
            items.push(PageItemConfig::Nav {
                label: "Next".to_string(),
                target: page_ids[page_idx + 1].clone(),
                priority: Some(-80),
                visible: false,
            });
        }

        for entry in chunk {
            items.push(PageItemConfig::PodmanMonitor {
                container_id: entry.id.clone(),
                label: entry.label.clone(),
                state: entry.state.clone(),
                priority: Some(0),
            });
        }

        pages.push(PageConfig {
            id: page_ids[page_idx].clone(),
            title: format!("Podman {} / {}", page_idx + 1, page_ids.len()),
            items,
        });
    }

    pages
}

/// Podman Unix ソケットパスを解決する。
/// `cfg.socket` が指定されていればそれを使い、なければ `XDG_RUNTIME_DIR` から解決する。
pub fn resolve_podman_socket_path(cfg: &DynamicPodmanConfig) -> String {
    if let Some(socket) = &cfg.socket {
        let trimmed = socket.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", read_process_uid()));
    format!("{}/podman/podman.sock", runtime_dir)
}

/// `/proc/self/status` から実行中プロセスの UID を読み取る。失敗時は 1000 を返す。
fn read_process_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(1000)
}

// ── テスト ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_podman_cfg(
        page_size: usize,
        poll_ms: u64,
        hist_len: usize,
        log_cmd: Vec<String>,
    ) -> DynamicPodmanConfig {
        DynamicPodmanConfig {
            enabled: true,
            page_id_prefix: "podman".to_string(),
            page_size,
            socket: None,
            poll_interval_ms: poll_ms,
            cpu_history_len: hist_len,
            log_command: log_cmd,
            terminal: vec![],
            colors: PodmanButtonColors::default(),
            container_list_poll_interval_ms: 5000,
        }
    }

    fn base_app(podman: Option<DynamicPodmanConfig>) -> AppConfig {
        AppConfig {
            app: AppMeta {
                home: "home".to_string(),
            },
            dashboard: DashboardConfig {
                sections: vec![DashboardSectionConfig {
                    kind: DashboardSectionKind::Cpu,
                    capacity: 10,
                }],
            },
            pages: vec![PageConfig {
                id: "home".to_string(),
                title: "Home".to_string(),
                items: vec![],
            }],
            display: DisplayConfig::default(),
            dynamic: DynamicConfig {
                ssh_hosts: None,
                podman: podman.map(|p| p),
            },
            watch: WatchConfig::default(),
        }
    }

    /// 値域確認: page_size=0 はエラー
    #[test]
    fn test_podman_validate_page_size_zero() {
        let app = base_app(Some(make_podman_cfg(0, 500, 10, vec!["podman".into()])));
        let report = validate_app_config(&app, None, false);
        assert!(
            report.errors.iter().any(|e| e.contains("page_size")),
            "エラー一覧: {:?}",
            report.errors
        );
        assert!(!report.podman_enabled);
    }

    /// 値域確認: poll_interval_ms<100 はエラー
    #[test]
    fn test_podman_validate_poll_interval_too_small() {
        let app = base_app(Some(make_podman_cfg(6, 99, 10, vec!["podman".into()])));
        let report = validate_app_config(&app, None, false);
        assert!(
            report.errors.iter().any(|e| e.contains("poll_interval_ms")),
            "{:?}",
            report.errors
        );
    }

    /// 値域確認: cpu_history_len<2 はエラー
    #[test]
    fn test_podman_validate_history_len_too_small() {
        let app = base_app(Some(make_podman_cfg(6, 500, 1, vec!["podman".into()])));
        let report = validate_app_config(&app, None, false);
        assert!(
            report.errors.iter().any(|e| e.contains("cpu_history_len")),
            "{:?}",
            report.errors
        );
    }

    /// 値域確認: log_command 空はエラー
    #[test]
    fn test_podman_validate_log_command_empty() {
        let app = base_app(Some(make_podman_cfg(6, 500, 10, vec![])));
        let report = validate_app_config(&app, None, false);
        assert!(
            report.errors.iter().any(|e| e.contains("log_command")),
            "{:?}",
            report.errors
        );
    }

    /// 正常系: 有効設定で podman_enabled = true
    #[test]
    fn test_podman_validate_valid() {
        let app = base_app(Some(make_podman_cfg(
            6,
            500,
            10,
            vec!["podman".into(), "logs".into(), "-f".into()],
        )));
        let report = validate_app_config(&app, None, false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.podman_enabled);
    }

    fn make_entries(n: usize) -> Vec<PodmanContainerEntry> {
        (0..n)
            .map(|i| PodmanContainerEntry {
                id: format!("id{i:012}"),
                label: format!("container{i}"),
                state: "running".to_string(),
            })
            .collect()
    }

    fn make_input() -> PodmanPageBuildInput {
        PodmanPageBuildInput {
            page_size: 4,
            page_id_prefix: "podman".to_string(),
            terminal: vec![],
            log_command: vec!["podman".into(), "logs".into(), "-f".into()],
        }
    }

    /// 正常系: 0件でページ生成なし
    #[test]
    fn test_podman_pages_empty() {
        let pages = build_dynamic_podman_pages(&make_input(), &[]);
        assert!(pages.is_empty());
    }

    /// 正常系: 1件で1ページ (Back のみ、Prev/Next なし)
    #[test]
    fn test_podman_pages_single() {
        let pages = build_dynamic_podman_pages(&make_input(), &make_entries(1));
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].id, "podman");
        let has_back = pages[0]
            .items
            .iter()
            .any(|i| matches!(i, PageItemConfig::Back { .. }));
        let has_prev = pages[0]
            .items
            .iter()
            .any(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Prev"));
        let has_next = pages[0]
            .items
            .iter()
            .any(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Next"));
        assert!(has_back);
        assert!(!has_prev);
        assert!(!has_next);
    }

    /// 正常系: page_size=4 で5件 → 2ページ (Next/Back/Prev)
    #[test]
    fn test_podman_pages_pagination() {
        let pages = build_dynamic_podman_pages(&make_input(), &make_entries(5));
        assert_eq!(pages.len(), 2);
        // page1: Back + Next + 4 containers
        let p1 = &pages[0];
        assert_eq!(p1.id, "podman");
        let monitor_count = p1
            .items
            .iter()
            .filter(|i| matches!(i, PageItemConfig::PodmanMonitor { .. }))
            .count();
        assert_eq!(monitor_count, 4);
        let has_next = p1
            .items
            .iter()
            .any(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Next"));
        assert!(has_next);
        // page2: Back + Prev + 1 container
        let p2 = &pages[1];
        assert_eq!(p2.id, "podman_2");
        let monitor_count2 = p2
            .items
            .iter()
            .filter(|i| matches!(i, PageItemConfig::PodmanMonitor { .. }))
            .count();
        assert_eq!(monitor_count2, 1);
        let has_prev = p2
            .items
            .iter()
            .any(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Prev"));
        assert!(has_prev);
    }

    /// 後方互換: TOML に visible 未指定の Nav は visible=true として扱われる
    #[test]
    fn test_nav_visible_default_true() {
        let toml = r#"
            [[pages]]
            id = "home"
            title = "Home"
            [[pages.items]]
            kind = "nav"
            label = "Go"
            target = "home"
        "#;
        let app: AppConfig = toml::from_str(toml).unwrap();
        let item = &app.pages[0].items[0];
        if let PageItemConfig::Nav { visible, .. } = item {
            assert!(*visible, "デフォルトは true であるべき");
        } else {
            panic!("Nav アイテムでない");
        }
    }

    /// 値域確認: visible=false の Nav アイテムは TOML で明示的に設定できる
    #[test]
    fn test_nav_visible_false_in_toml() {
        let toml = r#"
            [[pages]]
            id = "home"
            title = "Home"
            [[pages.items]]
            kind = "nav"
            label = "Prev"
            target = "home"
            visible = false
        "#;
        let app: AppConfig = toml::from_str(toml).unwrap();
        let item = &app.pages[0].items[0];
        if let PageItemConfig::Nav { visible, .. } = item {
            assert!(!*visible, "visible=false が正しく設定されるべき");
        } else {
            panic!("Nav アイテムでない");
        }
    }

    /// 正常系: 動的 SSH ページの Prev/Next は visible=false で生成される
    #[test]
    fn test_ssh_pages_prev_next_invisible() {
        use crate::config::{HostEntry, SshPageBuildInput};
        let input = SshPageBuildInput {
            page_size: 1,
            page_id_prefix: "ssh".to_string(),
            terminal: vec![],
            ssh_template: "ssh {host}".to_string(),
        };
        let hosts = vec![
            HostEntry {
                id: "h1".to_string(),
                label: "Host1".to_string(),
                host: "host1.example.com".to_string(),
                tags: vec![],
            },
            HostEntry {
                id: "h2".to_string(),
                label: "Host2".to_string(),
                host: "host2.example.com".to_string(),
                tags: vec![],
            },
        ];
        let pages = build_dynamic_ssh_pages(&input, &hosts);
        // page1 の Next は visible=false
        let next_item = pages[0]
            .items
            .iter()
            .find(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Next"));
        let PageItemConfig::Nav { visible, .. } = next_item.unwrap() else {
            panic!()
        };
        assert!(!*visible, "動的 SSH Next は visible=false であるべき");
        // page2 の Prev は visible=false
        let prev_item = pages[1]
            .items
            .iter()
            .find(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Prev"));
        let PageItemConfig::Nav { visible, .. } = prev_item.unwrap() else {
            panic!()
        };
        assert!(!*visible, "動的 SSH Prev は visible=false であるべき");
    }

    /// 正常系: 動的 Podman ページの Prev/Next は visible=false で生成される
    #[test]
    fn test_podman_pages_prev_next_invisible() {
        let pages = build_dynamic_podman_pages(&make_input(), &make_entries(5));
        // page1 の Next は visible=false
        let next_item = pages[0]
            .items
            .iter()
            .find(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Next"));
        let PageItemConfig::Nav { visible, .. } = next_item.unwrap() else {
            panic!()
        };
        assert!(!*visible, "Podman Next は visible=false であるべき");
        // page2 の Prev は visible=false
        let prev_item = pages[1]
            .items
            .iter()
            .find(|i| matches!(i, PageItemConfig::Nav { label, .. } if label == "Prev"));
        let PageItemConfig::Nav { visible, .. } = prev_item.unwrap() else {
            panic!()
        };
        assert!(!*visible, "Podman Prev は visible=false であるべき");
    }

    /// 正常系: container_id と label が正しく PodmanMonitor に入る
    #[test]
    fn test_podman_pages_item_fields() {
        let entries = vec![PodmanContainerEntry {
            id: "abc123".to_string(),
            label: "myapp".to_string(),
            state: "running".to_string(),
        }];
        let pages = build_dynamic_podman_pages(&make_input(), &entries);
        let item = pages[0]
            .items
            .iter()
            .find(|i| matches!(i, PageItemConfig::PodmanMonitor { .. }))
            .unwrap();
        if let PageItemConfig::PodmanMonitor {
            container_id,
            label,
            state,
            ..
        } = item
        {
            assert_eq!(container_id, "abc123");
            assert_eq!(label, "myapp");
            assert_eq!(state, "running");
        } else {
            panic!("PodmanMonitor が見つかりません");
        }
    }
}
