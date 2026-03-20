# safety-plugin-host

停止を避けたい制御向けホットリロード可能プラグインホスト。
プラグインがパニックしてもホストは停止せず、HTTP API でゼロダウンタイムのプラグイン更新が可能。

## 所感

- ホストに制御インスタンスを作ったままロジックをプラグインで切り替えることはできる
- FFI 呼び出し自体のコストは < 1 ns。遅延の主因は Mutex・ルーティング・.so 非インライン化
- release .so + `RStr` ゼロコピーインターフェース（`plugin_rstr`）なら native との差は **hello で 100 ns、add で 12 ns**
- 参照渡し（`RStr<'a>` / `RSlice<'a>`）でホスト側のメモリコピーコストはゼロにできる

## アーキテクチャ概要

```
┌──────────────────────────────────────────────────────────────────┐
│  safety-plugin-host (axum HTTP サーバー)                          │
│                                                                  │
│  GET /api/hello ──► PluginRouter ──► [example-plugin.so]        │
│  POST /api/echo ──►    最長       ──►  extern "C" fn handle()   │
│                       プレフィックス    catch_unwind → 500 on panic│
│  POST /sample/add──►    一致       ──► [sample-plugin.so]       │
│  POST /sample/mul ──►  ルーティング ──►  extern "C" fn handle()  │
│                                                                  │
│  POST /plugin/{prefix}/reload    ──► VersionManager + reload    │
│  GET  /plugin/{prefix}/status    ──► ロード状態レポート           │
│  GET  /plugin/{prefix}/versions  ──► バージョン一覧              │
│  POST /plugin/{prefix}/rollback/{v}► 旧バージョンに戻す          │
│  GET  /plugin/prefixes           ──► 登録プレフィックス一覧       │
└──────────────────────────────────────────────────────────────────┘
```

- プラグインは `cdylib` として別クレートでビルドし、`define_http_plugin!` マクロで実装する。
- ABI 安定化に `abi_stable` を使用。異なる Rust コンパイラバージョン間で互換性を保つ。
- ホットリロード時に `shutdown()` → `init(prev_state)` で内部状態（カウンタ等）を引き継ぐ。
- `libloading` で直接 `.so` をロードし、`abi_stable` のプロセスグローバルキャッシュを迂回することで
  複数の異なるプラグインを同一プロセスで動かすことができる。

---

## クイックスタート

### 1. ビルド

```bash
make build
# または個別に:
make build-example   # example-plugin (.so)
make build-sample    # sample-plugin (.so)
make build-host      # ホストバイナリ
```

### 2. ホストを起動

```bash
make run
# → /api: example-plugin、/sample: sample-plugin で起動（デフォルト: 0.0.0.0:8080）
```

カスタムポートやディレクトリを指定する場合:

```bash
make run PORT=9090 PLUGIN_DIR=/tmp/my-plugins
# または直接:
cargo run -p safety-plugin-host -- \
    --plugin /api:../../target/debug/libsafety_plugin_example.so \
    --plugin /sample:../../target/debug/libsafety_plugin_sample.so \
    --addr 0.0.0.0:9090 \
    --plugin-dir /tmp/my-plugins
```

### 3. 動作確認

```bash
make hello          # GET /api/hello → JSON
make add            # POST /sample/add {"a":3,"b":4} → {"result":7}
make mul            # POST /sample/mul {"a":3,"b":4} → {"result":12}
make sample-status  # GET /sample/status → {"op_count":N}
make prefixes       # 登録済みプレフィックス一覧
```

---

## HTTP API リファレンス

### プラグイン管理 API

`{prefix}` は CLI 指定時の先頭 `/` を除いた文字列（例: `api`、`sample`）。

#### `POST /plugin/{prefix}/reload`

.so バイナリをアップロードしてプラグインをホットリロードする。

```bash
make reload-api     # /api プラグインをリロード
make reload-sample  # /sample プラグインをリロード
# または直接:
curl -X POST http://localhost:8080/plugin/api/reload \
  --data-binary @../../target/debug/libsafety_plugin_example.so \
  -H "Content-Type: application/octet-stream"
```

```json
{"prefix": "/api", "version": 3, "status": "loaded"}
```

`status` は `"loaded"`（成功）または `"fallback"`（失敗時は旧バージョンで継続）。

#### `GET /plugin/{prefix}/status`

```bash
make status-api
```

```json
{"prefix": "/api", "loaded": true, "version": 3, "fallback_count": 0}
```

#### `GET /plugin/{prefix}/versions`

```bash
make versions-api
```

```json
{
  "prefix": "/api",
  "current": 3,
  "versions": [
    {"version": 1, "saved_at": 1700000001, "path": "/tmp/.../plugin_v1.so"},
    {"version": 3, "saved_at": 1700000120, "path": "/tmp/.../plugin_v3.so"}
  ]
}
```

#### `POST /plugin/{prefix}/rollback/{version}`

```bash
make rollback-api V=1    # /api をバージョン 1 に戻す
make rollback-sample V=2 # /sample をバージョン 2 に戻す
```

#### `GET /plugin/prefixes`

```bash
make prefixes
```

```json
{"prefixes": ["/api", "/sample"]}
```

---

### プラグイン API（example-plugin）

#### `GET /api/hello`

```bash
curl http://localhost:8080/api/hello
```

```json
{"message": "hello", "count": 42}
```

`count` は累積リクエスト処理数（ホットリロードをまたいで引き継がれる）。

#### `POST /api/echo`

```bash
curl -X POST http://localhost:8080/api/echo -d "hello"
# → "hello"（ボディをそのまま返す）
```

---

### プラグイン API（sample-plugin）

#### `POST /sample/add` / `POST /sample/mul`

```bash
make add   # POST /sample/add  {"a":3,"b":4} → {"result":7}
make mul   # POST /sample/mul  {"a":3,"b":4} → {"result":12}
```

#### `GET /sample/status`

```bash
make sample-status
```

```json
{"op_count": 5}
```

`op_count` は add/mul の累積実行回数（ホットリロードをまたいで引き継がれる）。

---

## ホットリロードワークフロー

### ケース 1: ファイル監視（自動）

プラグインの .so ファイルを上書きすると `notify` が変更を検知して自動リロードされる。

```bash
# ターミナル 1
make run

# ターミナル 2: プラグインを再ビルド → 自動リロード
make build-example
```

### ケース 2: HTTP API 経由（CI/CD 向け）

```bash
make reload-api     # 新バイナリをアップロードしてリロード
make hello          # 動作確認
make versions-api   # バージョン一覧を確認
make rollback-api V=1  # 問題があればロールバック
```

### ケース 3: SIGUSR1 で全プラグインを手動リロード

```bash
kill -USR1 $(pgrep safety-plugin-host)
```

---

## 状態引き継ぎ

リロード時に `shutdown()` → `init(prev_state)` の流れで内部状態が引き継がれる。

```
リロード前: request_count = 42
   ↓ shutdown() → [42 as LE bytes]
 ホストが保存
   ↓ init(prev_state=[42 as LE bytes])
リロード後: request_count = 42  ← 引き継がれている
```

フォーマットはプラグインが自由に決める（example-plugin は little-endian u64、sample-plugin は JSON）。

---

## フォールバック動作

| 状況                             | レスポンス                                     |
| :------------------------------- | :--------------------------------------------- |
| プレフィックス未登録             | 404 Not Found（fallback_count に加算しない）   |
| プレフィックス登録済み・未ロード | 503 Service Unavailable（fallback_count 加算） |
| プラグイン内でパニック           | 500 Internal Server Error（ホストは継続）      |
| リロード失敗                     | 旧バージョンで継続（Err を返すが動作継続）     |

---

## ベンチマーク

criterion を使ったホスト直書き実装（native）と FFI 経由プラグイン実装（plugin）のレイテンシ比較。

```bash
make bench
# または:
cargo bench -p safety-plugin-host
```

### 計測結果

#### debug .so（`cargo build`）× bench バイナリ

| ハンドラ  |   native |   plugin | plugin_pooled |  plugin_rstr |
| :-------- | -------: | -------: | ------------: | -----------: |
| **hello** |  40.8 ns | 316.5 ns |      281.5 ns |     216.3 ns |
| **add**   | 145.4 ns | 1,641 ns |      1,591 ns |    1,515 ns  |

#### release .so（`cargo build --release`）× bench バイナリ

```bash
SAFETY_PLUGIN_EXAMPLE_PATH=target/release/libsafety_plugin_example.so \
SAFETY_PLUGIN_SAMPLE_PATH=target/release/libsafety_plugin_sample.so \
cargo bench -p safety-plugin-host --bench request_latency
```

| ハンドラ  |   native |   plugin | plugin_pooled |  plugin_rstr | plugin vs rstr |
| :-------- | -------: | -------: | ------------: | -----------: | -------------: |
| **hello** |  40.9 ns | 247.0 ns |      214.6 ns | **141.6 ns** | −105 ns (−43%) |
| **add**   | 143.9 ns | 246.9 ns |      218.2 ns | **155.9 ns** |  −91 ns (−37%) |

計測環境: Linux x86-64、bench バイナリは `--profile bench`（release 相当）

**release .so による改善:**

| ハンドラ | debug plugin | release plugin | debug rstr | release rstr |
| :------- | -----------: | -------------: | ---------: | -----------: |
| hello | 316.5 ns | 247.0 ns (−22%) | 216.3 ns | **141.6 ns (−35%)** |
| add | 1,641 ns | 246.9 ns (**−85%**) | 1,515 ns | **155.9 ns (−90%)** |

`add` の劇的な改善（−85%）は serde_json の最適化によるもの。
release .so での `add/plugin_rstr`（156 ns）は `native`（144 ns）との差がわずか **12 ns** まで縮まる。

### 要素ベンチマーク（`cargo bench --bench ffi_overhead`）

native と plugin の差（〜175 ns）の各要因を個別に計測した結果:

```bash
cargo bench -p safety-plugin-host --bench ffi_overhead
```

#### call グループ: 呼び出し方式のオーバーヘッド

| 計測項目 | 時間 | 備考 |
| :------- | ---: | :--- |
| `call/direct`（Rust 直接呼び出し） | 0.80 ns | ベースライン |
| `call/rust_fn_ptr`（Rust 関数ポインタ） | 1.19 ns | +0.4 ns |
| `call/extern_c_ptr`（`extern "C"` 関数ポインタ） | 1.26 ns | **+0.5 ns**（プラグイン FFI と同じ形式） |

→ **FFI 呼び出し自体のオーバーヘッドは < 1 ns**。支配的要因ではない。

#### mutex グループ: Mutex のロックコスト

| 計測項目 | 時間 |
| :------- | ---: |
| `mutex/lock_only`（ロック+解放のみ） | 8.0 ns |
| `mutex/lock_increment`（ロック+カウンタ更新） | 8.2 ns |

→ **Mutex lock ≈ 8 ns**。プラグインの状態アクセスごとに発生。

#### alloc グループ: 文字列表現の構築コスト（method / path / query × 3）

| 計測項目 | 時間 | 説明 |
| :------- | ---: | :--- |
| `alloc/rstr_borrow`（`RStr` ゼロコピー） | **14.3 ns** | ポインタ+長さのみ、アロケーションなし |
| `alloc/rstring_new`（`RString` 新規） | 21.0 ns | `plugin` 版のホスト側コスト |
| `alloc/rstring_pool`（プール・コールド） | 40.6 ns | TLS + RefCell オーバーヘッドが malloc より大きい |
| `alloc/rstring_pool_warm`（プール・ウォーム） | 46.1 ns | clear + push_str のみだが TLS 往復が重い |

→ **debug ビルドでは `RString` 新規アロケーション（21 ns）がプール再利用（46 ns）より速い**。
TLS アクセスと RefCell の borrow/unborrow コストが malloc のキャッシュヒットコストを上回るため。

#### env グループ: `std::env::var` のコスト

| 計測項目 | 時間 |
| :------- | ---: |
| `env/var_once` | 37.7 ns |
| `env/var_twice` | 73.9 ns |

→ `std::env::var` は **1 回あたり約 37 ns**。example-plugin が `PLUGIN_SHOULD_PANIC` を
毎リクエスト参照していた頃は 74 ns のオーバーヘッドが発生していた（現在は `#[cfg(test)]` で除外済み）。

#### routing グループ: プレフィックス検索コスト

| 計測項目 | 時間 |
| :------- | ---: |
| `routing/prefix_1`（1 エントリ） | 19.0 ns |
| `routing/prefix_2`（2 エントリ） | 22.7 ns |

→ **ルーティング ≈ 22 ns**（2 プラグイン構成）。エントリ数増加の影響は小さい。

#### catch_unwind グループ: パニック検知ラッパのコスト

| 計測項目 | 時間 |
| :------- | ---: |
| `catch_unwind/direct_closure` | 0.20 ns |
| `catch_unwind/wrapped`（catch_unwind のみ） | 0.30 ns |
| `catch_unwind/wrapped_with_mutex` | 8.17 ns |

→ **`catch_unwind` 自体のオーバーヘッドは +0.1 ns（実質ゼロ）**。コストはほぼ Mutex のみ。

#### plugin_inner グループ: プラグイン処理をインライン再現したコスト

| 計測項目 | 時間 | 説明 |
| :------- | ---: | :--- |
| `plugin_inner/response_build` | 37.5 ns | format! + HttpResponse 構築（native と同等） |
| `plugin_inner/mutex_response` | 43.9 ns | OnceLock + Mutex + レスポンス構築 |
| `plugin_inner/catch_unwind_full` | 36.7 ns | catch_unwind + Mutex + レスポンス構築 |

→ **プラグインの処理ロジックをベンチバイナリ内で再現すると native（41 ns）とほぼ同等**。
.so バウンダリを経由した場合（plugin_rstr）との差：

| | plugin_rstr 実測値 | catch_unwind_full（インライン） | 差（.so バウンダリ分） |
|:--|--:|--:|--:|
| debug .so | 216 ns | 37 ns | **−179 ns** ← .so 非インライン化が主因 |
| release .so | 141 ns | 37 ns | **−104 ns** ← ルーティング(23)+Mutex(8)+env_var(38)+PLT(31)≒100 ns |

release .so での残差（104 ns）は測定コンポーネントでほぼ説明できる。
debug .so での超過分（179 − 104 = **75 ns**）が .so 非インライン化によるオーバーヘッド。

### hello/plugin_rstr のコスト積み上げ（debug .so / release .so 比較）

| 要因 | debug .so | release .so | 計測元 |
| :--- | --------: | ----------: | :----- |
| native と同等の処理（format! + レスポンス構築） | 41 ns | 41 ns | `hello/native` |
| プレフィックスルーティング（2 エントリ） | 23 ns | 23 ns | `routing/prefix_2` |
| OnceLock + Mutex lock | 8 ns | 8 ns | `once_lock/get_or_init` |
| `extern "C"` 関数ポインタ呼び出し | < 1 ns | < 1 ns | `call/extern_c_ptr` |
| `catch_unwind` | < 1 ns | < 1 ns | `catch_unwind/wrapped` |
| `PLUGIN_SHOULD_PANIC` env_var チェック | 38 ns | 38 ns | `env/var_once` |
| **測定コンポーネント合計** | **〜111 ns** | **〜111 ns** | |
| .so 非インライン化オーバーヘッド † | **〜105 ns** | **〜31 ns** | 差分 |
| **hello/plugin_rstr 実測値** | **216 ns** | **141 ns** | |

† .so 内の関数（format!、trait メソッド、RVec 操作等）は debug では非インライン化される。
release .so ではこれらが最適化されて **74 ns 削減**。残る 31 ns は動的リンクの PLT 呼び出し（malloc 等）に起因。

`plugin_inner/catch_unwind_full`（37 ns）はベンチバイナリ内でのインライン実行で、
理論上の最小値に相当。実際の .so 境界を経由すると release でも +31 ns のオーバーヘッドが残る。

### 3 種類の FFI インターフェース比較

| インターフェース                  | ホスト側アロケーション   | debug .so | release .so |
| :-------------------------------- | :----------------------- | --------: | ----------: |
| `plugin`（`HttpRequest`）         | `RString` × 3 + `String` |  316.5 ns |    247.0 ns |
| `plugin_pooled`                   | `String` × 1（プール後） |  281.5 ns |    214.6 ns |
| `plugin_rstr`（`HttpRequestRef`） | **ゼロ**                 |  216.3 ns | **141.6 ns** |

**pool に関する注意**: `ffi_overhead/alloc` ベンチが示すとおり、bench バイナリ内のプール操作（TLS + RefCell）は
debug/release ともに 45 ns 程度かかり、新規 `malloc`（21 ns）より重い。
にもかかわらず `plugin_pooled` が `plugin` より速いのは、PoolRouter 全体のフローで見ると
`String::to_owned()` → `rstring_from_pool` のサイクルで割り当て済みメモリを再利用できるためで、
特に release .so では serde_json 等の最適化と相まって −32 ns の効果がある。

### RStr インターフェースの設計

`HttpRequestRef<'a>` は `RStr<'a>` フィールドを持つ借用型で、`&str` と同じABI安定レイアウト:

```rust
#[repr(C)]
#[derive(StableAbi)]
pub struct HttpRequestRef<'a> {
    pub method: RStr<'a>,   // ポインタ + 長さのみ（アロケーションなし）
    pub path:   RStr<'a>,
    pub query:  RStr<'a>,
    pub body:   RSlice<'a, u8>,
}
```

プラグイン側は `define_http_plugin!` に `handler_ref: fn_name` を追加するだけで対応できる:

```rust
define_http_plugin! {
    name: "my-plugin",
    state: MyState,
    handler: handle_inner,        // HttpRequest（所有型）版: 後方互換
    handler_ref: handle_ref_inner, // HttpRequestRef（借用型）版: ゼロコピー
    state_save: save,
    state_load: load,
}

fn handle_ref_inner(req: &HttpRequestRef<'_>, state: &mut MyState) -> HttpResponse {
    match req.path.as_str() { ... }  // RStr も .as_str() で &str を得られる
}
```

`handler_ref` を省略した場合はマクロが変換ラッパを自動生成するため、
古いプラグインでも `handle_ref` エンドポイントへのフォールバックが可能。

---

## メモリ使用量（10,000 回リロード）

```bash
make memory-check
# または:
cargo test -p safety-plugin-host reload_memory_check_10000 -- --ignored --nocapture
```

### 計測結果

```
[メモリ計測開始] VmSize: 72,272 KB / VmRSS: 3,580 KB
  リロード数    VmSize(KB)     VmRSS(KB)    ΔVmSize(KB)    ΔVmRSS(KB)
----------------------------------------------------------------------
    1,000        72,272         3,716             +0          +136
    2,000        72,272         3,776             +0          +196
    3,000        72,272         3,840             +0          +260
    ...
   10,000        72,272         4,288             +0          +708
----------------------------------------------------------------------
[結果] 10,000 回リロード後: ΔVmSize=+0 KB / ΔVmRSS=+708 KB
```

### 考察

- **VmSize（仮想アドレス空間）が +0 KB** — `dlclose` でライブラリセグメントが正常に解放されている。
- **VmRSS は約 64 KB/1,000 回**（≈ 64 バイト/リロード）で線形増加。
  `define_http_plugin!` マクロ内の `leak_into_prefix()` が
  リロードごとに `Box<RobotPlugin>`（≈ 32 バイト）をグローバルヒープに残留させることが原因。
  アロケータのアリーナ拡張分と合わせて 64 バイト/リロード程度。
- **10,000 回で +708 KB は実用上無視できる範囲**
  （頻繁なリロードを想定しても数 MB 以下）。

---

## テスト

```bash
make test
# または:
cargo test -p safety-plugin-host -- --test-threads=1
```

テストは直列実行が必要（環境変数 `PLUGIN_RESET` / `PLUGIN_SHOULD_PANIC` の競合を避けるため）。

| テストファイル                      | 内容                                           | テスト数 |
| :---------------------------------- | :--------------------------------------------- | :------: |
| `tests/plugin_integration.rs`       | example-plugin: 正常系・異常系・ホットリロード |    11    |
| `tests/sample_integration.rs`       | sample-plugin: add/mul/status・状態引き継ぎ    |    6     |
| `tests/multi_plugin_integration.rs` | 複数プラグイン同時運用・状態独立性             |    3     |
| `src/lib.rs` (unit)                 | PluginRouter / VersionManager ユニットテスト   |    9     |

---

## プラグイン開発ガイド

`define_http_plugin!` マクロを使うと `init` / `handle` / `shutdown` のボイラープレートなしに
プラグインを実装できる。

```rust
use safety_plugin_common::{define_http_plugin, HttpRequest, HttpResponse};

#[derive(Default)]
struct MyState {
    count: u64,
}

define_http_plugin! {
    name: "my-plugin",
    state: MyState,
    handler: handle_inner,
    state_save: save_state,
    state_load: load_state,
}

fn save_state(state: &MyState) -> Vec<u8> {
    state.count.to_le_bytes().to_vec()
}

fn load_state(bytes: &[u8]) -> Option<MyState> {
    let arr: [u8; 8] = bytes.try_into().ok()?;
    Some(MyState { count: u64::from_le_bytes(arr) })
}

fn handle_inner(req: &HttpRequest, state: &mut MyState) -> HttpResponse {
    state.count += 1;
    match req.path.as_str() {
        "/ping" => HttpResponse {
            status: 200,
            content_type: "text/plain".into(),
            body: b"pong".to_vec().into(),
        },
        _ => HttpResponse {
            status: 404,
            content_type: "text/plain".into(),
            body: b"not found".to_vec().into(),
        },
    }
}
```

`Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
abi_stable = "0.11"
safety-plugin-common = { path = "../common" }
```

起動時にプレフィックスを指定してロード:

```bash
cargo run -p safety-plugin-host -- \
    --plugin /ping:target/debug/libmy_plugin.so
```

---

## 環境変数（example-plugin 用）

| 変数                  | 値   | 説明                                                  |
| :-------------------- | :--- | :---------------------------------------------------- |
| `PLUGIN_SHOULD_PANIC` | `1`  | `handle()` 内で意図的にパニック（テスト用）           |
| `PLUGIN_RESET`        | `1`  | `init()` で request_count を 0 にリセット（テスト用） |

---

## CLI オプション

```
USAGE:
    safety-plugin-host [OPTIONS]

OPTIONS:
    -p, --plugin <PREFIX:PATH>    ロードするプラグイン（複数指定可）
                                  例: --plugin /api:target/debug/libexample.so
        --addr <ADDR>             リッスンアドレス [デフォルト: 0.0.0.0:8080]
        --plugin-dir <DIR>        バージョン管理ストレージ [デフォルト: plugin-versions]
        --max-versions <N>        保持バージョン数の上限 [デフォルト: 10]
    -h, --help                    ヘルプを表示
```
