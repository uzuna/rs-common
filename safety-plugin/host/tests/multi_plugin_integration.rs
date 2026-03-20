//! 複数プラグイン統合テスト: example-plugin と sample-plugin を同一プロセスで動かす。
//!
//! 前提: 以下を事前にビルドしておくこと。
//!   cargo build -p safety-plugin-example -p safety-plugin-sample
//!
//! # abi_stable キャッシュ迂回
//! ホストは `abi_stable::load_from_file` ではなく `libloading` 経由で `__plugin_create_ref`
//! を直接呼び出すため、同一プロセスで異なる `.so` を複数ロードできる。

use std::{path::PathBuf, sync::Mutex};

use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use safety_plugin_host::plugin_manager::PluginRouter;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn example_plugin_path() -> PathBuf {
    if let Ok(p) = std::env::var("SAFETY_PLUGIN_EXAMPLE_PATH") {
        return PathBuf::from(p);
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/libsafety_plugin_example.so")
}

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

fn plugins_or_skip() -> Option<(PathBuf, PathBuf)> {
    let ep = example_plugin_path();
    let sp = sample_plugin_path();
    if !ep.exists() || !sp.exists() {
        eprintln!("スキップ: `cargo build -p safety-plugin-example -p safety-plugin-sample` を実行してください。");
        return None;
    }
    Some((ep, sp))
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

// ─── 複数プラグイン同時動作テスト ─────────────────────────────────────────────

/// 正常系: example-plugin と sample-plugin を同一プロセスにロードし、
/// それぞれのエンドポイントが独立して正しく動作すること。
#[test]
fn test_multi_plugin_basic_routing() {
    let Some((example_path, sample_path)) = plugins_or_skip() else {
        return;
    };

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("PLUGIN_RESET", "1");
    std::env::set_var("PLUGIN_SHOULD_PANIC", "0");

    let mut router = PluginRouter::default();
    router
        .load("/api", &example_path)
        .expect("example-plugin のロードに失敗");
    router
        .load("/sample", &sample_path)
        .expect("sample-plugin のロードに失敗");

    // example-plugin: GET /api/hello → 200
    let resp = router.handle(make_get("/api/hello"));
    assert_eq!(resp.status, 200, "GET /api/hello は 200 が返るべき");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["message"], "hello");

    // sample-plugin: POST /sample/add → 200, result=7
    let body = b"{\"a\":3,\"b\":4}";
    let resp = router.handle(make_post("/sample/add", body));
    assert_eq!(resp.status, 200, "POST /sample/add は 200 が返るべき");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["result"].as_i64().unwrap(), 7);

    // sample-plugin: POST /sample/mul → 200, result=12
    let body = b"{\"a\":3,\"b\":4}";
    let resp = router.handle(make_post("/sample/mul", body));
    assert_eq!(resp.status, 200, "POST /sample/mul は 200 が返るべき");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(json["result"].as_i64().unwrap(), 12);

    assert_eq!(router.fallback_count, 0);

    std::env::remove_var("PLUGIN_RESET");
    std::env::remove_var("PLUGIN_SHOULD_PANIC");
}

/// 正常系: 複数プラグインの状態が互いに独立していること。
///
/// example-plugin の request_count と sample-plugin の op_count が
/// それぞれ独立してカウントされることを確認する。
#[test]
fn test_multi_plugin_independent_state() {
    let Some((example_path, sample_path)) = plugins_or_skip() else {
        return;
    };

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("PLUGIN_RESET", "1");
    std::env::set_var("PLUGIN_SHOULD_PANIC", "0");

    let mut router = PluginRouter::default();
    router
        .load("/api", &example_path)
        .expect("example-plugin のロードに失敗");
    router
        .load("/sample", &sample_path)
        .expect("sample-plugin のロードに失敗");

    // example-plugin に 3回リクエスト
    for _ in 0..3 {
        router.handle(make_get("/api/hello"));
    }

    // sample-plugin に 5回演算
    for _ in 0..5 {
        router.handle(make_post("/sample/add", b"{\"a\":1,\"b\":2}"));
    }

    // example-plugin の request_count = 3
    let resp = router.handle(make_get("/api/hello"));
    assert_eq!(resp.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        json["count"].as_u64().unwrap(),
        4,
        "example-plugin の request_count が不正（4回目のリクエスト）"
    );

    // sample-plugin の op_count = 5
    let resp = router.handle(make_get("/sample/status"));
    assert_eq!(resp.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        json["op_count"].as_u64().unwrap(),
        5,
        "sample-plugin の op_count が不正"
    );

    assert_eq!(router.fallback_count, 0);

    std::env::remove_var("PLUGIN_RESET");
    std::env::remove_var("PLUGIN_SHOULD_PANIC");
}

/// 正常系: 一方のプラグインをリロードしても、もう一方は影響を受けないこと。
#[test]
fn test_multi_plugin_reload_isolation() {
    let Some((example_path, sample_path)) = plugins_or_skip() else {
        return;
    };

    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("PLUGIN_RESET", "1");
    std::env::set_var("PLUGIN_SHOULD_PANIC", "0");

    let mut router = PluginRouter::default();
    router
        .load("/api", &example_path)
        .expect("example-plugin のロードに失敗");
    router
        .load("/sample", &sample_path)
        .expect("sample-plugin のロードに失敗");

    // 各プラグインに初期リクエスト
    for _ in 0..3 {
        router.handle(make_get("/api/hello"));
        router.handle(make_post("/sample/add", b"{\"a\":1,\"b\":1}"));
    }

    // sample-plugin のみリロード（状態は引き継ぐ）
    std::env::remove_var("PLUGIN_RESET");
    router
        .reload("/sample", &sample_path)
        .expect("sample-plugin のリロードに失敗");

    // example-plugin は影響を受けず動作継続
    let resp = router.handle(make_get("/api/hello"));
    assert_eq!(
        resp.status, 200,
        "sample-plugin リロード後も example-plugin は動作するべき"
    );

    // sample-plugin はリロード後も op_count を引き継いでいる
    let resp = router.handle(make_get("/sample/status"));
    assert_eq!(resp.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(
        json["op_count"].as_u64().unwrap(),
        3,
        "リロード後 sample-plugin の op_count が引き継がれていない"
    );

    assert_eq!(router.fallback_count, 0);

    std::env::remove_var("PLUGIN_RESET");
    std::env::remove_var("PLUGIN_SHOULD_PANIC");
}
