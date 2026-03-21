//! ホットリロード時のメモリリーク確認テスト。
//!
//! # 実行方法
//!
//! ```bash
//! cargo test -p moonbit-runner reload_memory -- --ignored --nocapture
//! ```
//!
//! # 前提
//!
//! `make -C examples/moonbit-runner build-plugin` で Wasm をビルド済みであること。
//!
//! # 設計意図
//!
//! wasmtime の `Component::new` → `Store::new` → drop のサイクルが
//! O(1) メモリで安定することを確認する。
//!
//! `Engine` 内部の JIT コードキャッシュが `Component` ごとに
//! 保持される可能性があるため、許容成長量を緩めに設定している。

use std::path::{Path, PathBuf};

use moonbit_runner::bindings::SensorData;
use moonbit_runner::plugin_manager::PluginManager;

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

fn sensor() -> SensorData {
    SensorData {
        load: 1.0,
        position: 0.5,
        extra: None,
    }
}

/// `/proc/self/status` から VmRSS（kB）を読み取る。Linux 専用。
fn read_vmrss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

/// N 回リロード後のメモリ増加が許容量以内であることを確認する。
///
/// - `reload_count`: リロード回数
/// - `max_growth_mb`: 許容するメモリ増加の上限（MB）
fn assert_reload_memory_stable(reload_count: usize, max_growth_mb: u64) {
    let Some(path) = skip_if_no_plugin() else {
        return;
    };

    let before_kb = read_vmrss_kb().unwrap_or(0);
    println!("VmRSS before: {} kB", before_kb);

    let mut mgr = PluginManager::new().expect("PluginManager 初期化失敗");
    mgr.load(&path).expect("初回ロード失敗");

    for i in 0..reload_count {
        mgr.update(&[sensor()]).expect(&format!("update #{i} 失敗"));
        mgr.reload(&path).expect(&format!("reload #{i} 失敗"));
    }

    // 最後の update で状態を確認
    mgr.update(&[sensor()]).expect("最終 update 失敗");

    let after_kb = read_vmrss_kb().unwrap_or(0);
    println!("VmRSS after {} reloads: {} kB", reload_count, after_kb);

    let growth_kb = after_kb.saturating_sub(before_kb);
    let growth_mb = growth_kb / 1024;
    println!(
        "VmRSS 増加: {} kB ({} MB) / 許容: {} MB",
        growth_kb, growth_mb, max_growth_mb
    );

    assert!(
        growth_mb <= max_growth_mb,
        "メモリ増加 {growth_mb} MB が許容量 {max_growth_mb} MB を超えました \
         (before={before_kb} kB, after={after_kb} kB)"
    );
}

// ─── テスト ────────────────────────────────────────────────────────────────

#[test]
#[ignore = "メモリリークチェック（`cargo test -- --ignored --nocapture` で明示実行）"]
fn reload_memory_check_100times() {
    // 100 回リロード後のメモリ増加が 100 MB 以内であることを確認する。
    // wasmtime の JIT キャッシュ蓄積を考慮して緩めに設定している。
    assert_reload_memory_stable(100, 100);
}

#[test]
#[ignore = "メモリリークチェック（`cargo test -- --ignored --nocapture` で明示実行）"]
fn reload_memory_check_1000times() {
    // 1000 回リロード後のメモリ増加が 500 MB 以内であることを確認する。
    // 線形増加（リーク）がないことを確認するためのストレステスト。
    assert_reload_memory_stable(1000, 500);
}
