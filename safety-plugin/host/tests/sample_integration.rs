//! 統合テスト（sample-plugin）: add/mul/status エンドポイントと状態引き継ぎの検証。
//!
//! 前提: `cargo build -p safety-plugin-sample` でプラグインをビルド済みであること。
//!
//! このテストは host クレートの `PluginRouter` を通じて、`libloading` により動的ロード  
//! された sample-plugin を実際に呼び出す統合テストである。  
//! plugin_integration.rs の example-plugin 向けテストと同様に、ホスト側は  
//! `__plugin_create_ref` を直接呼び出す実装となっており、複数プラグインの共存は  
//! ホスト側のロード方式によって保証される。  

use std::{path::PathBuf, sync::Mutex};

use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use safety_plugin_host::plugin_manager::PluginRouter;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn sample_plugin_path() -> PathBuf {
    if let Ok(p) = std::env::var("SAFETY_PLUGIN_SAMPLE_PATH") {
        return PathBuf::from(p);
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/libsafety_plugin_sample.so")
}

fn plugin_path_or_skip() -> Option<PathBuf> {
    let path = sample_plugin_path();
    if !path.exists() {
        eprintln!("スキップ: `cargo build -p safety-plugin-sample` を実行してください。");
        None
    } else {
        Some(path)
    }
}

fn make_get(path: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: path.into(),
        query: "".into(),
        body: RVec::new(),
    }
}

fn make_post(path: &str, body: &[u8]) -> HttpRequest {
    HttpRequest {
        method: "POST".into(),
        path: path.into(),
        query: "".into(),
        body: RVec::from(body.to_vec()),
    }
}

// ─── 正常系テスト ────────────────────────────────────────────────────────────

/// 正常系: POST /sample/add が正しく加算結果を返すこと。
#[test]
fn test_sample_add() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    // (a, b, expected_result)
    let cases = [(3i64, 4, 7), (0, 0, 0), (-1, 1, 0), (100, -200, -100)];

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    for (a, b, expected) in &cases {
        let body = format!(r#"{{"a":{a},"b":{b}}}"#);
        let resp = router.handle(make_post("/sample/add", body.as_bytes()));
        assert_eq!(resp.status, 200, "add は 200 を返すべき (a={a}, b={b})");
        let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(
            json["result"].as_i64().unwrap(),
            *expected,
            "add の結果が不正 (a={a}, b={b})"
        );
    }
}

/// 正常系: POST /sample/mul が正しく乗算結果を返すこと。
#[test]
fn test_sample_mul() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = [(3i64, 4, 12), (0, 100, 0), (-2, -3, 6), (7, -1, -7)];

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    for (a, b, expected) in &cases {
        let body = format!(r#"{{"a":{a},"b":{b}}}"#);
        let resp = router.handle(make_post("/sample/mul", body.as_bytes()));
        assert_eq!(resp.status, 200, "mul は 200 を返すべき (a={a}, b={b})");
        let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(
            json["result"].as_i64().unwrap(),
            *expected,
            "mul の結果が不正 (a={a}, b={b})"
        );
    }
}

/// 正常系: 不正なボディで 400 が返ること。
#[test]
fn test_sample_bad_request() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = [b"not json" as &[u8], b"", b"{\"a\":1}"];

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    for body in cases {
        let resp = router.handle(make_post("/sample/add", body));
        assert_eq!(resp.status, 400, "不正なボディは 400 が返るべき");
    }
}

/// 正常系: GET /sample/status が op_count を返すこと。
#[test]
fn test_sample_status_op_count() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    // add x2 + mul x1 → op_count=3
    let ops: &[(&str, &str)] = &[
        ("/sample/add", r#"{"a":1,"b":2}"#),
        ("/sample/add", r#"{"a":3,"b":4}"#),
        ("/sample/mul", r#"{"a":5,"b":6}"#),
    ];
    for (path, body) in ops {
        router.handle(make_post(path, body.as_bytes()));
    }

    let resp = router.handle(make_get("/sample/status"));
    assert_eq!(resp.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["op_count"].as_u64().unwrap(), 3, "op_count が不正");
}

/// 正常系: 未知パスは 404 が返ること。
#[test]
fn test_sample_unknown_path_returns_404() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = ["/sample/unknown", "/sample/div"];

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    for req_path in &cases {
        let resp = router.handle(make_get(req_path));
        assert_eq!(resp.status, 404, "パス {req_path} は 404 が返るべき");
    }
}

// ─── ホットリロードテスト ─────────────────────────────────────────────────────

/// 正常系: ホットリロード時に op_count が引き継がれること。
#[test]
fn test_sample_hot_reload_state_transfer() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let mut router = PluginRouter::default();
    router
        .load("/sample", &path)
        .expect("sample-plugin のロードに失敗");

    // リロード前に 5回演算
    for _ in 0..5 {
        router.handle(make_post("/sample/add", b"{\"a\":1,\"b\":2}"));
    }

    router.reload("/sample", &path).expect("リロードに失敗");

    // リロード後に 3回演算
    for _ in 0..3 {
        router.handle(make_post("/sample/mul", b"{\"a\":2,\"b\":3}"));
    }

    let resp = router.handle(make_get("/sample/status"));
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        json["op_count"].as_u64().unwrap(),
        8,
        "リロードをまたいで op_count が引き継がれていない"
    );
}
