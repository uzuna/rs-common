//! 統合テスト: プラグインのロード・HTTP処理・パニック隔離・ホットリロード・状態引き継ぎの検証。
//!
//! 前提: `cargo build -p safety-plugin-example` でプラグインをビルド済みであること。
//!
//! # abi_stable のキャッシュ仕様
//! `abi_stable` の `load_from_file` はプロセス内で最初の成功ロードをキャッシュする。
//! そのため、一度でも正常ロードが成功すると、以降は同じモジュールが返される。
//! テスト間での `STATE`（request_count）汚染を防ぐため、環境変数 `PLUGIN_RESET=1` を使う。
//!
//! # テスト並列実行について
//! 環境変数や `STATE` の競合を防ぐため、`ENV_MUTEX` で全テストを直列化している。

use std::{path::PathBuf, sync::Mutex};

use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use safety_plugin_host::plugin_manager::PluginManager;

/// 環境変数と STATE を排他操作するための Mutex。
/// パニックで毒化されても `unwrap_or_else` で回復する。
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// ビルド済みの example-plugin .so のパスを返す。
/// `SAFETY_PLUGIN_EXAMPLE_PATH` 環境変数で上書き可能。
fn example_plugin_path() -> PathBuf {
    if let Ok(p) = std::env::var("SAFETY_PLUGIN_EXAMPLE_PATH") {
        return PathBuf::from(p);
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent()
        .unwrap() // safety-plugin/
        .parent()
        .unwrap() // ワークスペースルート
        .join("target/debug/libsafety_plugin_example.so")
}

/// プラグインが未ビルドの場合はテストをスキップし `None` を返す。
fn plugin_path_or_skip() -> Option<PathBuf> {
    let path = example_plugin_path();
    if !path.exists() {
        eprintln!(
            "スキップ: `cargo build -p safety-plugin-example` を実行してください。\nパス: {}",
            path.display()
        );
        None
    } else {
        Some(path)
    }
}

/// 環境変数を一時的に設定し、クロージャ終了後に除去する。
/// パニックで毒化されたミューテックスも `unwrap_or_else` で回復する。
fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    for (k, v) in vars {
        std::env::set_var(k, v);
    }
    // パニックが起きてもenv varをクリーンアップできるよう catch_unwind でラップ
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (k, _) in vars {
        std::env::remove_var(k);
    }
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// テスト用の GET /api/hello リクエストを作成する。
fn make_get_hello() -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: "/api/hello".into(),
        query: "".into(),
        body: RVec::new(),
    }
}

/// テスト用の POST /api/echo リクエストを作成する。
fn make_post_echo(body: &[u8]) -> HttpRequest {
    HttpRequest {
        method: "POST".into(),
        path: "/api/echo".into(),
        query: "".into(),
        body: RVec::from(body.to_vec()),
    }
}

/// テスト用の指定パスへの GET リクエストを作成する。
fn make_get(path: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: path.into(),
        query: "".into(),
        body: RVec::new(),
    }
}

// ─── 正常系テスト ────────────────────────────────────────────────────────────

/// 正常系: プラグインが GET /api/hello に 200 を返すことを確認する。
#[test]
fn test_normal_handle() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    // (リクエスト数, 期待ステータス)
    let cases = [(1usize, 200u16), (3, 200), (5, 200)];

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        for (n, expected_status) in &cases {
            let mut mgr = PluginManager::default();
            mgr.load(&path).expect("プラグインのロードに失敗");
            let mut last_status = 0u16;
            for _ in 0..*n {
                last_status = mgr.handle(&make_get_hello()).status;
            }
            assert_eq!(
                last_status, *expected_status,
                "{n}回リクエスト後のステータスが不正"
            );
            assert_eq!(mgr.fallback_count, 0, "正常時はフォールバックしないこと");
        }
    });
}

/// 正常系: POST /api/echo がリクエストボディをそのまま返すことを確認する。
#[test]
fn test_echo_handle() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases: &[&[u8]] = &[b"hello", b"", b"\x00\x01\x02"];

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).expect("プラグインのロードに失敗");
        for body in cases {
            let resp = mgr.handle(&make_post_echo(body));
            assert_eq!(resp.status, 200, "echo は 200 を返すべき");
            assert_eq!(
                resp.body.as_slice(),
                *body,
                "echo はボディをそのまま返すべき"
            );
        }
    });
}

/// 正常系: プラグインが担当しないパスは 404 が返ることを確認する（プラグイン処理）。
#[test]
fn test_unmatched_path_in_plugin_returns_404() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    let cases = ["/api/unknown", "/api/foo/bar"];

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).expect("プラグインのロードに失敗");
        for req_path in &cases {
            let resp = mgr.handle(&make_get(req_path));
            assert_eq!(
                resp.status, 404,
                "パス {req_path} は 404 を返すべき（プラグイン内）"
            );
        }
        assert_eq!(mgr.fallback_count, 0, "404 はフォールバックではない");
    });
}

/// 正常系: プラグインが担当しないルートプレフィックスは 404 が返ることを確認する（ホスト処理）。
#[test]
fn test_unregistered_route_returns_404() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    // /api 以外のプレフィックスはホストが 404 を返す
    let cases = ["/other", "/metrics", "/healthz", "/"];

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).expect("プラグインのロードに失敗");
        for req_path in &cases {
            let resp = mgr.handle(&make_get(req_path));
            assert_eq!(
                resp.status, 404,
                "未登録パス {req_path} はホストが 404 を返すべき"
            );
        }
        assert_eq!(mgr.fallback_count, 0, "ルート未マッチはフォールバックではない");
    });
}

/// 異常系: プラグイン未ロード時は 503 が返り、fallback_count が加算される。
/// この テストは env var を使わないため ENV_MUTEX 外で実行できる。
#[test]
fn test_handle_without_plugin() {
    let cases = [1usize, 3, 10];
    for n in cases {
        let mut mgr = PluginManager::default();
        for _ in 0..n {
            let resp = mgr.handle(&make_get_hello());
            assert_eq!(resp.status, 503, "未ロード時は 503 が返るべき");
        }
        assert_eq!(mgr.fallback_count, n as u64, "fallback_count が一致しない");
    }
}

// ─── 異常系テスト ────────────────────────────────────────────────────────────

/// 異常系: プラグインがパニックした場合に 500 が返り、ホストが継続することを確認する。
#[test]
fn test_panic_returns_500() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    // (リクエスト数, 期待ステータス, 期待fallback_count)
    let cases = [(1usize, 500u16, 0u64), (3, 500, 0)];

    for (n, expected_status, expected_fc) in &cases {
        with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "1")], || {
            let mut mgr = PluginManager::default();
            mgr.load(&path).unwrap();
            let mut last_status = 0u16;
            for _ in 0..*n {
                last_status = mgr.handle(&make_get_hello()).status;
            }
            assert_eq!(
                last_status, *expected_status,
                "パニック時は 500 が返るべき（{n}回目）"
            );
            assert_eq!(
                mgr.fallback_count, *expected_fc,
                "パニック時は fallback_count が増えないこと（500 はプラグイン側処理）"
            );
        });
    }
}

// ─── ホットリロードテスト ─────────────────────────────────────────────────────

/// 正常系: ホットリロード後もプラグインが動作を継続することを確認する。
#[test]
fn test_hot_reload_continues() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap();

        for _ in 0..5 {
            assert_eq!(mgr.handle(&make_get_hello()).status, 200);
        }
        assert!(mgr.is_loaded());

        // ホットリロード
        let result = mgr.reload(&path);
        assert!(result.is_ok(), "リロードが失敗した: {result:?}");
        assert!(mgr.is_loaded(), "リロード後もロード済みであるべき");

        for _ in 0..3 {
            assert_eq!(
                mgr.handle(&make_get_hello()).status,
                200,
                "リロード後も 200 が返るべき"
            );
        }
        assert_eq!(mgr.fallback_count, 0, "正常リロードではフォールバックしない");
    });
}

/// 正常系: ホットリロード時に内部状態（request_count）が引き継がれることを確認する。
#[test]
fn test_hot_reload_state_transfer() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // init: PLUGIN_RESET=1 → request_count=0

        let first_requests = 7u64;
        for _ in 0..first_requests {
            mgr.handle(&make_get_hello()); // request_count: 0→7
        }

        // PLUGIN_RESET を解除してリロード（状態が引き継がれることを確認）
        std::env::remove_var("PLUGIN_RESET");
        mgr.reload(&path).unwrap(); // init: prev_state=[7] → request_count=7

        let second_requests = 3u64;
        for _ in 0..second_requests {
            mgr.handle(&make_get_hello()); // request_count: 7→10
        }

        let state = mgr.unload().expect("状態が保存されていない");
        assert_eq!(state.len(), 8, "request_count は 8バイト(u64)のはず");
        let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(
            request_count,
            first_requests + second_requests,
            "リロードをまたいで request_count が引き継がれていない（got {request_count}）"
        );
    });
}

/// 正常系: 複数回リロードにわたって状態が累積されることを確認する。
#[test]
fn test_multi_reload_state_accumulation() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // request_count=0

        // PLUGIN_RESET を解除して以降のリロードは状態を引き継ぐ
        std::env::remove_var("PLUGIN_RESET");

        let rounds = [5u64, 3, 2]; // 各ラウンドのリクエスト数
        let mut total = 0u64;

        for requests in rounds {
            for _ in 0..requests {
                mgr.handle(&make_get_hello());
            }
            total += requests;
            mgr.reload(&path).unwrap();
        }

        let state = mgr.unload().expect("状態が保存されていない");
        let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(
            request_count, total,
            "複数リロードにわたる累積 request_count が不正（got {request_count}）"
        );
    });
}

/// 異常系: 壊れた .so をリロードしても旧プラグインで継続することを確認する。
#[test]
fn test_reload_failure_falls_back_to_old() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap();

        for _ in 0..5 {
            mgr.handle(&make_get_hello());
        }

        // 存在しないパスでリロード → 失敗するが旧バージョンで継続
        let result =
            mgr.reload(std::path::Path::new("/nonexistent/plugin_that_does_not_exist.so"));

        assert!(mgr.is_loaded(), "リロード後（成否問わず）ロード済みであるべき");
        assert_eq!(
            mgr.handle(&make_get_hello()).status,
            200,
            "旧プラグインで 200 が返るべき"
        );
        assert_eq!(mgr.fallback_count, 0, "フォールバックしないこと");
        let _ = result; // Ok/Err どちらでも許容
    });
}

/// 安定性テスト: 100回リロードを繰り返しても動作が安定することを確認する。
#[test]
fn test_100_reload_stability() {
    let Some(path) = plugin_path_or_skip() else {
        return;
    };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // request_count=0

        // リロード後は状態を引き継ぐ（PLUGIN_RESET解除）
        std::env::remove_var("PLUGIN_RESET");

        for i in 0..100u32 {
            let status = mgr.handle(&make_get_hello()).status;
            assert_eq!(status, 200, "リロード {i} 回目のリクエストが失敗");
            mgr.reload(&path)
                .unwrap_or_else(|e| panic!("リロード {i} 回目が失敗: {e}"));
        }

        assert_eq!(
            mgr.handle(&make_get_hello()).status,
            200,
            "100回リロード後のリクエストが失敗"
        );
        assert_eq!(mgr.fallback_count, 0, "安定性テストでフォールバックが発生");

        // 100リクエスト（各ラウンド1回）+ 最後の1リクエスト = 101
        let state = mgr.unload().expect("状態が保存されていない");
        let request_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(
            request_count, 101,
            "100回リロード後の request_count が不正（got {request_count}）"
        );
    });
}
