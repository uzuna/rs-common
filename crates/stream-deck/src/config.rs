//! Step1 用の新設定モデル。
//! 現行の DashboardConfig と並行して導入し、段階的移行を行う。
#![allow(dead_code)]

use std::collections::BTreeSet;
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
    pub dynamic: DynamicConfig,
    #[serde(default)]
    pub watch: WatchConfig,
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
        | PageItemConfig::SshConnect { priority, .. } => *priority,
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DynamicConfig {
    #[serde(default)]
    pub ssh_hosts: Option<DynamicSshConfig>,
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

        let report = validate_app_config(
            &app,
            ssh_hosts.as_ref().map(Vec::as_slice),
            terminal_title_capable,
        );

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
            });
        }
        if page_idx + 1 < page_ids.len() {
            items.push(PageItemConfig::Nav {
                label: "Next".to_string(),
                target: page_ids[page_idx + 1].clone(),
                priority: Some(-80),
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
