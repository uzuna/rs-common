# safety-plugin-host

ロボット制御向けホットリロード可能プラグインホスト。
プラグインがパニックしてもホストは停止せず、HTTP API でゼロダウンタイムのプラグイン更新が可能。

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

| 状況 | レスポンス |
|:-----|:----------|
| プレフィックス未登録 | 404 Not Found（fallback_count に加算しない） |
| プレフィックス登録済み・未ロード | 503 Service Unavailable（fallback_count 加算） |
| プラグイン内でパニック | 500 Internal Server Error（ホストは継続） |
| リロード失敗 | 旧バージョンで継続（Err を返すが動作継続） |

---

## ベンチマーク

criterion を使ったホスト直書き実装（native）と FFI 経由プラグイン実装（plugin）のレイテンシ比較。

```bash
make bench
# または:
cargo bench -p safety-plugin-host
```

### 計測結果（`cargo bench` / debug ビルド）

| ハンドラ  | native | plugin（FFI 経由） | plugin_pooled | pool 改善 |
|:----------|-------:|------------------:|--------------:|----------:|
| **hello** | 44.9 ns |          377.9 ns |      356.9 ns | −21 ns (−5.6%) |
| **add**   |  153 ns |        1,632 ns   |    1,593 ns   | −39 ns (−2.4%) |

計測環境: Linux x86-64、`--profile dev`

### FFI オーバーヘッドの内訳

plugin 経由の追加コスト（hello の場合: 約 333 ns）:

| 要因 | 説明 |
|:-----|:-----|
| `RString` アロケーション × 3 | method / path / query の変換 |
| `String::to_owned()` | プレフィックス除去のための中間コピー |
| HashMap 最長プレフィックス検索 | PluginRouter のルーティング |
| `Mutex::lock` | プラグイン内部状態の排他制御 |
| 関数ポインタ経由 FFI 呼び出し | `extern "C" fn handle(req)` |
| `std::env::var` × 2 | example-plugin 実装固有（`PLUGIN_SHOULD_PANIC` 確認等） |

add の場合は上記に加えて serde_json による JSON パース/生成が含まれる（約 1,448 ns）。

### RString プールの効果と限界

`rstring_from_pool` / `rstring_to_pool` によるスレッドローカルプールを使うと、
定常状態で `method` / `query` の `RString` アロケーション（各 10–15 ns）を節約できる。

- **hello**: −21 ns (−5.6%) — `method`・`query` の 2 アロケーションを削減
- **add**: −39 ns (−2.4%) — 同上だが serde_json の JSON 処理（〜1,448 ns）が支配的

改善幅が限られる理由:
1. プレフィックス除去時に `String::to_owned()` が必要（`RString` 1 個分の節約と相殺）
2. `Mutex::lock` / FFI / `std::env::var` は削減できない
3. add では JSON パース/生成が全体の約 92% を占める

より抜本的なゼロアロケーションを目指す場合は `RStr<'a>`（借用 FFI 文字列型）を
使う設計変更が有効だが、`extern "C"` 関数ポインタ型の ABI 変更が必要になる。

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

| テストファイル | 内容 | テスト数 |
|:---|:---|:---:|
| `tests/plugin_integration.rs` | example-plugin: 正常系・異常系・ホットリロード | 11 |
| `tests/sample_integration.rs` | sample-plugin: add/mul/status・状態引き継ぎ | 6 |
| `tests/multi_plugin_integration.rs` | 複数プラグイン同時運用・状態独立性 | 3 |
| `src/lib.rs` (unit) | PluginRouter / VersionManager ユニットテスト | 9 |

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

| 変数 | 値 | 説明 |
|:-----|:---|:-----|
| `PLUGIN_SHOULD_PANIC` | `1` | `handle()` 内で意図的にパニック（テスト用） |
| `PLUGIN_RESET` | `1` | `init()` で request_count を 0 にリセット（テスト用） |

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
