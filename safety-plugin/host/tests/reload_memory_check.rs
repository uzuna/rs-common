//! プラグインを 10,000 回リロードしたときのメモリ使用量を計測するテスト。
//!
//! 通常のテストスイートには含めず、`--ignored` フラグで明示的に実行する。
//!
//! # 実行方法
//!
//! ```sh
//! cargo build -p safety-plugin-example
//! cargo test -p safety-plugin-host reload_memory -- --ignored --nocapture
//! ```
//!
//! # 計測方法
//!
//! Linux の `/proc/self/status` から `VmRSS`（実メモリ使用量）と
//! `VmSize`（仮想メモリサイズ）を読み取り、リロード前後の差分を出力する。
//!
//! # 期待される動作
//!
//! - dlopen/dlclose でライブラリセグメントが適切に解放されるため、
//!   RSS は O(N) で増加し続けず上限に収束することを確認する。
//! - `__plugin_create_ref` が呼ぶ `leak_into_prefix()` は
//!   グローバルヒープに `size_of::<RobotPlugin>()` ≒ 32 バイトを残留させるため、
//!   10,000 回で最大 320 KB のヒープ増加が予測される。

use std::path::PathBuf;

use abi_stable::std_types::RVec;
use safety_plugin_common::HttpRequest;
use safety_plugin_host::plugin_manager::PluginRouter;

// ─── ヘルパー ─────────────────────────────────────────────────────────────────

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

/// `/proc/self/status` から指定フィールドの値（KB 単位）を読み取る。
///
/// Linux 専用。macOS などでは `None` を返す。
fn read_proc_status_kb(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with(field) {
            return line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        }
    }
    None
}

/// メモリスナップショット。
#[derive(Clone, Copy, Default)]
struct MemSnapshot {
    /// 仮想メモリサイズ（KB）。
    vm_size_kb: u64,
    /// 実メモリ使用量（KB）。
    vm_rss_kb: u64,
}

impl MemSnapshot {
    fn capture() -> Self {
        Self {
            vm_size_kb: read_proc_status_kb("VmSize:").unwrap_or(0),
            vm_rss_kb: read_proc_status_kb("VmRSS:").unwrap_or(0),
        }
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

// ─── テスト ───────────────────────────────────────────────────────────────────

/// 10,000 回リロードしてメモリ使用量の変化を計測する。
///
/// `#[ignore]` を付けているため、`cargo test` では実行されない。
/// `cargo test -- --ignored --nocapture` で実行する。
#[test]
#[ignore]
fn reload_memory_check_10000() {
    let path = example_plugin_path();
    if !path.exists() {
        eprintln!(
            "スキップ: {} が見つかりません。`cargo build -p safety-plugin-example` を実行してください。",
            path.display()
        );
        return;
    }

    const TOTAL_RELOADS: u32 = 10_000;
    const CHECKPOINT_INTERVAL: u32 = 1_000;

    std::env::set_var("PLUGIN_RESET", "1");
    std::env::set_var("PLUGIN_SHOULD_PANIC", "0");

    let mut router = PluginRouter::default();
    router.load("/api", &path).expect("初回ロードに失敗");
    std::env::remove_var("PLUGIN_RESET");

    let baseline = MemSnapshot::capture();
    eprintln!(
        "\n[メモリ計測開始] VmSize: {} KB / VmRSS: {} KB",
        baseline.vm_size_kb, baseline.vm_rss_kb
    );
    eprintln!(
        "{:>8}  {:>12}  {:>12}  {:>14}  {:>14}",
        "リロード数", "VmSize(KB)", "VmRSS(KB)", "ΔVmSize(KB)", "ΔVmRSS(KB)"
    );
    eprintln!("{}", "-".repeat(70));

    let mut checkpoints: Vec<(u32, MemSnapshot)> = Vec::new();

    for i in 1..=TOTAL_RELOADS {
        // リロード（状態を引き継ぎながら繰り返す）
        router.reload("/api", &path).expect("リロードに失敗");

        // リロード後に 1 回リクエストして動作確認
        let resp = router.handle(make_get("/api/hello"));
        assert_eq!(
            resp.status, 200,
            "リロード {i} 回目: GET /api/hello が 200 を返すべき"
        );

        if i % CHECKPOINT_INTERVAL == 0 || i == TOTAL_RELOADS {
            let snap = MemSnapshot::capture();
            let d_size = snap.vm_size_kb as i64 - baseline.vm_size_kb as i64;
            let d_rss = snap.vm_rss_kb as i64 - baseline.vm_rss_kb as i64;
            eprintln!(
                "{:>8}  {:>12}  {:>12}  {:>+14}  {:>+14}",
                i, snap.vm_size_kb, snap.vm_rss_kb, d_size, d_rss
            );
            checkpoints.push((i, snap));
        }
    }

    let final_snap = checkpoints.last().unwrap().1;
    let d_rss_total = final_snap.vm_rss_kb as i64 - baseline.vm_rss_kb as i64;
    let d_size_total = final_snap.vm_size_kb as i64 - baseline.vm_size_kb as i64;

    // 最初と最後のチェックポイントで増加量を比較（線形増加かどうかの粗い確認）
    let rss_at_1000 = (checkpoints[0].1.vm_rss_kb as i64) - (baseline.vm_rss_kb as i64);
    let rss_at_10000 = d_rss_total;

    eprintln!("{}", "-".repeat(70));
    eprintln!(
        "\n[結果] {} 回リロード後の総増加量: ΔVmSize={:+} KB / ΔVmRSS={:+} KB",
        TOTAL_RELOADS, d_size_total, d_rss_total
    );
    eprintln!(
        "[期待] leak_into_prefix のヒープ残留: {} 回 × 32 バイト ≒ {} KB",
        TOTAL_RELOADS,
        (TOTAL_RELOADS as usize * 32) / 1024
    );
    eprintln!(
        "[参考] VmRSS増加: 1,000 回時点={:+} KB / 10,000 回時点={:+} KB",
        rss_at_1000, rss_at_10000
    );

    // 100 MB 以上の RSS 増加があれば異常（dlclose でライブラリが解放されていない可能性）
    assert!(
        d_rss_total < 100 * 1024,
        "{TOTAL_RELOADS} 回リロード後の VmRSS 増加 ({d_rss_total} KB) が 100 MB を超えています。\
        dlclose でライブラリセグメントが解放されていない可能性があります。"
    );
}
