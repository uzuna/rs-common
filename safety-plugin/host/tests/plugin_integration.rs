//! Phase 2 統合テスト: プラグインのロードとパニック隔離の検証。
//!
//! 前提: `cargo build -p safety-plugin-example` でプラグインをビルド済みであること。

use std::{path::PathBuf, sync::Mutex};

use safety_plugin_host::plugin_manager::{PluginManager, StepResult};

/// 環境変数 `PLUGIN_SHOULD_PANIC` を排他操作するための Mutex。
/// テストを並列実行した場合でも環境変数が競合しないようにする。
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// ビルド済みの example-plugin .so のパスを返す。
/// `safety_plugin_EXAMPLE_PATH` 環境変数で上書き可能。
fn example_plugin_path() -> PathBuf {
    if let Ok(p) = std::env::var("safety_plugin_EXAMPLE_PATH") {
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

/// 環境変数を一時的に設定し、クロージャ終了後に除去する。
fn with_panic_env<F: FnOnce()>(flag: &str, f: F) {
    let _guard = ENV_MUTEX.lock().unwrap();
    std::env::set_var("PLUGIN_SHOULD_PANIC", flag);
    f();
    std::env::remove_var("PLUGIN_SHOULD_PANIC");
}

/// 正常系: プラグインをロードし `step()` が `Ok` を返すことを確認する。
#[test]
fn test_normal_step() {
    let path = example_plugin_path();
    if !path.exists() {
        eprintln!(
            "スキップ: プラグインが未ビルドです。`cargo build -p safety-plugin-example` を実行してください。\nパス: {}",
            path.display()
        );
        return;
    }

    // (step回数, 期待するStepResult, 期待するfallback_count)
    let cases = [
        (1usize, StepResult::Ok, 0u64),
        (3, StepResult::Ok, 0),
        (5, StepResult::Ok, 0),
    ];

    with_panic_env("0", || {
        for (n, ref expected, expected_fc) in &cases {
            let mut mgr = PluginManager::default();
            mgr.load(&path).expect("プラグインのロードに失敗");
            for _ in 0..*n {
                let result = mgr.step();
                assert_eq!(&result, expected, "{n}回ステップ後の結果が不正");
            }
            assert_eq!(
                mgr.fallback_count, *expected_fc,
                "正常時はフォールバックしないこと"
            );
        }
    });
}

/// 異常系: プラグインがパニックした場合にFallbackが返り、ホストが継続することを確認する。
#[test]
fn test_panic_triggers_fallback() {
    let path = example_plugin_path();
    if !path.exists() {
        eprintln!("スキップ: プラグインが未ビルドです");
        return;
    }

    // (パニックフラグ, step呼び出し回数, 期待するStepResult, 期待するfallback_count)
    let cases = [
        ("1", 1usize, StepResult::Fallback, 1u64),
        ("1", 3, StepResult::Fallback, 3),
    ];

    for (flag, n, ref expected, expected_fc) in &cases {
        with_panic_env(flag, || {
            let mut mgr = PluginManager::default();
            mgr.load(&path).unwrap();
            let mut last = StepResult::Ok;
            for _ in 0..*n {
                last = mgr.step();
            }
            assert_eq!(
                &last, expected,
                "PLUGIN_SHOULD_PANIC={flag} のとき結果が不正"
            );
            assert_eq!(
                mgr.fallback_count, *expected_fc,
                "PLUGIN_SHOULD_PANIC={flag} のときフォールバック回数が不正"
            );
        });
    }
}

/// 異常系: 未ロード状態では常にFallbackが返ることを確認する。
#[test]
fn test_step_without_plugin() {
    let cases = [1usize, 3, 10];
    for n in cases {
        let mut mgr = PluginManager::default();
        for _ in 0..n {
            let result = mgr.step();
            assert_eq!(result, StepResult::Fallback);
        }
        assert_eq!(mgr.fallback_count, n as u64);
    }
}

/// 異常系: 存在しないパスをロードしたときにエラーが返ることを確認する。
#[test]
fn test_load_nonexistent_path() {
    let mut mgr = PluginManager::default();
    let result = mgr.load(std::path::Path::new("/nonexistent/path/plugin.so"));
    assert!(result.is_err(), "存在しないパスはエラーであるべき");
    // ロード失敗後はフォールバックモードで動作すること
    assert_eq!(mgr.step(), StepResult::Fallback);
}
