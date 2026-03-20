//! サンプル HTTP プラグイン実装（`define_http_plugin!` マクロ使用）。
//!
//! - `GET {prefix}/hello` → 200 JSON（累積リクエスト数を含む）
//! - `POST {prefix}/echo` → 200 リクエストボディをそのまま返す
//! - それ以外のパス → 404
//!
//! `{prefix}` はホストロード時に指定するマウントポイント（デフォルト: `/api`）。
//!
//! - パニックモード: 環境変数 `PLUGIN_SHOULD_PANIC=1` が設定されている場合、
//!   `handle` 内でパニックを発生させる。`catch_unwind` でラップされているため
//!   ホストプロセスは停止せず、500 レスポンスが返る。
//! - リセットモード: 環境変数 `PLUGIN_RESET=1` が設定されている場合、
//!   `init` の冒頭で request_count を 0 にリセットする（テスト用）。

use safety_plugin_common::{define_http_plugin, HttpRequest, HttpRequestRef, HttpResponse};

/// プラグイン内部状態（ホットリロード時に引き継ぐ）。
#[derive(Default)]
struct PluginState {
    /// 累積リクエスト処理数。状態引き継ぎ検証に使う。
    request_count: u64,
}

define_http_plugin! {
    name: "example-plugin",
    state: PluginState,
    handler: handle_inner,
    handler_ref: handle_inner_ref,
    state_save: save_state,
    state_load: load_state,
}

/// 状態をバイト列へ変換する（request_count を little-endian 8バイトで保存）。
fn save_state(state: &PluginState) -> Result<Vec<u8>, String> {
    Ok(state.request_count.to_le_bytes().to_vec())
}

/// バイト列から状態を復元する。
///
/// ただし `PLUGIN_RESET=1` が設定されている場合は 0 にリセットする（テスト用）。
fn load_state(bytes: &[u8]) -> Result<PluginState, String> {
    if std::env::var("PLUGIN_RESET").as_deref() == Ok("1") {
        return Ok(PluginState::default());
    }
    if bytes.len() == 8 {
        let arr: [u8; 8] = bytes[..8]
            .try_into()
            .map_err(|e| format!("スライス変換失敗: {e}"))?;
        Ok(PluginState {
            request_count: u64::from_le_bytes(arr),
        })
    } else {
        Err(format!(
            "無効なバイト列長: {} バイト（期待値: 8）",
            bytes.len()
        ))
    }
}

/// HTTP リクエストの実処理。マクロが生成した `__handle` から呼ばれる（`HttpRequest` 所有型版）。
fn handle_inner(req: &HttpRequest, state: &mut PluginState) -> HttpResponse {
    handle_core(
        req.method.as_str(),
        req.path.as_str(),
        req.body.as_slice(),
        state,
    )
}

/// ゼロコピー版ハンドラ。`__plugin_handle_ref` から呼ばれる（`HttpRequestRef` 借用型版）。
fn handle_inner_ref(req: &HttpRequestRef<'_>, state: &mut PluginState) -> HttpResponse {
    handle_core(
        req.method.as_str(),
        req.path.as_str(),
        req.body.as_slice(),
        state,
    )
}

/// 共通実装。`handle_inner` / `handle_inner_ref` 双方から呼ばれる。
fn handle_core(method: &str, path: &str, body: &[u8], state: &mut PluginState) -> HttpResponse {
    // パニックモード: 環境変数で制御（ホスト統合テストが PLUGIN_SHOULD_PANIC=1 を設定して使用）
    if std::env::var("PLUGIN_SHOULD_PANIC").as_deref() == Ok("1") {
        panic!("意図的なパニック（Phase 4 検証用）");
    }

    state.request_count += 1;

    match (method, path) {
        ("GET", p) if p.ends_with("/hello") => HttpResponse {
            status: 200,
            content_type: "application/json".into(),
            body: format!(r#"{{"message":"hello","count":{}}}"#, state.request_count)
                .into_bytes()
                .into(),
        },
        ("POST", p) if p.ends_with("/echo") => HttpResponse {
            status: 200,
            content_type: "application/octet-stream".into(),
            body: body.to_vec().into(),
        },
        _ => HttpResponse {
            status: 404,
            content_type: "text/plain".into(),
            body: format!("not found: {method} {path}").into_bytes().into(),
        },
    }
}
