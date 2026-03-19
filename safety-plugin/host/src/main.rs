//! ロボット制御ホストプロセス。
//!
//! プラグインのライフサイクルを管理し、axum HTTP サーバーとして稼働する。
//! プラグインがパニックしてもフォールバックレスポンスで動作を継続する。
//!
//! # リロードトリガー
//! - ファイル監視（`notify`）: プラグイン .so の変更を検知して自動リロード。
//! - `SIGUSR1`: 手動でリロードをトリガーする。
//!
//! # HTTP ルーティング
//! 全リクエストは `PluginManager::handle()` に委譲される。
//! プラグインが宣言したパスプレフィックスにマッチする場合のみプラグインが処理し、
//! それ以外は 404 / 503 を返す。

use safety_plugin_host::plugin_manager::PluginManager;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::Context;
use axum::{extract::State, response::Response, routing::any, Router};
use clap::Parser;
use http_body_util::BodyExt;
use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use tokio::sync::mpsc;

/// ホストプロセスのコマンドライン引数。
#[derive(Parser)]
#[command(name = "safety-plugin-host", about = "ロボット制御ホストプロセス（HTTPサーバー）")]
struct Cli {
    /// ロードするプラグイン (.so) のパス。
    #[arg(long, short = 'p')]
    plugin: PathBuf,

    /// HTTP サーバーのリッスンアドレス。
    #[arg(long, default_value = "0.0.0.0:8080")]
    addr: String,
}

/// リロードイベント。
enum ReloadEvent {
    /// 指定パスのプラグインをリロードする。
    Reload(PathBuf),
    /// プロセスを終了する。
    Shutdown,
}

type SharedManager = Arc<Mutex<PluginManager>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("safety_plugin_host=debug".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    // プラグインマネージャを初期化してプラグインをロード
    let manager: SharedManager = Arc::new(Mutex::new(PluginManager::default()));
    if let Err(e) = manager.lock().unwrap().load(&cli.plugin) {
        tracing::error!("初回プラグインロード失敗: {e}");
        tracing::warn!("フォールバックモードで起動します");
    }

    let (tx, mut rx) = mpsc::channel::<ReloadEvent>(32);

    // ファイル監視スレッドを起動する
    let _watcher = setup_file_watcher(tx.clone(), &cli.plugin)?;

    // SIGUSR1 ハンドラを起動する
    setup_sigusr1(tx.clone(), cli.plugin.clone());

    // SIGINT / SIGTERM でシャットダウンする
    setup_shutdown_signal(tx.clone());

    // axum HTTP サーバーを起動する
    let mgr_for_server = manager.clone();
    let addr = cli.addr.clone();
    let server_handle = tokio::spawn(async move {
        let app = Router::new()
            .fallback(any(plugin_handler))
            .with_state(mgr_for_server);
        tracing::info!("HTTP サーバー起動: http://{addr}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("アドレスのバインドに失敗");
        axum::serve(listener, app)
            .await
            .expect("HTTP サーバーエラー");
    });

    // リロードイベントループ
    while let Some(event) = rx.recv().await {
        match event {
            ReloadEvent::Reload(path) => {
                tracing::info!("リロードイベントを受信: {}", path.display());
                if let Err(e) = manager
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .reload(&path)
                {
                    tracing::error!("リロード失敗、旧バージョンで継続: {e}");
                }
            }
            ReloadEvent::Shutdown => {
                tracing::info!("シャットダウンします");
                server_handle.abort();
                break;
            }
        }
    }

    tracing::info!("ホストプロセス終了");
    Ok(())
}

/// axum のリクエストハンドラ。全リクエストを PluginManager へ委譲する。
async fn plugin_handler(
    State(mgr): State<SharedManager>,
    req: axum::extract::Request,
) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // ボディを収集する（最大 16MiB）
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();

    let http_req = HttpRequest {
        method: method.into(),
        path: path.into(),
        query: query.into(),
        body: RVec::from(body_bytes.to_vec()),
    };

    let resp = mgr
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle(&http_req);

    Response::builder()
        .status(resp.status)
        .header("content-type", resp.content_type.as_str())
        .body(axum::body::Body::from(resp.body.into_vec()))
        .unwrap()
}

/// ファイル監視を設定する。プラグイン .so の変更を検知してリロードイベントを送信する。
fn setup_file_watcher(
    tx: mpsc::Sender<ReloadEvent>,
    plugin_path: &std::path::Path,
) -> anyhow::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc as std_mpsc;

    let plugin_path = plugin_path.to_path_buf();
    let (ntx, nrx) = std_mpsc::channel();

    let mut watcher = notify::RecommendedWatcher::new(ntx, notify::Config::default())
        .context("ファイル監視の初期化に失敗")?;

    // プラグインが置かれているディレクトリを監視する
    let watch_dir = plugin_path.parent().unwrap_or(std::path::Path::new("."));
    watcher
        .watch(watch_dir, RecursiveMode::NonRecursive)
        .context("ディレクトリの監視開始に失敗")?;

    std::thread::spawn(move || {
        for event in nrx.into_iter().flatten() {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let target = event.paths.iter().any(|p| p == &plugin_path);
                if target {
                    tracing::info!("プラグインファイルの変更を検知: {}", plugin_path.display());
                    let _ = tx.blocking_send(ReloadEvent::Reload(plugin_path.clone()));
                }
            }
        }
    });

    Ok(watcher)
}

/// SIGUSR1 シグナルハンドラを設定する。受信時にリロードイベントを送信する。
fn setup_sigusr1(tx: mpsc::Sender<ReloadEvent>, plugin_path: PathBuf) {
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
            let _ = tx.blocking_send(ReloadEvent::Reload(plugin_path.clone()));
        }
    });
}

/// SIGINT / SIGTERM シグナルハンドラを設定する。受信時にシャットダウンイベントを送信する。
fn setup_shutdown_signal(tx: mpsc::Sender<ReloadEvent>) {
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
            let _ = tx.blocking_send(ReloadEvent::Shutdown);
        }
    });
}
