//! 計算プラグイン（`define_http_plugin!` マクロ使用サンプル）。
//!
//! `{prefix}` はホストロード時に指定するマウントポイント（デフォルト: `/sample`）。
//!
//! ## エンドポイント
//!
//! | メソッド | パス              | リクエストボディ           | レスポンス              |
//! | :------- | :---------------- | :------------------------- | :---------------------- |
//! | POST     | `{prefix}/add`    | `{"a":<i64>,"b":<i64>}`    | `{"result":<i64>}`      |
//! | POST     | `{prefix}/mul`    | `{"a":<i64>,"b":<i64>}`    | `{"result":<i64>}`      |
//! | GET      | `{prefix}/status` | —                          | `{"op_count":<u64>}`    |
//!
//! 内部状態として `op_count`（累積演算回数）を保持し、ホットリロード時に引き継ぐ。

use safety_plugin_common::{define_http_plugin, HttpRequest, HttpRequestRef, HttpResponse};
use serde::{Deserialize, Serialize};

/// プラグイン内部状態（ホットリロード時に引き継ぐ）。
#[derive(Default, Serialize, Deserialize)]
struct CalcState {
    /// 累積演算回数。状態引き継ぎ検証に使う。
    op_count: u64,
}

define_http_plugin! {
    name: "sample-plugin",
    state: CalcState,
    handler: handle_inner,
    handler_ref: handle_inner_ref,
    state_save: save_state,
    state_load: load_state,
}

/// 状態をバイト列へ変換する（JSON シリアライズ）。
fn save_state(state: &CalcState) -> Vec<u8> {
    serde_json::to_vec(state).unwrap_or_default()
}

/// バイト列から状態を復元する（JSON デシリアライズ）。
fn load_state(bytes: &[u8]) -> Option<CalcState> {
    serde_json::from_slice(bytes).ok()
}

/// 演算リクエストのボディ。
#[derive(Deserialize)]
struct CalcInput {
    a: i64,
    b: i64,
}

/// HTTP リクエストの実処理（`HttpRequest` 所有型版）。
fn handle_inner(req: &HttpRequest, state: &mut CalcState) -> HttpResponse {
    handle_core(
        req.method.as_str(),
        req.path.as_str(),
        req.body.as_slice(),
        state,
    )
}

/// ゼロコピー版ハンドラ（`HttpRequestRef` 借用型版）。
fn handle_inner_ref(req: &HttpRequestRef<'_>, state: &mut CalcState) -> HttpResponse {
    handle_core(
        req.method.as_str(),
        req.path.as_str(),
        req.body.as_slice(),
        state,
    )
}

/// 共通実装。`handle_inner` / `handle_inner_ref` 双方から呼ばれる。
fn handle_core(method: &str, path: &str, body: &[u8], state: &mut CalcState) -> HttpResponse {
    // パスの末尾セグメントでルーティング（プレフィックスは問わない）
    match (method, path.rsplit('/').next().unwrap_or("")) {
        ("POST", "add") => {
            let input = match parse_input(body) {
                Ok(v) => v,
                Err(e) => return bad_request(&e),
            };
            state.op_count += 1;
            json_response(200, &serde_json::json!({ "result": input.a + input.b }))
        }
        ("POST", "mul") => {
            let input = match parse_input(body) {
                Ok(v) => v,
                Err(e) => return bad_request(&e),
            };
            state.op_count += 1;
            json_response(200, &serde_json::json!({ "result": input.a * input.b }))
        }
        ("GET", "status") => json_response(200, &serde_json::json!({ "op_count": state.op_count })),
        _ => HttpResponse {
            status: 404,
            content_type: "text/plain".into(),
            body: b"not found".to_vec().into(),
        },
    }
}

fn parse_input(body: &[u8]) -> Result<CalcInput, String> {
    serde_json::from_slice(body).map_err(|e| format!("JSONパースエラー: {e}"))
}

fn json_response(status: u16, value: &serde_json::Value) -> HttpResponse {
    HttpResponse {
        status,
        content_type: "application/json".into(),
        body: serde_json::to_vec(value).unwrap_or_default().into(),
    }
}

fn bad_request(msg: &str) -> HttpResponse {
    HttpResponse {
        status: 400,
        content_type: "text/plain".into(),
        body: msg.as_bytes().to_vec().into(),
    }
}
