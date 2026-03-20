//! リクエスト処理レイテンシのベンチマーク。
//!
//! ホスト直書き実装（native）と FFI 経由のプラグイン実装（plugin）を比較する。
//!
//! # 計測範囲
//!
//! - `hello` グループ: JSON 文字列を返すだけのシンプルなハンドラ
//! - `add` グループ: JSON ボディをパースして加算し結果を返すハンドラ
//!
//! # 実行方法
//!
//! ```sh
//! cargo build -p safety-plugin-example -p safety-plugin-sample
//! cargo bench -p safety-plugin-host
//! ```
//!
//! プラグイン .so が見つからない場合、plugin ベンチマークはスキップされる。

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use abi_stable::std_types::RVec;
use criterion::{criterion_group, criterion_main, Criterion};
use safety_plugin_common::{HttpRequest, HttpResponse};
use safety_plugin_host::plugin_manager::{rstring_from_pool, PluginRouter};
// PluginRouter::handle_ref は &str スライスのみで呼べるためインポート不要
use std::hint::black_box;

// ─── プラグインパス ───────────────────────────────────────────────────────────

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

// ─── ネイティブ実装（ホスト直書きの場合の比較対象） ──────────────────────────

/// ネイティブ hello ハンドラ。
///
/// example-plugin と同じ出力形式で JSON を生成する。
/// プラグイン FFI や Mutex を介さず、直接 Rust 関数として呼び出す。
fn native_hello() -> HttpResponse {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    HttpResponse {
        status: 200,
        content_type: "application/json".into(),
        body: format!(r#"{{"message":"hello","count":{count}}}"#)
            .into_bytes()
            .into(),
    }
}

/// ネイティブ add ハンドラ。
///
/// sample-plugin と同じ処理（JSON パース → 加算 → JSON 生成）をホストで直書きする。
/// `serde` を直接依存に持たないため `serde_json::Value` でパースする。
fn native_add(body: &[u8]) -> HttpResponse {
    let result: Result<(i64, i64), _> = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let a = v.get("a")?.as_i64()?;
            let b = v.get("b")?.as_i64()?;
            Some(Ok((a, b)))
        })
        .unwrap_or(Err(()));

    match result {
        Ok((a, b)) => HttpResponse {
            status: 200,
            content_type: "application/json".into(),
            body: format!(r#"{{"result":{}}}"#, a + b).into_bytes().into(),
        },
        Err(_) => HttpResponse {
            status: 400,
            content_type: "text/plain".into(),
            body: b"bad request".to_vec().into(),
        },
    }
}

// ─── ヘルパー ─────────────────────────────────────────────────────────────────

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

/// プールから RString を借用して GET リクエストを構築する。
///
/// [`PluginRouter::handle`] がリクエスト処理後に文字列をプールへ返却するため、
/// 繰り返し呼び出すと定常状態では `method` / `path` / `query` のアロケーションがなくなる。
fn make_get_pooled(path: &str) -> HttpRequest {
    HttpRequest {
        method: rstring_from_pool("GET"),
        path: rstring_from_pool(path),
        query: rstring_from_pool(""),
        body: RVec::new(),
    }
}

/// プールから RString を借用して POST リクエストを構築する。
fn make_post_pooled(path: &str, body: &[u8]) -> HttpRequest {
    HttpRequest {
        method: rstring_from_pool("POST"),
        path: rstring_from_pool(path),
        query: rstring_from_pool(""),
        body: RVec::from(body.to_vec()),
    }
}

// ─── ベンチマーク関数 ─────────────────────────────────────────────────────────

/// hello レイテンシ比較: ネイティブ vs example-plugin FFI。
fn bench_hello(c: &mut Criterion) {
    let mut group = c.benchmark_group("hello");

    // native: ホスト直書きの Rust 関数
    group.bench_function("native", |b| {
        b.iter(|| black_box(native_hello()));
    });

    // plugin: PluginRouter 経由 FFI 呼び出し
    let path = example_plugin_path();
    if path.exists() {
        std::env::set_var("PLUGIN_RESET", "1");
        std::env::set_var("PLUGIN_SHOULD_PANIC", "0");
        let mut router = PluginRouter::default();
        router
            .load("/api", &path)
            .expect("example-plugin ロード失敗");
        std::env::remove_var("PLUGIN_RESET");

        group.bench_function("plugin", |b| {
            b.iter(|| black_box(router.handle(make_get("/api/hello"))));
        });

        // plugin_pooled: プールから RString を再利用して FFI 呼び出し
        group.bench_function("plugin_pooled", |b| {
            b.iter(|| black_box(router.handle(make_get_pooled("/api/hello"))));
        });

        // plugin_rstr: RStr<'_> ゼロコピー FFI 呼び出し（ホスト側アロケーション完全ゼロ）
        group.bench_function("plugin_rstr", |b| {
            b.iter(|| black_box(router.handle_ref("GET", "/api/hello", "", b"")));
        });
    } else {
        eprintln!(
            "[bench_hello] スキップ: {} が見つかりません",
            path.display()
        );
    }

    group.finish();
}

/// add レイテンシ比較: ネイティブ vs sample-plugin FFI。
fn bench_add(c: &mut Criterion) {
    let mut group = c.benchmark_group("add");

    let body = b"{\"a\":3,\"b\":4}";

    // native: ホスト直書きの Rust 関数（JSON パース込み）
    group.bench_function("native", |b| {
        b.iter(|| black_box(native_add(black_box(body.as_slice()))));
    });

    // plugin: PluginRouter 経由 FFI 呼び出し（JSON パース含む）
    let path = sample_plugin_path();
    if path.exists() {
        std::env::set_var("PLUGIN_RESET", "1");
        let mut router = PluginRouter::default();
        router
            .load("/sample", &path)
            .expect("sample-plugin ロード失敗");
        std::env::remove_var("PLUGIN_RESET");

        group.bench_function("plugin", |b| {
            b.iter(|| black_box(router.handle(make_post("/sample/add", black_box(body)))));
        });

        // plugin_pooled: プールから RString を再利用して FFI 呼び出し
        group.bench_function("plugin_pooled", |b| {
            b.iter(|| black_box(router.handle(make_post_pooled("/sample/add", black_box(body)))));
        });

        // plugin_rstr: RStr<'_> ゼロコピー FFI 呼び出し（ホスト側アロケーション完全ゼロ）
        group.bench_function("plugin_rstr", |b| {
            b.iter(|| black_box(router.handle_ref("POST", "/sample/add", "", black_box(body))));
        });
    } else {
        eprintln!("[bench_add] スキップ: {} が見つかりません", path.display());
    }

    group.finish();
}

criterion_group!(benches, bench_hello, bench_add);
criterion_main!(benches);
