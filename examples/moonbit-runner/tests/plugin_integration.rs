//! PluginManager 統合テスト。
//!
//! 前提: `make -C examples/moonbit-runner build-plugin` で Wasm をビルド済みであること。
//!
//! # テスト並列実行について
//!
//! 各テストは独立した `PluginManager` を使うため特に直列化は不要。
//! ただし `decode_call_count` は MoonBit stub.mbt の `save_state` 実装
//! （call_count を先頭4バイト LE）に依存する。

use std::path::{Path, PathBuf};

use moonbit_runner::{bindings::SensorData, plugin_manager::PluginManager};

fn component_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins")
        .join("control.component.wasm")
}

fn skip_if_no_plugin() -> Option<PathBuf> {
    let path = component_path();
    if !path.exists() {
        eprintln!(
            "スキップ: Wasm が見つかりません（{}）。先に `make -C examples/moonbit-runner build-plugin` を実行してください",
            path.display()
        );
        None
    } else {
        Some(path)
    }
}

fn new_manager() -> PluginManager {
    PluginManager::new().expect("PluginManager 初期化失敗")
}

/// `save_state` が返すバイト列から call_count を復元する。
///
/// MoonBit stub.mbt の実装に合わせ、先頭4バイトを little-endian i32 として読む。
fn decode_call_count(bytes: &[u8]) -> i32 {
    if bytes.len() < 4 {
        return 0;
    }
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn sensor() -> SensorData {
    SensorData {
        load: 10.0,
        position: 0.0,
        extra: None,
    }
}

// ─── 正常系 ────────────────────────────────────────────────────────────────

#[test]
fn test_load_plugin_正常系() {
    let Some(path) = skip_if_no_plugin() else {
        return;
    };
    let mut mgr = new_manager();
    mgr.load(&path).expect("ロード失敗");

    // update と get_status が動作する
    let outputs = mgr.update(&[sensor()]).expect("update 失敗");
    assert_eq!(outputs.len(), 1, "センサー1個→出力1個");
    assert!(outputs[0].torque >= 0.0, "torque >= 0");

    let status = mgr.get_status().expect("get_status 失敗");
    assert!(status.running, "running=true");
    assert_eq!(status.error_code, 0, "error_code=0");
}

// ─── 状態引き継ぎの直接確認 ───────────────────────────────────────────────

struct StateTransferCase {
    name: &'static str,
    update_before_reload: usize,
    update_after_reload: usize,
}

#[test]
fn test_reload_state_transfer_正常系() {
    let Some(path) = skip_if_no_plugin() else {
        return;
    };

    let cases = [
        StateTransferCase {
            name: "reload前1回・後1回",
            update_before_reload: 1,
            update_after_reload: 1,
        },
        StateTransferCase {
            name: "reload前5回・後3回",
            update_before_reload: 5,
            update_after_reload: 3,
        },
    ];

    for case in &cases {
        let mut mgr = new_manager();
        mgr.load(&path).expect("ロード失敗");

        // reload 前に update を呼ぶ
        for _ in 0..case.update_before_reload {
            mgr.update(&[sensor()])
                .expect(&format!("ケース '{}': reload前 update 失敗", case.name));
        }

        // reload 前の call_count を確認
        let before_bytes = mgr
            .snapshot_current_state()
            .expect(&format!("ケース '{}': snapshot 失敗", case.name))
            .expect("reload前はロード済みのはず");
        let count_before = decode_call_count(&before_bytes);
        assert_eq!(
            count_before, case.update_before_reload as i32,
            "ケース '{}': reload 前の call_count",
            case.name
        );

        // reload
        mgr.reload(&path)
            .expect(&format!("ケース '{}': reload 失敗", case.name));

        // reload 後に update をさらに呼ぶ
        for _ in 0..case.update_after_reload {
            mgr.update(&[sensor()])
                .expect(&format!("ケース '{}': reload後 update 失敗", case.name));
        }

        // reload 後の call_count が累積されていることを確認
        let after_bytes = mgr
            .snapshot_current_state()
            .expect(&format!("ケース '{}': reload後 snapshot 失敗", case.name))
            .expect("reload後もロード済みのはず");
        let count_after = decode_call_count(&after_bytes);
        let expected = (case.update_before_reload + case.update_after_reload) as i32;
        assert_eq!(
            count_after,
            expected,
            "ケース '{}': reload をまたいで call_count が累積 (before={}, after_delta={}, total={})",
            case.name,
            case.update_before_reload,
            case.update_after_reload,
            expected
        );
    }
}

// ─── フォールバック ────────────────────────────────────────────────────────

#[test]
fn test_reload_failure_falls_back_to_old_正常系() {
    let Some(path) = skip_if_no_plugin() else {
        return;
    };
    let mut mgr = new_manager();
    mgr.load(&path).expect("初回ロード失敗");

    // 正常動作を確認
    mgr.update(&[sensor()]).expect("初回 update 失敗");
    let before = mgr.snapshot_current_state().unwrap().unwrap();
    let count_before = decode_call_count(&before);

    // 壊れた .wasm でリロードを試みる
    let broken = tempfile_broken_wasm();
    let result = mgr.reload(&broken);
    assert!(result.is_err(), "壊れた .wasm はエラーを返す");

    // 旧プラグインにフォールバックして継続動作する
    let result = mgr.update(&[sensor()]);
    assert!(
        result.is_ok(),
        "フォールバック後も update が動作する: {result:?}"
    );

    // フォールバック後は call_count が reload 前の値を引き継いでいる
    let after = mgr.snapshot_current_state().unwrap().unwrap();
    let count_after = decode_call_count(&after);
    assert_eq!(
        count_after,
        count_before + 1,
        "フォールバック後 update 1回で call_count が 1 増える"
    );

    // フォールバック時 fallback_count は加算されない（旧プラグインが動いているため）
    assert_eq!(
        mgr.fallback_count, 0,
        "旧プラグインで継続中は fallback_count=0"
    );

    // 後片付け
    let _ = std::fs::remove_file(&broken);
}

// ─── 安定性 ────────────────────────────────────────────────────────────────

#[test]
fn test_reload_100回_安定性() {
    let Some(path) = skip_if_no_plugin() else {
        return;
    };
    let mut mgr = new_manager();
    mgr.load(&path).expect("初回ロード失敗");

    for i in 0..100 {
        mgr.update(&[sensor()])
            .expect(&format!("reload前 update #{i} 失敗"));
        mgr.reload(&path).expect(&format!("reload #{i} 失敗"));
    }

    // 100 回リロード後も正常動作する
    mgr.update(&[sensor()])
        .expect("100回リロード後 update 失敗");

    // call_count = 101（update 100回 + reload後1回）
    let bytes = mgr.snapshot_current_state().unwrap().unwrap();
    let count = decode_call_count(&bytes);
    assert_eq!(count, 101, "100回リロード後の call_count = 101");
    assert_eq!(mgr.fallback_count, 0, "fallback_count は 0 のまま");
}

// ─── ヘルパー ──────────────────────────────────────────────────────────────

/// 壊れた（無効な）.wasm ファイルを一時ファイルとして生成して返す。
fn tempfile_broken_wasm() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "moonbit_runner_broken_{}.wasm",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::write(&path, b"this is not valid wasm").expect("一時ファイル生成失敗");
    path
}
