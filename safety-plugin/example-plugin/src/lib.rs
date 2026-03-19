//! サンプル HTTP プラグイン実装。
//!
//! - `GET /api/hello` → 200 JSON（累積リクエスト数を含む）
//! - `POST /api/echo` → 200 リクエストボディをそのまま返す
//! - それ以外のパス → 404
//!
//! - パニックモード: 環境変数 `PLUGIN_SHOULD_PANIC=1` が設定されている場合、
//!   `handle` 内でパニックを発生させる。`catch_unwind` でラップされているため
//!   ホストプロセスは停止せず、500 レスポンスが返る。
//! - リセットモード: 環境変数 `PLUGIN_RESET=1` が設定されている場合、
//!   `init` の冒頭で request_count を 0 にリセットする（テスト用）。

use std::sync::Mutex;

use abi_stable::{
    export_root_module,
    prefix_type::PrefixTypeTrait,
    std_types::{ROption, RSlice, RVec},
};
use safety_plugin_common::{
    HttpRequest, HttpResponse, PluginContext, PluginKind, RobotPlugin, RobotPlugin_Ref,
    RouteDescriptor,
};

/// プラグイン内部状態（ホットリロード時に引き継ぐ）。
#[derive(Default)]
struct PluginState {
    /// 累積リクエスト処理数。状態引き継ぎ検証に使う。
    request_count: u64,
}

static STATE: Mutex<PluginState> = Mutex::new(PluginState { request_count: 0 });

/// abi_stable がこの関数をエントリポイントとして認識する。
#[export_root_module]
fn get_library() -> RobotPlugin_Ref {
    RobotPlugin {
        kind,
        init,
        handle,
        shutdown,
    }
    .leak_into_prefix()
}

/// このプラグインの種類を返す。
extern "C" fn kind() -> PluginKind {
    PluginKind::Http
}

/// 初期化。前回の状態があれば復元し、担当ルートのリストを返す。
extern "C" fn init(
    _ctx: &PluginContext,
    prev_state: ROption<RSlice<'_, u8>>,
) -> RVec<RouteDescriptor> {
    let mut state = STATE.lock().unwrap();

    // PLUGIN_RESET=1 が設定されている場合は状態を 0 にリセットする（テスト用）
    if std::env::var("PLUGIN_RESET").as_deref() == Ok("1") {
        state.request_count = 0;
    } else if let abi_stable::std_types::RSome(bytes) = prev_state {
        // 前回の状態（バイト列）を復元する
        if bytes.len() == 8 {
            let arr: [u8; 8] = bytes[..8].try_into().unwrap_or([0u8; 8]);
            state.request_count = u64::from_le_bytes(arr);
        }
    }

    eprintln!(
        "[example-plugin] init: request_count={} から再開",
        state.request_count
    );

    // 担当するパスプレフィックスを宣言する
    RVec::from(vec![RouteDescriptor {
        path_prefix: "/api".into(),
    }])
}

/// HTTP リクエスト処理。パニックは catch_unwind でブロックし 500 を返す。
extern "C" fn handle(req: &HttpRequest) -> HttpResponse {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_inner(req)));
    match result {
        Ok(resp) => resp,
        Err(_) => {
            eprintln!("[example-plugin] panic を捕捉しました。500 を返します");
            HttpResponse {
                status: 500,
                content_type: "text/plain".into(),
                body: b"internal plugin error".to_vec().into(),
            }
        }
    }
}

/// handle の実処理。パニックを発生させる可能性がある。
fn handle_inner(req: &HttpRequest) -> HttpResponse {
    // パニックモード: 環境変数で制御
    if std::env::var("PLUGIN_SHOULD_PANIC").as_deref() == Ok("1") {
        panic!("意図的なパニック（Phase 4 検証用）");
    }

    let mut state = STATE.lock().unwrap();
    state.request_count += 1;

    let path = req.path.as_str();
    let method = req.method.as_str();

    match (method, path) {
        ("GET", p) if p.starts_with("/api/hello") => HttpResponse {
            status: 200,
            content_type: "application/json".into(),
            body: format!(
                r#"{{"message":"hello","count":{}}}"#,
                state.request_count
            )
            .into_bytes()
            .into(),
        },
        ("POST", p) if p.starts_with("/api/echo") => HttpResponse {
            status: 200,
            content_type: "application/octet-stream".into(),
            body: req.body.clone(),
        },
        _ => HttpResponse {
            status: 404,
            content_type: "text/plain".into(),
            body: b"not found".to_vec().into(),
        },
    }
}

/// 終了処理。内部状態をバイト列として返す。
extern "C" fn shutdown() -> RVec<u8> {
    let state = STATE.lock().unwrap();
    eprintln!(
        "[example-plugin] shutdown: request_count={} を保存します",
        state.request_count
    );
    // request_count を little-endian 8バイトとして保存
    RVec::from(state.request_count.to_le_bytes().to_vec())
}
