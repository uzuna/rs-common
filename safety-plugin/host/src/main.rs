//! ロボット制御ホストプロセス。
//!
//! プラグインのライフサイクルを管理し、axum HTTP サーバーとして稼働する。
//! プラグインがパニックしてもフォールバックレスポンスで動作を継続する。
//!
//! # リロードトリガー
//! - ファイル監視（`notify`）: プラグイン .so の変更を検知して自動リロード。
//! - `SIGUSR1`: 登録済み全プラグインを手動リロードする。
//! - HTTP API `POST /plugin/{prefix}/reload`: バイナリをアップロードしてリロード。
//!
//! # HTTP ルーティング
//! - `/plugin/...` 以下はプラグイン管理 API が担当する。
//! - それ以外のパスは `PluginRouter::handle()` に委譲される（最長プレフィックス一致）。
//!
//! # CLI 引数
//! `--plugin <prefix>:<path>` 形式でプレフィックスとプラグインパスを指定する。
//! 例: `--plugin /api:target/debug/libexample.so --plugin /sample:target/debug/libsample.so`

use safety_plugin_host::{
    plugin_manager::PluginRouter,
    version_manager::{PluginVersion, VersionManager},
};

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use abi_stable::std_types::RVec;
use anyhow::Context;
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use clap::Parser;
use http_body_util::BodyExt;
use safety_plugin_common::HttpRequest;
use serde_json::json;
use tokio::sync::mpsc;

// ─── CLI 引数 ─────────────────────────────────────────────────────────────────

/// ホストプロセスのコマンドライン引数。
#[derive(Parser)]
#[command(
    name = "safety-plugin-host",
    about = "ロボット制御ホストプロセス（HTTP サーバー）"
)]
struct Cli {
    /// ロードするプラグインの指定（複数指定可）。形式: `<prefix>:<path>`
    /// 例: `--plugin /api:target/debug/libexample.so`
    #[arg(long, short = 'p')]
    plugin: Vec<String>,

    /// HTTP サーバーのリッスンアドレス。
    #[arg(long, default_value = "0.0.0.0:8080")]
    addr: String,

    /// バージョン管理用のストレージディレクトリ。
    #[arg(long, default_value = "plugin-versions")]
    plugin_dir: PathBuf,

    /// 保持するバージョン数の上限。
    #[arg(long, default_value_t = 10)]
    max_versions: usize,
}

/// `<prefix>:<path>` 形式の文字列を分解する。
fn parse_plugin_arg(s: &str) -> anyhow::Result<(String, PathBuf)> {
    let (prefix, path) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--plugin の形式が不正です（`prefix:path` が必要）: {s}"))?;
    anyhow::ensure!(!prefix.is_empty(), "prefix が空文字列です: {s}");
    anyhow::ensure!(!path.is_empty(), "path が空文字列です: {s}");
    Ok((prefix.to_string(), PathBuf::from(path)))
}

// ─── アプリケーション状態 ────────────────────────────────────────────────────

/// リロードイベント。
enum ReloadEvent {
    /// 指定プレフィックスのプラグインをリロードする。
    Reload { prefix: String, path: PathBuf },
    /// プロセスを終了する。
    Shutdown,
}

/// axum の共有アプリケーション状態。
struct AppState {
    router: Mutex<PluginRouter>,
    /// プレフィックスごとのバージョンマネージャ。
    versions: Mutex<HashMap<String, VersionManager>>,
    /// バージョン管理ディレクトリのルート。
    plugin_dir: PathBuf,
    max_versions: usize,
}

type SharedState = Arc<AppState>;

// ─── main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("safety_plugin_host=debug".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    // CLI 引数をパース
    let mut plugin_specs: Vec<(String, PathBuf)> = Vec::new();
    for s in &cli.plugin {
        plugin_specs.push(parse_plugin_arg(s).context("--plugin 引数のパースに失敗")?);
    }

    let state: SharedState = Arc::new(AppState {
        router: Mutex::new(PluginRouter::default()),
        versions: Mutex::new(HashMap::new()),
        plugin_dir: cli.plugin_dir.clone(),
        max_versions: cli.max_versions,
    });

    // バージョンマネージャを prefix ごとに初期化
    for (prefix, _) in &plugin_specs {
        let dir = prefix_to_dir(&cli.plugin_dir, prefix);
        let vm = VersionManager::new(dir, cli.max_versions)
            .with_context(|| format!("バージョンマネージャの初期化に失敗: {prefix}"))?;
        state
            .versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(prefix.clone(), vm);
    }

    // 初回プラグインロード
    if plugin_specs.is_empty() {
        tracing::warn!("--plugin が未指定のためフォールバックモードで起動します");
    }
    for (prefix, path) in &plugin_specs {
        if let Err(e) = state
            .router
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .load(prefix, path)
        {
            tracing::error!("初回プラグインロード失敗 ({prefix}): {e}");
            tracing::warn!("プレフィックス {prefix} はフォールバックモードで起動します");
        }
    }

    let (tx, mut rx) = mpsc::channel::<ReloadEvent>(32);

    // ファイル監視スレッドを起動（各プラグインのファイルを監視）
    let mut _watchers = Vec::new();
    for (prefix, path) in &plugin_specs {
        match setup_file_watcher(tx.clone(), prefix.clone(), path) {
            Ok(w) => _watchers.push(w),
            Err(e) => tracing::warn!("ファイル監視の開始に失敗 ({prefix}): {e}"),
        }
    }

    // SIGUSR1 で全プラグインをリロード
    if !plugin_specs.is_empty() {
        setup_sigusr1(tx.clone(), plugin_specs.clone());
    }

    // SIGINT / SIGTERM でシャットダウン
    setup_shutdown_signal(tx.clone());

    // axum HTTP サーバーを起動する
    let app = Router::new()
        // プラグイン管理 API（prefix は単一パスセグメント: "api", "sample" 等）
        .route("/plugin/prefixes", get(api_prefixes))
        .route("/plugin/{prefix}/reload", post(api_reload))
        .route("/plugin/{prefix}/status", get(api_status))
        .route("/plugin/{prefix}/versions", get(api_versions))
        .route("/plugin/{prefix}/rollback/{version}", post(api_rollback))
        // それ以外は全てプラグインルーターへ委譲
        .fallback(any(plugin_handler))
        .with_state(state.clone());

    let addr = cli.addr.clone();
    let server_handle = tokio::spawn(async move {
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
            ReloadEvent::Reload { prefix, path } => {
                tracing::info!("リロードイベントを受信: {} → {}", prefix, path.display());
                if let Err(e) = state
                    .router
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .reload(&prefix, &path)
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

// ─── プラグイン管理 API ───────────────────────────────────────────────────────

/// GET /plugin/prefixes
///
/// 登録済みプレフィックス一覧を返す。
async fn api_prefixes(State(state): State<SharedState>) -> Response {
    let prefixes = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .prefixes();
    Json(json!({ "prefixes": prefixes })).into_response()
}

/// POST /plugin/{prefix}/reload
///
/// .so バイナリをボディで受け取り、バージョンとして保存してプラグインをリロードする。
/// `{prefix}` は CLI 指定時の先頭 `/` を除いたもの（例: `api`, `sample`）。
async fn api_reload(
    State(state): State<SharedState>,
    Path(prefix_param): Path<String>,
    request: Request,
) -> Response {
    let prefix = format!("/{prefix_param}");

    let body_bytes = match request.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                format!("ボディの読み込みに失敗: {e}"),
            )
        }
    };

    if body_bytes.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "ボディが空です");
    }

    // バージョンとして保存
    let (version, path) = {
        let mut versions = state.versions.lock().unwrap_or_else(|e| e.into_inner());
        let vm = match get_or_create_vm(
            &mut versions,
            &prefix,
            &state.plugin_dir,
            state.max_versions,
        ) {
            Ok(vm) => vm,
            Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        let v = match vm.save(&body_bytes) {
            Ok(v) => v,
            Err(e) => {
                return api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("保存に失敗: {e}"),
                )
            }
        };
        let p = vm.path_of(v).unwrap().to_path_buf();
        (v, p)
    };

    let reload_ok = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reload(&prefix, &path)
        .is_ok();

    let status = if reload_ok { "loaded" } else { "fallback" };
    tracing::info!("API リロード ({prefix}): v{version} → {status}");

    Json(json!({
        "prefix": prefix,
        "version": version,
        "status": status,
    }))
    .into_response()
}

/// GET /plugin/{prefix}/status
///
/// 指定プレフィックスのプラグイン状態を返す。
async fn api_status(
    State(state): State<SharedState>,
    Path(prefix_param): Path<String>,
) -> Response {
    let prefix = format!("/{prefix_param}");

    let loaded = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_loaded(&prefix);

    let fallback_count = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fallback_count;

    let current_version = state
        .versions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&prefix)
        .and_then(|vm| vm.current_version());

    Json(json!({
        "prefix": prefix,
        "loaded": loaded,
        "version": current_version,
        "fallback_count": fallback_count,
    }))
    .into_response()
}

/// GET /plugin/{prefix}/versions
///
/// 指定プレフィックスのバージョン一覧を返す。
async fn api_versions(
    State(state): State<SharedState>,
    Path(prefix_param): Path<String>,
) -> Response {
    let prefix = format!("/{prefix_param}");

    let versions_lock = state.versions.lock().unwrap_or_else(|e| e.into_inner());
    let (versions_json, current) = match versions_lock.get(&prefix) {
        Some(vm) => (
            vm.list().iter().map(version_to_json).collect::<Vec<_>>(),
            vm.current_version(),
        ),
        None => (Vec::new(), None),
    };

    Json(json!({
        "prefix": prefix,
        "current": current,
        "versions": versions_json,
    }))
    .into_response()
}

/// POST /plugin/{prefix}/rollback/{version}
///
/// 指定バージョンのプラグインに切り替える。
async fn api_rollback(
    State(state): State<SharedState>,
    Path((prefix_param, version)): Path<(String, u64)>,
) -> Response {
    let prefix = format!("/{prefix_param}");

    let path = {
        let versions = state.versions.lock().unwrap_or_else(|e| e.into_inner());
        match versions.get(&prefix).and_then(|vm| vm.path_of(version)) {
            Some(p) => p.to_path_buf(),
            None => {
                return api_error(
                    StatusCode::NOT_FOUND,
                    format!("バージョン {version} は存在しません (prefix: {prefix})"),
                )
            }
        }
    };

    let reload_ok = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reload(&prefix, &path)
        .is_ok();

    if reload_ok {
        if let Some(vm) = state
            .versions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&prefix)
        {
            vm.mark_current(version);
        }
    }

    let status = if reload_ok { "loaded" } else { "fallback" };
    tracing::info!("API ロールバック ({prefix}): v{version} → {status}");

    Json(json!({
        "prefix": prefix,
        "version": version,
        "status": status,
    }))
    .into_response()
}

// ─── プラグイン委譲ハンドラ ───────────────────────────────────────────────────

/// axum のフォールバックハンドラ。全リクエストを PluginRouter へ委譲する。
async fn plugin_handler(State(state): State<SharedState>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

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

    let resp = state
        .router
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle(http_req);

    Response::builder()
        .status(resp.status)
        .header("content-type", resp.content_type.as_str())
        .body(axum::body::Body::from(resp.body.into_vec()))
        .unwrap()
}

// ─── ユーティリティ ───────────────────────────────────────────────────────────

/// エラーレスポンスを生成する。
fn api_error(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({"error": msg.into()}))).into_response()
}

/// `PluginVersion` を JSON 値に変換する。
fn version_to_json(v: &PluginVersion) -> serde_json::Value {
    json!({
        "version": v.version,
        "saved_at": v.saved_at,
        "path": v.path.display().to_string(),
    })
}

/// プレフィックスをディレクトリ名に変換する（`/api` → `plugin_dir/api`）。
fn prefix_to_dir(plugin_dir: &std::path::Path, prefix: &str) -> PathBuf {
    let seg = prefix.trim_start_matches('/').replace('/', "_");
    plugin_dir.join(seg)
}

/// HashMap にプレフィックスの `VersionManager` がなければ作成して返す。
fn get_or_create_vm<'a>(
    versions: &'a mut HashMap<String, VersionManager>,
    prefix: &str,
    plugin_dir: &std::path::Path,
    max_versions: usize,
) -> anyhow::Result<&'a mut VersionManager> {
    if !versions.contains_key(prefix) {
        let dir = prefix_to_dir(plugin_dir, prefix);
        let vm = VersionManager::new(dir, max_versions)?;
        versions.insert(prefix.to_string(), vm);
    }
    Ok(versions.get_mut(prefix).unwrap())
}

// ─── シグナル・ファイル監視 ───────────────────────────────────────────────────

fn setup_file_watcher(
    tx: mpsc::Sender<ReloadEvent>,
    prefix: String,
    plugin_path: &std::path::Path,
) -> anyhow::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::mpsc as std_mpsc;

    let plugin_path = plugin_path.to_path_buf();
    let (ntx, nrx) = std_mpsc::channel();

    let mut watcher = notify::RecommendedWatcher::new(ntx, notify::Config::default())
        .context("ファイル監視の初期化に失敗")?;

    let watch_dir = plugin_path.parent().unwrap_or(std::path::Path::new("."));
    watcher
        .watch(watch_dir, RecursiveMode::NonRecursive)
        .context("ディレクトリの監視開始に失敗")?;

    std::thread::spawn(move || {
        for event in nrx.into_iter().flatten() {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let target = event.paths.iter().any(|p| p == &plugin_path);
                if target {
                    tracing::info!(
                        "プラグインファイルの変更を検知 ({}): {}",
                        prefix,
                        plugin_path.display()
                    );
                    let _ = tx.blocking_send(ReloadEvent::Reload {
                        prefix: prefix.clone(),
                        path: plugin_path.clone(),
                    });
                }
            }
        }
    });

    Ok(watcher)
}

/// SIGUSR1 受信で全プラグインをリロードする。
fn setup_sigusr1(tx: mpsc::Sender<ReloadEvent>, plugins: Vec<(String, PathBuf)>) {
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
            tracing::info!("SIGUSR1 を受信しました。全プラグインをリロードします");
            for (prefix, path) in &plugins {
                let _ = tx.blocking_send(ReloadEvent::Reload {
                    prefix: prefix.clone(),
                    path: path.clone(),
                });
            }
        }
    });
}

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
