//! 統合テスト: プラグインのロード・パニック隔離・ホットリロード・状態引き継ぎの検証。
//!
//! 前提: `cargo build -p safety-plugin-example` でプラグインをビルド済みであること。
//!
//! # abi_stable のキャッシュ仕様
//! `abi_stable` の `load_from_file` はプロセス内で最初の成功ロードをキャッシュする。
//! そのため、一度でも正常ロードが成功すると、以降は同じモジュールが返される。
//! テスト間での `STATE`（step_count）汚染を防ぐため、環境変数 `PLUGIN_RESET=1` を使う。
//!
//! # テスト並列実行について
//! 環境変数や `STATE` の競合を防ぐため、`ENV_MUTEX` で全テストを直列化している。

use std::{path::PathBuf, sync::Mutex};

use safety_plugin_host::plugin_manager::{PluginManager, StepResult};

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

// ─── Phase 2 テスト ───────────────────────────────────────────────────────────

/// 正常系: プラグインをロードし `step()` が `Ok` を返すことを確認する。
#[test]
fn test_normal_step() {
    let Some(path) = plugin_path_or_skip() else { return };

    // (step回数, 期待するStepResult, 期待するfallback_count)
    let cases = [
        (1usize, StepResult::Ok, 0u64),
        (3, StepResult::Ok, 0),
        (5, StepResult::Ok, 0),
    ];

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        for (n, ref expected, expected_fc) in &cases {
            let mut mgr = PluginManager::default();
            mgr.load(&path).expect("プラグインのロードに失敗");
            for _ in 0..*n {
                let result = mgr.step();
                assert_eq!(&result, expected, "{n}回ステップ後の結果が不正");
            }
            assert_eq!(mgr.fallback_count, *expected_fc, "正常時はフォールバックしないこと");
        }
    });
}

/// 異常系: プラグインがパニックした場合にFallbackが返り、ホストが継続することを確認する。
#[test]
fn test_panic_triggers_fallback() {
    let Some(path) = plugin_path_or_skip() else { return };

    // (パニックフラグ, step呼び出し回数, 期待するStepResult, 期待するfallback_count)
    let cases = [
        ("1", 1usize, StepResult::Fallback, 1u64),
        ("1", 3, StepResult::Fallback, 3),
    ];

    for (flag, n, ref expected, expected_fc) in &cases {
        with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", flag)], || {
            let mut mgr = PluginManager::default();
            mgr.load(&path).unwrap();
            let mut last = StepResult::Ok;
            for _ in 0..*n {
                last = mgr.step();
            }
            assert_eq!(&last, expected, "PLUGIN_SHOULD_PANIC={flag} のとき結果が不正");
            assert_eq!(
                mgr.fallback_count, *expected_fc,
                "PLUGIN_SHOULD_PANIC={flag} のときフォールバック回数が不正"
            );
        });
    }
}

/// 異常系: 未ロード状態では常にFallbackが返ることを確認する。
/// この テストは env var を使わないため ENV_MUTEX 外で実行できる。
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

// ─── Phase 3 テスト ───────────────────────────────────────────────────────────

/// 正常系: ホットリロード後もプラグインが動作を継続することを確認する。
#[test]
fn test_hot_reload_continues() {
    let Some(path) = plugin_path_or_skip() else { return };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap();

        for _ in 0..5 {
            assert_eq!(mgr.step(), StepResult::Ok);
        }
        assert!(mgr.is_loaded());

        // ホットリロード
        let result = mgr.reload(&path);
        assert!(result.is_ok(), "リロードが失敗した: {result:?}");
        assert!(mgr.is_loaded(), "リロード後もロード済みであるべき");

        for _ in 0..3 {
            assert_eq!(mgr.step(), StepResult::Ok, "リロード後もOkが返るべき");
        }
        assert_eq!(mgr.fallback_count, 0, "正常リロードではフォールバックしない");
    });
}

/// 正常系: ホットリロード時に内部状態（step_count）が引き継がれることを確認する。
///
/// PLUGIN_RESET=1 で初回 init を0から開始し、リロード後に累積値を確認する。
#[test]
fn test_hot_reload_state_transfer() {
    let Some(path) = plugin_path_or_skip() else { return };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // init: PLUGIN_RESET=1 → step_count=0

        let first_steps = 7u64;
        for _ in 0..first_steps {
            mgr.step(); // step_count: 0→7
        }

        // reload: shutdown(step_count=7) → saved_state=[7] → init(PLUGIN_RESET=0だが引き継ぎあり)
        // PLUGIN_RESET を解除してリロード（状態が引き継がれることを確認）
        std::env::remove_var("PLUGIN_RESET");
        mgr.reload(&path).unwrap(); // init: prev_state=[7] → step_count=7

        let second_steps = 3u64;
        for _ in 0..second_steps {
            mgr.step(); // step_count: 7→10
        }

        let state = mgr.unload().expect("状態が保存されていない");
        assert_eq!(state.len(), 8, "step_count は 8バイト(u64)のはず");
        let step_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(
            step_count,
            first_steps + second_steps,
            "リロードをまたいで step_count が引き継がれていない（got {step_count}）"
        );
    });
}

/// 正常系: 複数回リロードにわたって状態が累積されることを確認する。
#[test]
fn test_multi_reload_state_accumulation() {
    let Some(path) = plugin_path_or_skip() else { return };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // step_count=0

        // PLUGIN_RESET を解除して以降のリロードは状態を引き継ぐ
        std::env::remove_var("PLUGIN_RESET");

        let rounds = [5u64, 3, 2]; // 各ラウンドのステップ数
        let mut total = 0u64;

        for steps in rounds {
            for _ in 0..steps {
                mgr.step();
            }
            total += steps;
            mgr.reload(&path).unwrap();
        }

        let state = mgr.unload().expect("状態が保存されていない");
        let step_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(step_count, total, "複数リロードにわたる累積 step_count が不正（got {step_count}）");
    });
}

/// 異常系: 壊れた .so をリロードしても旧プラグインで継続することを確認する。
#[test]
fn test_reload_failure_falls_back_to_old() {
    let Some(path) = plugin_path_or_skip() else { return };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap();

        for _ in 0..5 {
            mgr.step();
        }

        // 存在しないパスでリロード → 失敗するが旧バージョンで継続
        // （注: abi_stable のキャッシュにより load_from_file はキャッシュ済みモジュールを返す可能性があるが、
        //   LibraryError 系エラーは abi_stable 内部のバージョン検証で発生する場合もある）
        let result = mgr.reload(std::path::Path::new("/nonexistent/plugin_that_does_not_exist.so"));

        // キャッシュにより Ok になる場合もあるが、どちらでも動作継続すること
        assert!(mgr.is_loaded(), "リロード後（成否問わず）ロード済みであるべき");
        assert_eq!(mgr.step(), StepResult::Ok, "プラグインで step() が動くはず");
        assert_eq!(mgr.fallback_count, 0, "フォールバックしないこと");
        let _ = result; // Ok/Err どちらでも許容
    });
}

/// 安定性テスト: 100回リロードを繰り返しても動作が安定することを確認する。
#[test]
fn test_100_reload_stability() {
    let Some(path) = plugin_path_or_skip() else { return };

    with_env(&[("PLUGIN_RESET", "1"), ("PLUGIN_SHOULD_PANIC", "0")], || {
        let mut mgr = PluginManager::default();
        mgr.load(&path).unwrap(); // step_count=0

        // リロード後は状態を引き継ぐ（PLUGIN_RESET解除）
        std::env::remove_var("PLUGIN_RESET");

        for i in 0..100u32 {
            let step_result = mgr.step();
            assert_eq!(step_result, StepResult::Ok, "リロード {i} 回目のステップが失敗");
            mgr.reload(&path)
                .unwrap_or_else(|e| panic!("リロード {i} 回目が失敗: {e}"));
        }

        assert_eq!(mgr.step(), StepResult::Ok, "100回リロード後のステップが失敗");
        assert_eq!(mgr.fallback_count, 0, "安定性テストでフォールバックが発生");

        // 100ステップ（各ラウンド1回）+ 最後の1ステップ = 101
        let state = mgr.unload().expect("状態が保存されていない");
        let step_count = u64::from_le_bytes(state[..8].try_into().unwrap());
        assert_eq!(step_count, 101, "100回リロード後の step_count が不正（got {step_count}）");
    });
}
