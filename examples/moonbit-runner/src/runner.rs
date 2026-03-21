//! MoonBit Wasm プラグインへ HTTP 経由で処理を委譲するランナー
//!
//! # リロードトリガー
//! - ファイル監視（`notify`）: Wasm ファイルの変更を検知して自動リロード。
//! - `SIGUSR1`: プラグインを手動リロードする。
//! - SIGINT / SIGTERM: グレースフルシャットダウン。

use anyhow::{bail, ensure, Context};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::bindings::{MotorOutput, PluginStatus, SensorData};
use crate::plugin_manager::PluginManager;

/// 利用する WASI サポートの種類
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum WasiSupport {
    #[default]
    None,
    Preview2,
}

impl WasiSupport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Preview2 => "preview2",
        }
    }
}

/// サーバー起動設定
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerConfig {
    /// 読み込む Wasm コンポーネントのパス
    pub wasm: PathBuf,
    /// 利用を要求する WASI サポート
    pub wasi: WasiSupport,
    /// HTTP サーバー待ち受けアドレス
    pub bind_addr: SocketAddr,
    /// Wasm プラグインへ委譲する URL プレフィックス
    pub plugin_prefix: String,
}

impl RunnerConfig {
    /// 実行前に最低限の設定値を検証する
    pub fn validate(&self) -> anyhow::Result<()> {
        let _ = normalize_plugin_prefix(&self.plugin_prefix)?;
        Ok(())
    }
}

/// リロードイベント
enum ReloadEvent {
    /// Wasm プラグインをリロードする
    Reload { path: PathBuf },
    /// プロセスを終了する
    Shutdown,
}

struct AppState {
    manager: Arc<Mutex<PluginManager>>,
    plugin_prefix: String,
    wasi: WasiSupport,
}

#[derive(Debug, Clone, Serialize)]
struct ServerStatusResponse {
    service: &'static str,
    plugin_prefix: String,
    wasm: String,
    wasi: &'static str,
    loaded: bool,
    fallback_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct SensorDataRequest {
    load: f32,
    position: f32,
    extra: Option<f32>,
}

impl From<SensorDataRequest> for SensorData {
    fn from(value: SensorDataRequest) -> Self {
        Self {
            load: value.load,
            position: value.position,
            extra: value.extra,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct MotorOutputResponse {
    position: f32,
    torque: f32,
}

impl From<MotorOutput> for MotorOutputResponse {
    fn from(value: MotorOutput) -> Self {
        Self {
            position: value.position,
            torque: value.torque,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct PluginStatusResponse {
    running: bool,
    error_code: u32,
    temperature: f32,
}

impl From<PluginStatus> for PluginStatusResponse {
    fn from(value: PluginStatus) -> Self {
        Self {
            running: value.running,
            error_code: value.error_code,
            temperature: value.temperature,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateRequest {
    input: Vec<SensorDataRequest>,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateResponse {
    output: Vec<MotorOutputResponse>,
}

type HttpError = (axum::http::StatusCode, String);

type HttpResult<T> = Result<T, HttpError>;

/// HTTP サーバーを起動し、指定パス以下の処理を Wasm プラグインへ委譲する
pub fn serve_http(config: RunnerConfig) -> anyhow::Result<()> {
    use axum::{routing::get, routing::post, Router};
    use tokio::net::TcpListener;

    config.validate()?;
    ensure_supported_wasi(config.wasi)?;

    let plugin_prefix = normalize_plugin_prefix(&config.plugin_prefix)?;

    let mut mgr = PluginManager::new()?;
    mgr.load(&config.wasm).context("初回プラグインロード失敗")?;

    let manager = Arc::new(Mutex::new(mgr));

    let state = Arc::new(AppState {
        manager: manager.clone(),
        plugin_prefix: plugin_prefix.clone(),
        wasi: config.wasi,
    });

    let plugin_router = Router::new()
        .route("/status", get(handle_plugin_status))
        .route("/update", post(handle_plugin_update));

    let app = Router::new()
        .route("/status", get(handle_server_status))
        .nest(&plugin_prefix, plugin_router)
        .with_state(state);

    let (tx, mut rx) = mpsc::channel::<ReloadEvent>(32);

    // ファイル監視（失敗してもサーバー起動は継続）
    let _watcher = match setup_file_watcher(tx.clone(), config.wasm.clone()) {
        Ok(w) => {
            tracing::info!("ファイル監視を開始: {}", config.wasm.display());
            Some(w)
        }
        Err(e) => {
            tracing::warn!("ファイル監視の開始に失敗（無効化）: {e}");
            None
        }
    };

    setup_sigusr1(tx.clone(), config.wasm.clone());
    setup_shutdown_signal(tx.clone());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("tokio ランタイムの構築に失敗しました")?;

    rt.block_on(async move {
        let listener = TcpListener::bind(config.bind_addr)
            .await
            .with_context(|| format!("アドレス {} への bind に失敗しました", config.bind_addr))?;
        println!("サーバー起動: http://{}/status", config.bind_addr);
        println!(
            "Wasm 委譲エンドポイント: http://{}{}/*",
            config.bind_addr, plugin_prefix
        );

        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("HTTP サーバーがエラー終了しました");
        });

        // リロードイベントループ
        while let Some(event) = rx.recv().await {
            match event {
                ReloadEvent::Reload { path } => {
                    tracing::info!("リロードイベント受信: {}", path.display());
                    if let Err(e) = manager
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .reload(&path)
                    {
                        tracing::error!("リロード失敗、旧バージョンで継続: {e:#}");
                    }
                }
                ReloadEvent::Shutdown => {
                    tracing::info!("シャットダウンします");
                    server_handle.abort();
                    break;
                }
            }
        }
        Ok(())
    })
}

fn normalize_plugin_prefix(prefix: &str) -> anyhow::Result<String> {
    let trimmed = prefix.trim();
    ensure!(!trimmed.is_empty(), "plugin_prefix は空にできません");

    let prefixed = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    let normalized = prefixed.trim_end_matches('/').to_string();

    ensure!(
        !normalized.is_empty(),
        "plugin_prefix に `/` は指定できません"
    );
    Ok(normalized)
}

fn ensure_supported_wasi(wasi: WasiSupport) -> anyhow::Result<()> {
    match wasi {
        WasiSupport::None => Ok(()),
        WasiSupport::Preview2 => {
            bail!("WASI Preview2 は moonbit-runner では未対応です")
        }
    }
}

// ─── HTTP ハンドラ ─────────────────────────────────────────────────────────

async fn handle_server_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::Json<ServerStatusResponse> {
    let mgr = state.manager.lock().unwrap_or_else(|e| e.into_inner());
    let wasm = mgr
        .current_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let loaded = mgr.is_loaded();
    let fallback_count = mgr.fallback_count;
    drop(mgr);

    axum::Json(ServerStatusResponse {
        service: "moonbit-runner",
        plugin_prefix: state.plugin_prefix.clone(),
        wasm,
        wasi: state.wasi.as_str(),
        loaded,
        fallback_count,
    })
}

async fn handle_plugin_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> HttpResult<axum::Json<PluginStatusResponse>> {
    let mut mgr = state.manager.lock().map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "プラグイン状態ロックの取得に失敗しました".to_string(),
        )
    })?;

    let status = mgr.get_status().map_err(|err| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("Wasm プラグインの status 取得に失敗しました: {err}"),
        )
    })?;

    Ok(axum::Json(PluginStatusResponse::from(status)))
}

async fn handle_plugin_update(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    axum::Json(request): axum::Json<UpdateRequest>,
) -> HttpResult<axum::Json<UpdateResponse>> {
    let input: Vec<SensorData> = request.input.into_iter().map(SensorData::from).collect();

    let mut mgr = state.manager.lock().map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "プラグイン状態ロックの取得に失敗しました".to_string(),
        )
    })?;

    let outputs = mgr.update(&input).map_err(|err| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("Wasm プラグインの update 呼び出しに失敗しました: {err}"),
        )
    })?;

    let output = outputs.into_iter().map(MotorOutputResponse::from).collect();
    Ok(axum::Json(UpdateResponse { output }))
}

// ─── シグナル・ファイル監視 ───────────────────────────────────────────────────

fn setup_file_watcher(
    tx: mpsc::Sender<ReloadEvent>,
    wasm_path: PathBuf,
) -> anyhow::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc as std_mpsc;

    let (ntx, nrx) = std_mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(ntx, notify::Config::default())
        .context("ファイル監視の初期化に失敗")?;

    let watch_dir = wasm_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .context("ディレクトリの監視開始に失敗")?;

    std::thread::spawn(move || {
        for event in nrx.into_iter().flatten() {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
                && event.paths.iter().any(|p| p == &wasm_path) {
                    tracing::info!("Wasm ファイルの変更を検知: {}", wasm_path.display());
                    let _ = tx.blocking_send(ReloadEvent::Reload {
                        path: wasm_path.clone(),
                    });
                }
        }
    });

    Ok(watcher)
}

/// SIGUSR1 受信でプラグインをリロードする
fn setup_sigusr1(tx: mpsc::Sender<ReloadEvent>, wasm_path: PathBuf) {
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
            tracing::info!("SIGUSR1 を受信しました。プラグインをリロードします");
            let _ = tx.blocking_send(ReloadEvent::Reload {
                path: wasm_path.clone(),
            });
        }
    });
}

/// SIGINT / SIGTERM でグレースフルシャットダウンする
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

#[cfg(test)]
mod tests {
    use super::*;

    struct PrefixCase {
        name: &'static str,
        input: &'static str,
        expect_ok: bool,
        expected: &'static str,
    }

    fn assert_prefix_case(case: PrefixCase) {
        let result = normalize_plugin_prefix(case.input);
        assert_eq!(
            result.is_ok(),
            case.expect_ok,
            "prefix ケース `{}` の成否が想定と異なります",
            case.name
        );

        match result {
            Ok(actual) => {
                assert_eq!(
                    actual, case.expected,
                    "prefix ケース `{}` の正規化結果が想定と異なります",
                    case.name
                );
            }
            Err(err) => {
                assert!(
                    err.to_string().contains(case.expected),
                    "prefix ケース `{}` のエラー内容が想定と異なります: {}",
                    case.name,
                    err
                );
            }
        }
    }

    #[test]
    fn prefix正規化_値域確認() {
        let cases = [
            PrefixCase {
                name: "1文字プレフィックス",
                input: "a",
                expect_ok: true,
                expected: "/a",
            },
            PrefixCase {
                name: "前後空白付き",
                input: "  /api  ",
                expect_ok: true,
                expected: "/api",
            },
            PrefixCase {
                name: "末尾スラッシュ付き",
                input: "/service/",
                expect_ok: true,
                expected: "/service",
            },
        ];

        for case in cases {
            assert_prefix_case(case);
        }
    }

    #[test]
    fn prefix正規化_正常系() {
        let cases = [
            PrefixCase {
                name: "先頭スラッシュなし",
                input: "api/v1",
                expect_ok: true,
                expected: "/api/v1",
            },
            PrefixCase {
                name: "先頭スラッシュあり",
                input: "/api/v1",
                expect_ok: true,
                expected: "/api/v1",
            },
        ];

        for case in cases {
            assert_prefix_case(case);
        }
    }

    #[test]
    fn prefix正規化_異常系() {
        let cases = [
            PrefixCase {
                name: "空文字",
                input: "",
                expect_ok: false,
                expected: "plugin_prefix は空にできません",
            },
            PrefixCase {
                name: "ルートのみ",
                input: "/",
                expect_ok: false,
                expected: "plugin_prefix に `/` は指定できません",
            },
            PrefixCase {
                name: "空白のみ",
                input: "   ",
                expect_ok: false,
                expected: "plugin_prefix は空にできません",
            },
        ];

        for case in cases {
            assert_prefix_case(case);
        }
    }
}
