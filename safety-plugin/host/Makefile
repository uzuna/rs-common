# safety-plugin-host Makefile
#
# 使い方:
#   make build           - ビルド（全プラグイン + ホスト）
#   make run             - ホストを起動（/api: example, /sample: sample）
#   make reload-api      - /api プラグインをアップロードしてリロード
#   make reload-sample   - /sample プラグインをアップロードしてリロード
#   make status-api      - /api プラグインの状態を確認
#   make status-sample   - /sample プラグインの状態を確認
#   make versions-api    - /api プラグインのバージョン一覧
#   make rollback-api V=1 - /api プラグインをバージョン 1 にロールバック
#   make hello           - GET /api/hello を叩いて動作確認
#   make add             - POST /sample/add を叩いて動作確認
#   make mul             - POST /sample/mul を叩いて動作確認
#   make test            - 全テストを実行

WORKSPACE_ROOT := ../..
EXAMPLE_SO     := $(WORKSPACE_ROOT)/target/debug/libsafety_plugin_example.so
SAMPLE_SO      := $(WORKSPACE_ROOT)/target/debug/libsafety_plugin_sample.so
HOST_ADDR      ?= http://localhost:8080
PORT           ?= 8080
PLUGIN_DIR     ?= /tmp/safety-plugin-versions

.PHONY: build build-example build-sample build-host run run-fallback \
        reload-api reload-sample \
        status-api status-sample \
        versions-api versions-sample \
        rollback-api rollback-sample \
        prefixes hello add mul test clean

# ─── ビルド ──────────────────────────────────────────────────────────────────

## 全プラグインとホストをビルドする
build: build-example build-sample build-host

## example-plugin (.so) をビルドする
build-example:
	cargo build -p safety-plugin-example

## sample-plugin (.so) をビルドする
build-sample:
	cargo build -p safety-plugin-sample

## safety-plugin-host バイナリをビルドする
build-host:
	cargo build -p safety-plugin-host

# ─── 起動 ────────────────────────────────────────────────────────────────────

## 両プラグインを指定してホストを起動する
## /api  → example-plugin (GET /api/hello, POST /api/echo)
## /sample → sample-plugin (POST /sample/add, POST /sample/mul, GET /sample/status)
run: build
	cargo run -p safety-plugin-host -- \
		--plugin /api:$(EXAMPLE_SO) \
		--plugin /sample:$(SAMPLE_SO) \
		--addr 0.0.0.0:$(PORT) \
		--plugin-dir $(PLUGIN_DIR)

## プラグインなしのフォールバックモードで起動する
run-fallback: build-host
	cargo run -p safety-plugin-host -- \
		--addr 0.0.0.0:$(PORT) \
		--plugin-dir $(PLUGIN_DIR)

# ─── プラグイン管理 API ───────────────────────────────────────────────────────

## /api プラグインをアップロードしてリロードする
reload-api: build-example
	@echo ">>> POST /plugin/api/reload"
	curl -s -X POST $(HOST_ADDR)/plugin/api/reload \
		--data-binary @$(EXAMPLE_SO) \
		-H "Content-Type: application/octet-stream" | jq .

## /sample プラグインをアップロードしてリロードする
reload-sample: build-sample
	@echo ">>> POST /plugin/sample/reload"
	curl -s -X POST $(HOST_ADDR)/plugin/sample/reload \
		--data-binary @$(SAMPLE_SO) \
		-H "Content-Type: application/octet-stream" | jq .

## /api プラグインの現在の状態を確認する
status-api:
	@echo ">>> GET /plugin/api/status"
	curl -s $(HOST_ADDR)/plugin/api/status | jq .

## /sample プラグインの現在の状態を確認する
status-sample:
	@echo ">>> GET /plugin/sample/status"
	curl -s $(HOST_ADDR)/plugin/sample/status | jq .

## /api のバージョン一覧を表示する
versions-api:
	@echo ">>> GET /plugin/api/versions"
	curl -s $(HOST_ADDR)/plugin/api/versions | jq .

## /sample のバージョン一覧を表示する
versions-sample:
	@echo ">>> GET /plugin/sample/versions"
	curl -s $(HOST_ADDR)/plugin/sample/versions | jq .

## /api プラグインを指定バージョンにロールバックする（例: make rollback-api V=1）
rollback-api:
ifndef V
	$(error V を指定してください。例: make rollback-api V=1)
endif
	@echo ">>> POST /plugin/api/rollback/$(V)"
	curl -s -X POST $(HOST_ADDR)/plugin/api/rollback/$(V) | jq .

## /sample プラグインを指定バージョンにロールバックする（例: make rollback-sample V=1）
rollback-sample:
ifndef V
	$(error V を指定してください。例: make rollback-sample V=1)
endif
	@echo ">>> POST /plugin/sample/rollback/$(V)"
	curl -s -X POST $(HOST_ADDR)/plugin/sample/rollback/$(V) | jq .

## 登録済みプレフィックス一覧を表示する
prefixes:
	@echo ">>> GET /plugin/prefixes"
	curl -s $(HOST_ADDR)/plugin/prefixes | jq .

# ─── 動作確認 ─────────────────────────────────────────────────────────────────

## GET /api/hello を叩いて動作確認する
hello:
	@echo ">>> GET /api/hello"
	curl -s $(HOST_ADDR)/api/hello | jq .

## POST /sample/add を叩いて加算確認する
add:
	@echo ">>> POST /sample/add"
	curl -s -X POST $(HOST_ADDR)/sample/add \
		-H "Content-Type: application/json" \
		-d '{"a":3,"b":4}' | jq .

## POST /sample/mul を叩いて乗算確認する
mul:
	@echo ">>> POST /sample/mul"
	curl -s -X POST $(HOST_ADDR)/sample/mul \
		-H "Content-Type: application/json" \
		-d '{"a":3,"b":4}' | jq .

## GET /sample/status を叩いて演算回数を確認する
sample-status:
	@echo ">>> GET /sample/status"
	curl -s $(HOST_ADDR)/sample/status | jq .

# ─── テスト・整備 ─────────────────────────────────────────────────────────────

## 全テストを実行する（直列実行で env var の競合を防ぐ）
test: build-example build-sample
	cargo test -p safety-plugin-host -- --test-threads=1

## criterion ベンチマークを実行する（native vs plugin レイテンシ比較、debug .so）
bench: build-example build-sample
	cargo bench -p safety-plugin-host

## criterion ベンチマークを release .so で実行する（最適化効果を確認）
bench-release:
	cargo build -p safety-plugin-example -p safety-plugin-sample --release
	SAFETY_PLUGIN_EXAMPLE_PATH=$(WORKSPACE_ROOT)/target/release/libsafety_plugin_example.so \
	SAFETY_PLUGIN_SAMPLE_PATH=$(WORKSPACE_ROOT)/target/release/libsafety_plugin_sample.so \
	cargo bench -p safety-plugin-host

## FFI・Mutex・アロケーション等の要素ベンチマークを実行する
bench-overhead:
	cargo bench -p safety-plugin-host --bench ffi_overhead

## 10,000 回リロード時のメモリ使用量を計測する
memory-check: build-example
	cargo test -p safety-plugin-host reload_memory_check_10000 -- --ignored --nocapture

## ビルド成果物を削除する
clean:
	cargo clean -p safety-plugin-host -p safety-plugin-example -p safety-plugin-sample
	rm -rf $(PLUGIN_DIR)

# ─── ヘルプ ──────────────────────────────────────────────────────────────────

help:
	@echo "利用可能なターゲット:"
	@echo "  build            全プラグインとホストをビルド"
	@echo "  run              両プラグインでホストを起動"
	@echo "  run-fallback     プラグインなしで起動"
	@echo "  reload-api       /api プラグインをリロード"
	@echo "  reload-sample    /sample プラグインをリロード"
	@echo "  status-api       /api プラグイン状態を確認"
	@echo "  status-sample    /sample プラグイン状態を確認"
	@echo "  versions-api     /api バージョン一覧"
	@echo "  versions-sample  /sample バージョン一覧"
	@echo "  rollback-api V=N /api を v N にロールバック"
	@echo "  rollback-sample V=N /sample を v N にロールバック"
	@echo "  prefixes         登録済みプレフィックス一覧"
	@echo "  hello            GET /api/hello で動作確認"
	@echo "  add              POST /sample/add で加算確認"
	@echo "  mul              POST /sample/mul で乗算確認"
	@echo "  test             全テストを実行"
	@echo "  bench            criterion ベンチマーク debug .so（native vs plugin）"
	@echo "  bench-release    criterion ベンチマーク release .so（最適化効果を確認）"
	@echo "  bench-overhead   FFI・Mutex・アロケーション等の要素ベンチマーク"
	@echo "  memory-check     10,000 回リロードのメモリ計測"
	@echo "  clean            ビルド成果物を削除"
	@echo ""
	@echo "環境変数:"
	@echo "  HOST_ADDR        接続先アドレス（デフォルト: http://localhost:8080）"
	@echo "  PORT             リッスンポート（デフォルト: 8080）"
	@echo "  PLUGIN_DIR       バージョンストレージ（デフォルト: /tmp/safety-plugin-versions）"
