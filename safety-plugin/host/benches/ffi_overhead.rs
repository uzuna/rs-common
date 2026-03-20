//! FFI・Mutex・アロケーション等の要素ベンチマーク。
//!
//! `native` と `plugin` のレイテンシ差（約 270–340 ns）の各要因を個別に計測し、
//! どのコストが支配的かを明らかにする。
//!
//! # 計測グループ
//!
//! - `call`   : 直接呼び出し / Rust 関数ポインタ / `extern "C"` 関数ポインタ の比較
//! - `mutex`  : Mutex ロック・アンロックのみ / 状態更新込み
//! - `alloc`  : `RString` 新規アロケーション / プール再利用 / `RStr`（ゼロコピー）の比較
//! - `env`    : `std::env::var` 1 回 / 2 回の呼び出しコスト
//! - `routing`: HashMap 最長プレフィックス検索（1 エントリ / 2 エントリ）
//!
//! # 実行方法
//!
//! ```sh
//! cargo bench -p safety-plugin-host --bench ffi_overhead
//! ```

use std::{
    collections::HashMap,
    hint::black_box,
    sync::{Mutex, OnceLock},
};

use abi_stable::std_types::{RString, RVec};
use criterion::{criterion_group, criterion_main, Criterion};
use safety_plugin_common::HttpResponse;
use safety_plugin_host::plugin_manager::{rstring_from_pool, rstring_to_pool};

// ─── call グループ ────────────────────────────────────────────────────────────

/// 計測対象の最小 Rust 関数（インライン展開抑制）。
#[inline(never)]
fn rust_fn(x: u64) -> u64 {
    x.wrapping_add(1)
}

/// 計測対象の最小 extern "C" 関数（インライン展開抑制）。
///
/// `extern "C"` は通常の Rust 関数と ABI が異なり、
/// 呼び出し規約の変換（レジスタ退避等）が発生する可能性がある。
#[inline(never)]
extern "C" fn c_fn(x: u64) -> u64 {
    x.wrapping_add(1)
}

/// 3 種類の呼び出し形式のオーバーヘッドを比較する。
///
/// - `direct`       : `rust_fn(x)` を直接呼び出し
/// - `rust_fn_ptr`  : `fn(u64) -> u64` 型のポインタ経由で呼び出し
/// - `extern_c_ptr` : `extern "C" fn(u64) -> u64` 型のポインタ経由で呼び出し
fn bench_call(c: &mut Criterion) {
    let mut group = c.benchmark_group("call");

    // 直接呼び出し（ベースライン）
    group.bench_function("direct", |b| {
        let mut x = 0u64;
        b.iter(|| {
            x = black_box(rust_fn(black_box(x)));
        });
    });

    // Rust 関数ポインタ経由
    group.bench_function("rust_fn_ptr", |b| {
        let f: fn(u64) -> u64 = rust_fn;
        let mut x = 0u64;
        b.iter(|| {
            x = black_box(f(black_box(x)));
        });
    });

    // extern "C" 関数ポインタ経由（プラグイン FFI と同じ呼び出し形式）
    group.bench_function("extern_c_ptr", |b| {
        let f: extern "C" fn(u64) -> u64 = c_fn;
        let mut x = 0u64;
        b.iter(|| {
            x = black_box(f(black_box(x)));
        });
    });

    group.finish();
}

// ─── mutex グループ ───────────────────────────────────────────────────────────

/// Mutex のロック・アンロックコストを計測する。
///
/// - `lock_only`     : lock → 値読み出し → drop（状態更新なし）
/// - `lock_increment`: lock → カウンタインクリメント → drop（プラグイン内部状態更新の模擬）
fn bench_mutex(c: &mut Criterion) {
    let mut group = c.benchmark_group("mutex");

    let m = Mutex::new(0u64);

    // ロックと値の読み出しのみ
    group.bench_function("lock_only", |b| {
        b.iter(|| {
            let guard = m.lock().unwrap();
            black_box(*guard);
        });
    });

    // ロック + カウンタ更新（プラグインの状態アクセスに相当）
    group.bench_function("lock_increment", |b| {
        b.iter(|| {
            let mut guard = m.lock().unwrap();
            *guard = black_box(guard.wrapping_add(1));
        });
    });

    group.finish();
}

// ─── alloc グループ ───────────────────────────────────────────────────────────

/// 文字列表現の構築コストを比較する。
///
/// - `rstring_new`   : `RString::from(s)` で毎回新規アロケート（`plugin` 版の動作）
/// - `rstring_pool`  : プールから再利用（`plugin_pooled` 版の動作）
/// - `rstr_borrow`   : `RStr::from(s)` でゼロコピー（`plugin_rstr` 版の動作）
///
/// 各ケースで method / path / query に相当する 3 文字列を構築する。
fn bench_alloc(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc");

    // RString 新規アロケーション × 3（plugin 版のホスト側コスト）
    group.bench_function("rstring_new", |b| {
        b.iter(|| {
            let method = black_box(RString::from(black_box("GET")));
            let path = black_box(RString::from(black_box("/api/hello")));
            let query = black_box(RString::from(black_box("")));
            drop(method);
            drop(path);
            drop(query);
        });
    });

    // プール再利用（plugin_pooled 版: 定常状態でのホスト側コスト）
    // warmup 中にプールが満たされるため、実測値はほぼ clear+push_str のコスト
    group.bench_function("rstring_pool", |b| {
        b.iter(|| {
            let method = black_box(rstring_from_pool(black_box("GET")));
            let path = black_box(rstring_from_pool(black_box("/api/hello")));
            let query = black_box(rstring_from_pool(black_box("")));
            drop(method);
            drop(path);
            drop(query);
        });
    });

    // プール再利用（ウォーム状態: 前イテレーションが返却した文字列を再利用する定常状態）
    // rstring_to_pool でループ内に返却するため warmup 後は常にプールから取り出せる
    group.bench_function("rstring_pool_warm", |b| {
        b.iter(|| {
            let method = rstring_from_pool(black_box("GET"));
            let path = rstring_from_pool(black_box("/api/hello"));
            let query = rstring_from_pool(black_box(""));
            // 処理が終わった文字列をプールへ返却（PluginRouter::handle の動作を再現）
            rstring_to_pool(method);
            rstring_to_pool(path);
            rstring_to_pool(query);
        });
    });

    // RStr ゼロコピー（plugin_rstr 版: アロケーションなし、ポインタ+長さのみ）
    group.bench_function("rstr_borrow", |b| {
        b.iter(|| {
            let method = black_box(abi_stable::std_types::RStr::from(black_box("GET")));
            let path = black_box(abi_stable::std_types::RStr::from(black_box("/api/hello")));
            let query = black_box(abi_stable::std_types::RStr::from(black_box("")));
            black_box((method, path, query));
        });
    });

    group.finish();
}

// ─── env グループ ─────────────────────────────────────────────────────────────

/// `std::env::var` の呼び出しコストを計測する。
///
/// example-plugin は `handle_core` 内で `PLUGIN_SHOULD_PANIC` を毎リクエスト参照する。
/// これが plugin レイテンシの主要因のひとつかどうかを確認する。
fn bench_env(c: &mut Criterion) {
    let mut group = c.benchmark_group("env");

    // std::env::var 1 回
    group.bench_function("var_once", |b| {
        b.iter(|| {
            let _ = black_box(std::env::var(black_box("PLUGIN_SHOULD_PANIC")));
        });
    });

    // std::env::var 2 回（example-plugin が参照する変数数）
    group.bench_function("var_twice", |b| {
        b.iter(|| {
            let _ = black_box(std::env::var(black_box("PLUGIN_SHOULD_PANIC")));
            let _ = black_box(std::env::var(black_box("PLUGIN_RESET")));
        });
    });

    group.finish();
}

// ─── routing グループ ─────────────────────────────────────────────────────────

/// `PluginRouter` が行う最長プレフィックス検索のコストを計測する。
///
/// - `prefix_1`: プラグイン 1 つ登録時のルーティング
/// - `prefix_2`: プラグイン 2 つ登録時のルーティング（実際の使用例）
fn bench_routing(c: &mut Criterion) {
    let mut group = c.benchmark_group("routing");

    // 1 エントリ
    let mut map1: HashMap<String, ()> = HashMap::new();
    map1.insert("/api".to_string(), ());

    group.bench_function("prefix_1", |b| {
        b.iter(|| {
            let path = black_box("/api/hello");
            let result = map1
                .keys()
                .filter(|prefix| path.starts_with(prefix.as_str()))
                .max_by_key(|prefix| prefix.len())
                .cloned();
            black_box(result);
        });
    });

    // 2 エントリ（make run のデフォルト構成）
    let mut map2: HashMap<String, ()> = HashMap::new();
    map2.insert("/api".to_string(), ());
    map2.insert("/sample".to_string(), ());

    group.bench_function("prefix_2", |b| {
        b.iter(|| {
            let path = black_box("/api/hello");
            let result = map2
                .keys()
                .filter(|prefix| path.starts_with(prefix.as_str()))
                .max_by_key(|prefix| prefix.len())
                .cloned();
            black_box(result);
        });
    });

    group.finish();
}

// ─── OnceLock グループ ────────────────────────────────────────────────────────

/// `OnceLock<Mutex<T>>` の初期化済みアクセスコストを計測する。
///
/// プラグインの `__PLUGIN_STATE` は `OnceLock<Mutex<State>>` であり、
/// `handle` のたびに `get_or_init` を経由してミューテックスを取得する。
/// 初期化済み状態での `get_or_init` のコストを単体計測する。
fn bench_once_lock(c: &mut Criterion) {
    let mut group = c.benchmark_group("once_lock");

    static CELL: OnceLock<Mutex<u64>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(0u64));

    // OnceLock::get_or_init（初期化済み、fast path）
    group.bench_function("get_or_init", |b| {
        b.iter(|| {
            let m = CELL.get_or_init(|| Mutex::new(0u64));
            let mut g = m.lock().unwrap();
            *g = black_box(g.wrapping_add(1));
        });
    });

    group.finish();
}

// ─── catch_unwind グループ ────────────────────────────────────────────────────

/// `std::panic::catch_unwind` の呼び出しコストを計測する。
///
/// プラグインの `__handle` / `__plugin_handle_ref` はパニック検知のため
/// 全リクエストを `catch_unwind(AssertUnwindSafe(|| ...))` でラップする。
/// パニックが起きない場合（通常パス）でもランディングパッド設置のオーバーヘッドがある。
fn bench_catch_unwind(c: &mut Criterion) {
    let mut group = c.benchmark_group("catch_unwind");

    // 直接クロージャ実行（ベースライン）
    group.bench_function("direct_closure", |b| {
        let mut x = 0u64;
        b.iter(|| {
            x = black_box(x.wrapping_add(1));
        });
    });

    // catch_unwind でラップした同じクロージャ（プラグイン handle の構造に相当）
    group.bench_function("wrapped", |b| {
        let mut x = 0u64;
        b.iter(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                black_box(x.wrapping_add(1))
            }));
            x = black_box(result.unwrap());
        });
    });

    // catch_unwind + Mutex lock（プラグインの実際のパターンに近い）
    group.bench_function("wrapped_with_mutex", |b| {
        static CELL: OnceLock<Mutex<u64>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(0u64));
        b.iter(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut g = CELL.get_or_init(|| Mutex::new(0u64)).lock().unwrap();
                *g = black_box(g.wrapping_add(1));
                black_box(*g)
            }));
            black_box(result.unwrap());
        });
    });

    group.finish();
}

// ─── plugin_inner グループ ────────────────────────────────────────────────────

/// プラグインの実処理パスをホスト内で段階的に再現してコストを積み上げる。
///
/// `hello/plugin_rstr`（216 ns）と `hello/native`（41 ns）の差（〜175 ns）の
/// 内訳を FFI・ルーティングを除いた純粋なロジック側から分析する。
///
/// - `response_build`       : format! + HttpResponse 構築のみ（native と同等の処理）
/// - `mutex_response`       : OnceLock::get_or_init + Mutex::lock + response_build
/// - `catch_unwind_full`    : catch_unwind + OnceLock + Mutex + response_build
///                            （__plugin_handle_ref の内側を再現）
fn bench_plugin_inner(c: &mut Criterion) {
    let mut group = c.benchmark_group("plugin_inner");

    // response_build: native_hello と同じ処理をプラグイン側のコードとして実行
    // （native のベースラインと比較するためここに含める）
    group.bench_function("response_build", |b| {
        let mut count = 0u64;
        b.iter(|| {
            count = black_box(count.wrapping_add(1));
            black_box(HttpResponse {
                status: 200,
                content_type: "application/json".into(),
                body: RVec::from(format!(r#"{{"message":"hello","count":{count}}}"#).into_bytes()),
            });
        });
    });

    // mutex_response: OnceLock + Mutex ロック込みでレスポンスを構築
    group.bench_function("mutex_response", |b| {
        static CELL: OnceLock<Mutex<u64>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(0u64));
        b.iter(|| {
            let mut g = CELL.get_or_init(|| Mutex::new(0u64)).lock().unwrap();
            *g = black_box(g.wrapping_add(1));
            let count = *g;
            black_box(HttpResponse {
                status: 200,
                content_type: "application/json".into(),
                body: RVec::from(format!(r#"{{"message":"hello","count":{count}}}"#).into_bytes()),
            });
        });
    });

    // catch_unwind_full: __plugin_handle_ref の内部処理をそのまま模倣
    // FFI 境界とルーティングだけ除いた "プラグイン側" のコストを計測
    group.bench_function("catch_unwind_full", |b| {
        static CELL2: OnceLock<Mutex<u64>> = OnceLock::new();
        CELL2.get_or_init(|| Mutex::new(0u64));
        b.iter(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut g = CELL2.get_or_init(|| Mutex::new(0u64)).lock().unwrap();
                *g = black_box(g.wrapping_add(1));
                let count = *g;
                HttpResponse {
                    status: 200,
                    content_type: "application/json".into(),
                    body: RVec::from(
                        format!(r#"{{"message":"hello","count":{count}}}"#).into_bytes(),
                    ),
                }
            }));
            black_box(result.unwrap());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_call,
    bench_mutex,
    bench_alloc,
    bench_env,
    bench_routing,
    bench_once_lock,
    bench_catch_unwind,
    bench_plugin_inner,
);
criterion_main!(benches);
