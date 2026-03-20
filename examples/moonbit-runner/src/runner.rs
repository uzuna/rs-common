//! MoonBit Wasm プラグインへ HTTP 経由で処理を委譲するランナー

use anyhow::{bail, ensure, Context};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wasmtime::{component::Component, Engine};

use crate::bindings::{MotorOutput, PluginInst, PluginStatus, SensorData};
use crate::context::ExecStore;
use crate::engine;

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

struct AppState {
    plugin: Mutex<PluginInst>,
    plugin_prefix: String,
    wasm_path: String,
    wasi: WasiSupport,
}

#[derive(Debug, Clone, Serialize)]
struct ServerStatusResponse {
    service: &'static str,
    plugin_prefix: String,
    wasm: String,
    wasi: &'static str,
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
    let engine = engine::create_engine_from_env()?;
    let component = load_component(&engine, &config.wasm)?;
    let plugin = instantiate_plugin(&engine, &component, config.wasi)?;

    let state = Arc::new(AppState {
        plugin: Mutex::new(plugin),
        plugin_prefix: plugin_prefix.clone(),
        wasm_path: config.wasm.display().to_string(),
        wasi: config.wasi,
    });

    let plugin_router = Router::new()
        .route("/status", get(handle_plugin_status))
        .route("/update", post(handle_plugin_update));

    let app = Router::new()
        .route("/status", get(handle_server_status))
        .nest(&plugin_prefix, plugin_router)
        .with_state(state);

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
        axum::serve(listener, app)
            .await
            .context("HTTP サーバーがエラー終了しました")
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

fn load_component(engine: &Engine, path: &Path) -> anyhow::Result<Component> {
    let buffer = std::fs::read(path)
        .with_context(|| format!("Wasmファイルの読み込み失敗: {}", path.display()))?;
    Component::new(engine, &buffer)
        .map_err(|e| anyhow::anyhow!("Componentの生成失敗: {}: {}", path.display(), e))
}

fn instantiate_plugin(
    engine: &Engine,
    component: &Component,
    wasi: WasiSupport,
) -> anyhow::Result<PluginInst> {
    match wasi {
        WasiSupport::None => {
            let store = ExecStore::new(engine);
            PluginInst::new_with_binary(store, component)
                .context("WASIなしプラグインの初期化に失敗しました")
        }
        WasiSupport::Preview2 => {
            bail!("WASI Preview2 は moonbit-runner では未対応です")
        }
    }
}

async fn handle_server_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::Json<ServerStatusResponse> {
    axum::Json(ServerStatusResponse {
        service: "moonbit-runner",
        plugin_prefix: state.plugin_prefix.clone(),
        wasm: state.wasm_path.clone(),
        wasi: state.wasi.as_str(),
    })
}

async fn handle_plugin_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> HttpResult<axum::Json<PluginStatusResponse>> {
    let mut plugin = state.plugin.lock().map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "プラグイン状態ロックの取得に失敗しました".to_string(),
        )
    })?;

    let status = plugin.get_status().map_err(|err| {
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

    let mut plugin = state.plugin.lock().map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "プラグイン状態ロックの取得に失敗しました".to_string(),
        )
    })?;

    let outputs = plugin.update(&input).map_err(|err| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            format!("Wasm プラグインの update 呼び出しに失敗しました: {err}"),
        )
    })?;

    let output = outputs.into_iter().map(MotorOutputResponse::from).collect();
    Ok(axum::Json(UpdateResponse { output }))
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
