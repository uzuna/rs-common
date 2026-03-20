//! 統合テスト（example-plugin）: ロード・HTTP処理・パニック隔離・ホットリロード・状態引き継ぎ。
//!
//! 前提: `cargo build -p safety-plugin-example` でプラグインをビルド済みであること。
//!
//! # プラグインロード方式について  
//! 旧実装では `abi_stable::load_from_file` のプロセス内キャッシュ仕様を前提としていたが、  
//! 現在のホスト実装は `libloading` で `.so` をロードし、`__plugin_create_ref` を直接呼び出す。  
//! これにより、`abi_stable` のプロセスグローバルなキャッシュを迂回しつつ、複数の .so を同一  
//! プロセス内にロードできる設計になっている。  
//!  
//! このテストバイナリでは example-plugin のみを対象としているが、これはテストの見通しと  
//! 状態管理を単純にするためであり、「同一プロセスに複数 .so をロードできない」ためではない。  
//!
//! # テスト並列実行について
//! 環境変数や STATE の競合を防ぐため、`ENV_MUTEX` で全テストを直列化している。

use std::{path::PathBuf, sync::Mutex};

use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use safety_plugin_host::plugin_manager::PluginRouter;

/// 環境変数と STATE を排他操作するための Mutex。
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

fn plugin_path_or_skip() -> Option<PathBuf> {
    let path = example_plugin_path();
    if !path.exists() {
        eprintln!("スキップ: `cargo build -p safety-plugin-example` を実行してください。");
        None
    } else {
        Some(path)
    }
}

fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    for (k, v) in vars {
        std::env::set_var(k, v);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (k, _) in vars {
        std::env::remove_var(k);
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
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

/// 正常系: GET /api/hello に 200 が返ること。
#[test]
fn test_normal_handle() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = [(1usize, 200u16), (3, 200), (5, 200)];

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            for (n, expected_status) in &cases {
                let mut router = PluginRouter::default();
                router
                    .load("/api", &path)
                    .expect("プラグインのロードに失敗");
                let mut last_status = 0u16;
                for _ in 0..*n {
                    last_status = router.handle(make_get("/api/hello")).status;
                }
                assert_eq!(
                    last_status, *expected_status,
                    "{n}回リクエスト後のステータスが不正"
                );
                assert_eq!(router.fallback_count, 0);
            }
        },
    );
}

/// 正常系: POST /api/echo がボディをそのまま返すこと。
#[test]
fn test_echo_handle() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases: &[&[u8]] = &[b"hello", b"", b"\x00\x01\x02"];

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router
                .load("/api", &path)
                .expect("プラグインのロードに失敗");
            for body in cases {
                let resp = router.handle(make_post("/api/echo", body));
                assert_eq!(resp.status, 200);
                assert_eq!(resp.body.as_slice(), *body);
            }
        },
    );
}

/// 正常系: プラグイン内で未知のパスは 404 が返ること。
#[test]
fn test_unmatched_path_in_plugin_returns_404() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = ["/api/unknown", "/api/foo/bar"];

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router
                .load("/api", &path)
                .expect("プラグインのロードに失敗");
            for req_path in &cases {
                let resp = router.handle(make_get(req_path));
                assert_eq!(resp.status, 404, "パス {req_path} は 404 が返るべき");
            }
            assert_eq!(router.fallback_count, 0);
        },
    );
}

/// 正常系: 未登録プレフィックスはホストが 404 を返すこと。
#[test]
fn test_unregistered_route_returns_404() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = ["/other", "/metrics", "/healthz", "/"];

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router
                .load("/api", &path)
                .expect("プラグインのロードに失敗");
            for req_path in &cases {
                let resp = router.handle(make_get(req_path));
                assert_eq!(resp.status, 404, "未登録パス {req_path} は 404 が返るべき");
            }
            assert_eq!(router.fallback_count, 0);
        },
    );
}

/// 異常系: プレフィックス未登録時は 404 が返り fallback_count に加算されないこと。
#[test]
fn test_handle_without_plugin() {
    let cases = [1usize, 3, 10];
    for n in cases {
        let mut router = PluginRouter::default();
        for _ in 0..n {
            let resp = router.handle(make_get("/api/hello"));
            assert_eq!(resp.status, 404);
        }
        assert_eq!(router.fallback_count, 0);
    }
}

// ─── 異常系テスト ────────────────────────────────────────────────────────────

/// 異常系: プラグインがパニックした場合に 500 が返り、ホストが継続すること。
#[test]
fn test_panic_returns_500() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = [(1usize, 500u16, 0u64), (3, 500, 0)];

    for (n, expected_status, expected_fc) in &cases {
        with_env(
            &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "1")],
            || {
                let mut router = PluginRouter::default();
                router.load("/api", &path).unwrap();
                let mut last_status = 0u16;
                for _ in 0..*n {
                    last_status = router.handle(make_get("/api/hello")).status;
                }
                assert_eq!(last_status, *expected_status);
                assert_eq!(router.fallback_count, *expected_fc);
            },
        );
    }
}

// ─── ホットリロードテスト ─────────────────────────────────────────────────────

/// 正常系: ホットリロード後もプラグインが動作を継続すること。
#[test]
fn test_hot_reload_continues() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router.load("/api", &path).unwrap();

            for _ in 0..5 {
                assert_eq!(router.handle(make_get("/api/hello")).status, 200);
            }

            router.reload("/api", &path).expect("リロードが失敗");
            assert!(router.is_loaded("/api"));

            for _ in 0..3 {
                assert_eq!(router.handle(make_get("/api/hello")).status, 200);
            }
            assert_eq!(router.fallback_count, 0);
        },
    );
}

/// 正常系: ホットリロード時に request_count が引き継がれること。
#[test]
fn test_hot_reload_state_transfer() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router.load("/api", &path).unwrap();

            let first_requests = 7u64;
            for _ in 0..first_requests {
                router.handle(make_get("/api/hello"));
            }

            std::env::remove_var("PLUGIN_RESET");
            router.reload("/api", &path).unwrap();

            let second_requests = 3u64;
            for _ in 0..second_requests {
                router.handle(make_get("/api/hello"));
            }

            let state = router.unload("/api").expect("状態が保存されていない");
            assert_eq!(state.len(), 8);
            let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
            assert_eq!(
                request_count,
                first_requests + second_requests,
                "リロードをまたいで request_count が引き継がれていない（got {request_count}）"
            );
        },
    );
}

/// 正常系: 複数回リロードにわたって状態が累積されること。
#[test]
fn test_multi_reload_state_accumulation() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router.load("/api", &path).unwrap();

            std::env::remove_var("PLUGIN_RESET");

            let rounds = [5u64, 3, 2];
            let mut total = 0u64;

            for requests in rounds {
                for _ in 0..requests {
                    router.handle(make_get("/api/hello"));
                }
                total += requests;
                router.reload("/api", &path).unwrap();
            }

            let state = router.unload("/api").expect("状態が保存されていない");
            let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
            assert_eq!(
                request_count, total,
                "累積 request_count が不正（got {request_count}）"
            );
        },
    );
}

/// 異常系: 壊れた .so をリロードしても旧プラグインで継続すること。
#[test]
fn test_reload_failure_falls_back_to_old() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router.load("/api", &path).unwrap();

            for _ in 0..5 {
                router.handle(make_get("/api/hello"));
            }

            let _result = router.reload("/api", std::path::Path::new("/nonexistent/plugin.so"));

            assert!(router.is_loaded("/api"));
            assert_eq!(router.handle(make_get("/api/hello")).status, 200);
            assert_eq!(router.fallback_count, 0);
        },
    );
}

/// 安定性テスト: 100回リロードを繰り返しても動作が安定すること。
#[test]
fn test_100_reload_stability() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(
        &[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")],
        || {
            let mut router = PluginRouter::default();
            router.load("/api", &path).unwrap();

            std::env::remove_var("PLUGIN_RESET");

            for i in 0..100u32 {
                let status = router.handle(make_get("/api/hello")).status;
                assert_eq!(status, 200, "リロード {i} 回目のリクエストが失敗");
                router
                    .reload("/api", &path)
                    .unwrap_or_else(|e| panic!("リロード {i} 回目が失敗: {e}"));
            }

            assert_eq!(router.handle(make_get("/api/hello")).status, 200);
            assert_eq!(router.fallback_count, 0);

            let state = router.unload("/api").expect("状態が保存されていない");
            let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
            assert_eq!(
                request_count, 101,
                "request_count が不正（got {request_count}）"
            );
        },
    );
}
