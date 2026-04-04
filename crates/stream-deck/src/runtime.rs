//! 実行時設定管理モジュール。
//! RuntimeConfig と ConfigManager (ホットリロード含む) を提供する。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(feature = "file-watch")]
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(feature = "file-watch")]
use std::sync::mpsc;

use tracing::{info, warn};

use crate::config;

/// 設定ファイルリロードのデバウンス間隔
#[cfg(feature = "file-watch")]
pub const RELOAD_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);

/// 実行時に利用する統合設定。
pub struct RuntimeConfig {
    pub sections: Vec<config::DashboardSectionConfig>,
    pub home_page_id: String,
    pub pages: Vec<config::PageConfig>,
    pub display: crate::display::dto::DisplayConfigDto,
    pub watch_targets: Vec<PathBuf>,
    pub podman: Option<config::DynamicPodmanConfig>,
}

/// 指定 ID のページが設定中に存在するか確認する。
pub fn page_exists(config: &RuntimeConfig, page_id: &str) -> bool {
    config.pages.iter().any(|p| p.id == page_id)
}

/// ホームページ ID を返す。存在しない場合は最初のページ ID を返す。
pub fn fallback_page_id(config: &RuntimeConfig) -> Option<String> {
    if page_exists(config, &config.home_page_id) {
        return Some(config.home_page_id.clone());
    }
    config.pages.first().map(|p| p.id.clone())
}

// ── ConfigManager ─────────────────────────────────────────────────

/// 設定のアクティブ値と監視状態を管理するコンポーネント。
/// ファイル変更通知を受けてデバウンス付きで設定を再読込する。
pub struct ConfigManager {
    pub config_path: String,
    pub active: RuntimeConfig,
    pub reload_err_count: u32,
    pub last_reload_at: Instant,
    pub watch_targets: Vec<PathBuf>,
    #[cfg(feature = "file-watch")]
    pub watcher: Option<FileWatcher>,
}

impl ConfigManager {
    pub fn new(config_path: &str, initial: RuntimeConfig) -> Self {
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

    pub fn current(&self) -> &RuntimeConfig {
        &self.active
    }

    pub fn poll_reload(&mut self) -> bool {
        #[cfg(feature = "file-watch")]
        {
            let has_signal = self.watcher.as_ref().is_some_and(|w| w.has_pending());
            if has_signal && self.last_reload_at.elapsed() >= RELOAD_DEBOUNCE {
                return self.do_reload();
            }
        }
        false
    }

    pub fn do_reload(&mut self) -> bool {
        self.last_reload_at = Instant::now();
        match crate::load_runtime_config(&self.config_path) {
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

    /// Podman ページを差し替える。外部からページを直接書き換える代わりにこのメソッドを使う。
    pub fn update_podman_pages(&mut self, prefix: &str, new_pages: Vec<config::PageConfig>) {
        self.active
            .pages
            .retain(|page| !page.id.starts_with(prefix));
        self.active.pages.extend(new_pages);
    }
}

// ── FileWatcher ──────────────────────────────────────────────────

#[cfg(feature = "file-watch")]
pub fn create_file_watcher(targets: &[PathBuf]) -> Option<FileWatcher> {
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
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<()>,
}

#[cfg(feature = "file-watch")]
impl FileWatcher {
    pub fn new(targets: &[PathBuf]) -> anyhow::Result<Self> {
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

    pub fn has_pending(&self) -> bool {
        let mut any = false;
        while self.receiver.try_recv().is_ok() {
            any = true;
        }
        any
    }
}

pub fn normalize_path(path: &Path) -> PathBuf {
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
