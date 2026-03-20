# safety-plugin-host

ロボット制御向けホットリロード可能プラグインホスト。
プラグインがパニックしてもホストは停止せず、HTTP API でゼロダウンタイムのプラグイン更新が可能。

## アーキテクチャ概要

```
┌─────────────────────────────────────────────────────┐
│  safety-plugin-host (axum HTTP サーバー)             │
│                                                     │
│  GET/POST /api/* ──► PluginManager::handle()        │
│                         │                           │
│                         ▼                           │
│              [example-plugin.so]                    │
│              extern "C" fn handle(req) -> resp       │
│              catch_unwind → 500 on panic             │
│                                                     │
│  POST /plugin/reload ──► VersionManager::save()     │
│                      ──► PluginManager::reload()    │
│  GET  /plugin/status ──► 状態レポート               │
│  GET  /plugin/versions──► バージョン一覧            │
│  POST /plugin/rollback/:v ► 旧バージョンに戻す      │
└─────────────────────────────────────────────────────┘
```

プラグインは `cdylib` として別クレートでビルドされ、ABI 安定化に `abi_stable` を使用。
ホットリロード時に `shutdown()` → `init(prev_state)` で内部状態を引き継ぐ。

---

## クイックスタート

### 1. ビルド

```bash
make build
# または個別に:
make build-plugin   # example-plugin (.so) のみ
make build-host     # ホストバイナリのみ
```

### 2. ホストを起動

```bash
make run
# 別ターミナルで起動（デフォルト: 0.0.0.0:8080）
```

カスタムポートやディレクトリを指定する場合:

```bash
make run PORT=9090 PLUGIN_DIR=/tmp/my-plugins
# または直接:
cargo run -p safety-plugin-host -- \
    --plugin ../../target/debug/libsafety_plugin_example.so \
    --addr 0.0.0.0:9090 \
    --plugin-dir /tmp/my-plugins \
    --max-versions 5
```

### 3. 動作確認

```bash
make hello      # GET /api/hello → JSON レスポンス
make echo       # POST /api/echo → ボディをそのまま返す
make status     # プラグイン状態を確認
```

---

## HTTP API リファレンス

### プラグイン管理 API

#### `POST /plugin/reload`

.so バイナリをアップロードしてプラグインをホットリロードする。

```bash
curl -X POST http://localhost:8080/plugin/reload \
  --data-binary @../../target/debug/libsafety_plugin_example.so \
  -H "Content-Type: application/octet-stream"
```

レスポンス:
```json
{
  "version": 3,
  "status": "loaded",
  "routes": ["/api"]
}
```

- `status`: `"loaded"` または `"fallback"`（リロード失敗時は旧バージョンで継続）

#### `GET /plugin/status`

現在のプラグイン状態を返す。

```bash
curl http://localhost:8080/plugin/status
```

```json
{
  "loaded": true,
  "version": 3,
  "routes": ["/api"],
  "fallback_count": 0
}
```

- `version`: `null` はファイル監視経由のリロード（API 未使用）
- `fallback_count`: プラグイン未ロードでリクエストが来た累計回数

#### `GET /plugin/versions`

保持しているバージョン一覧を返す。

```bash
curl http://localhost:8080/plugin/versions
```

```json
{
  "current": 3,
  "versions": [
    {"version": 1, "saved_at": 1700000001, "path": "/tmp/.../plugin_v1.so"},
    {"version": 2, "saved_at": 1700000060, "path": "/tmp/.../plugin_v2.so"},
    {"version": 3, "saved_at": 1700000120, "path": "/tmp/.../plugin_v3.so"}
  ]
}
```

#### `POST /plugin/rollback/:version`

指定バージョンのプラグインに切り替える。

```bash
# バージョン 1 にロールバック
curl -X POST http://localhost:8080/plugin/rollback/1
# または Makefile 経由:
make rollback V=1
```

```json
{
  "version": 1,
  "status": "loaded",
  "routes": ["/api"]
}
```

---

### プラグインが担当する API（example-plugin）

#### `GET /api/hello`

```bash
curl http://localhost:8080/api/hello
```

```json
{"message": "hello", "count": 42}
```

- `count`: 累積リクエスト処理数（ホットリロードをまたいで引き継がれる）

#### `POST /api/echo`

```bash
curl -X POST http://localhost:8080/api/echo -d "hello"
# → "hello"（ボディをそのまま返す）
```

---

## ホットリロードワークフロー

### ケース 1: ファイル監視（自動）

プラグインの .so ファイルを上書きすると自動リロードされる。

```bash
# ターミナル 1: ホストを起動
make run

# ターミナル 2: プラグインを再ビルド
make build-plugin
# → notify がファイル変更を検知して自動リロード
```

### ケース 2: HTTP API 経由（手動・CI/CD 向け）

```bash
# 1. プラグインをビルド
make build-plugin

# 2. アップロードしてリロード
make reload

# 3. 動作確認
make hello

# 4. 問題があればロールバック
make versions       # バージョン一覧を確認
make rollback V=1   # バージョン 1 に戻す
```

### ケース 3: SIGUSR1 で手動トリガー

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

---

## フォールバック動作

| 状況 | レスポンス |
|------|-----------|
| プラグイン未ロード | 503 Service Unavailable |
| プラグインが未登録パスを受信 | 404 Not Found（ホスト側） |
| プラグインがパニック | 500 Internal Server Error（ホストは継続） |
| リロード失敗 | 旧バージョンで継続（Err を返すが動作継続） |

---

## テスト

```bash
make test
# または:
cargo test -p safety-plugin-host -- --test-threads=1
```

全テストは直列実行（`--test-threads=1`）が必要。
`abi_stable` のプロセスレベルキャッシュと共有グローバル状態（`STATE`）の競合を避けるため。

---

## 環境変数（example-plugin 用）

| 変数 | 値 | 説明 |
|------|----|------|
| `PLUGIN_SHOULD_PANIC` | `1` | `handle()` 内で意図的にパニック（テスト用） |
| `PLUGIN_RESET` | `1` | `init()` で request_count を 0 にリセット（テスト用） |

---

## CLI オプション

```
USAGE:
    safety-plugin-host [OPTIONS]

OPTIONS:
    -p, --plugin <PATH>           初回ロードするプラグイン (.so) のパス
        --addr <ADDR>             HTTP サーバーのリッスンアドレス [デフォルト: 0.0.0.0:8080]
        --plugin-dir <DIR>        バージョン管理用ストレージディレクトリ [デフォルト: plugin-versions]
        --max-versions <N>        保持するバージョン数の上限 [デフォルト: 10]
    -h, --help                    ヘルプを表示
```
