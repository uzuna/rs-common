//! ロボット制御ホストプロセス。
//!
//! プラグインのライフサイクルを管理し、24/365稼働する基盤として機能する。
//! プラグインがパニックしてもフォールバックロジックで動作を継続する。
//!
//! # リロードトリガー
//! - ファイル監視（`notify`）: プラグイン .so の変更を検知して自動リロード。
//! - `SIGUSR1`: 手動でリロードをトリガーする。

use safety_plugin_host::plugin_manager;

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use anyhow::Context;
use clap::Parser;
use plugin_manager::{PluginManager, StepResult};

/// ホストプロセスのコマンドライン引数。
#[derive(Parser)]
#[command(name = "safety-plugin-host", about = "ロボット制御ホストプロセス")]
struct Cli {
    /// ロードするプラグイン (.so) のパス。
    #[arg(long, short = 'p')]
    plugin: PathBuf,

    /// 制御ループの実行間隔（ミリ秒）。
    #[arg(long, default_value = "10")]
    interval_ms: u64,
}

/// リロードイベント。
enum ReloadEvent {
    /// 指定パスのプラグインをリロードする。
    Reload(PathBuf),
    /// プロセスを終了する。
    Shutdown,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("safety_plugin_host=debug".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    let (tx, rx) = mpsc::channel::<ReloadEvent>();

    // ファイル監視スレッドを起動する
    let _watcher = setup_file_watcher(tx.clone(), &cli.plugin)?;

    // SIGUSR1 ハンドラを起動する
    setup_sigusr1(tx.clone(), cli.plugin.clone());

    // SIGINT / SIGTERM でシャットダウンする
    setup_shutdown_signal(tx.clone());

    run_loop(&cli.plugin, cli.interval_ms, rx)
}

/// メイン制御ループ。
fn run_loop(
    initial_plugin: &std::path::Path,
    interval_ms: u64,
    rx: Receiver<ReloadEvent>,
) -> anyhow::Result<()> {
    let mut manager = PluginManager::default();
    let interval = Duration::from_millis(interval_ms);

    // 初回ロード
    if let Err(e) = manager.load(initial_plugin) {
        tracing::error!("初回プラグインロード失敗: {e}");
        tracing::warn!("フォールバックモードで起動します");
    }

    tracing::info!("制御ループ開始（間隔: {}ms）", interval_ms);

    loop {
        // リロードイベントを非ブロッキングで確認する
        match rx.try_recv() {
            Ok(ReloadEvent::Reload(path)) => {
                tracing::info!("リロードイベントを受信: {}", path.display());
                if let Err(e) = manager.reload(&path) {
                    tracing::error!("リロード失敗、フォールバックモードで継続: {e}");
                }
            }
            Ok(ReloadEvent::Shutdown) => {
                tracing::info!("シャットダウンします");
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::warn!("イベントチャネルが切断されました");
                break;
            }
        }

        // 制御ループの1ステップ
        let result = manager.step();
        if result == StepResult::Fallback {
            tracing::debug!("フォールバック実行中（累計: {}回）", manager.fallback_count);
        }

        std::thread::sleep(interval);
    }

    tracing::info!("制御ループ終了");
    Ok(())
}

/// ファイル監視を設定する。プラグイン .so の変更を検知してリロードイベントを送信する。
fn setup_file_watcher(
    tx: Sender<ReloadEvent>,
    plugin_path: &std::path::Path,
) -> anyhow::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};

    let plugin_path = plugin_path.to_path_buf();
    let (ntx, nrx) = mpsc::channel();

    let mut watcher = notify::RecommendedWatcher::new(ntx, notify::Config::default())
        .context("ファイル監視の初期化に失敗")?;

    // プラグインが置かれているディレクトリを監視する
    let watch_dir = plugin_path.parent().unwrap_or(std::path::Path::new("."));
    watcher
        .watch(watch_dir, RecursiveMode::NonRecursive)
        .context("ディレクトリの監視開始に失敗")?;

    std::thread::spawn(move || {
        for event in nrx.into_iter().flatten() {
            // ファイルの変更のみを対象とし、対象ファイルが監視中のプラグインパスと一致するか確認する
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let target = event.paths.iter().any(|p| p == &plugin_path);
                if target {
                    tracing::info!("プラグインファイルの変更を検知: {}", plugin_path.display());
                    let _ = tx.send(ReloadEvent::Reload(plugin_path.clone()));
                }
            }
        }
    });

    Ok(watcher)
}

/// SIGUSR1 シグナルハンドラを設定する。受信時にリロードイベントを送信する。
fn setup_sigusr1(tx: Sender<ReloadEvent>, plugin_path: PathBuf) {
    use signal_hook::{consts::SIGUSR1, iterator::Signals};

    std::thread::spawn(move || {
        let mut signals = match Signals::new([SIGUSR1]) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("SIGUSR1 ハンドラの設定に失敗: {e}");
                return;
            }
        };
        for _ in signals.forever() {
            tracing::info!("SIGUSR1 を受信しました。リロードします");
            let _ = tx.send(ReloadEvent::Reload(plugin_path.clone()));
        }
    });
}

/// SIGINT / SIGTERM シグナルハンドラを設定する。受信時にシャットダウンイベントを送信する。
fn setup_shutdown_signal(tx: Sender<ReloadEvent>) {
    use signal_hook::{consts::SIGINT, consts::SIGTERM, iterator::Signals};

    std::thread::spawn(move || {
        let mut signals = match Signals::new([SIGINT, SIGTERM]) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("シャットダウンシグナルハンドラの設定に失敗: {e}");
                return;
            }
        };
        for _ in signals.forever() {
            tracing::info!("シャットダウンシグナルを受信しました");
            let _ = tx.send(ReloadEvent::Shutdown);
        }
    });
}
