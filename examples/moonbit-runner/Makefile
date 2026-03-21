include ../../common.mk

RUNNER_CRATE := moonbit-runner
MOONBIT_CONTROL_DIR := $(PROJECT_DIR)/wasm-comp/moonbit-control
WASM := $(PROJECT_DIR)/examples/moonbit-runner/plugins/control.component.wasm
RAW_WASM_SYMBOLIZED := $(PROJECT_DIR)/examples/moonbit-runner/plugins/control.core.symbolized.wasm
SERVER_ADDR ?= 127.0.0.1:8080
PLUGIN_PREFIX ?= /api
PLUGIN_DIR ?= plugin-versions
MAX_VERSIONS ?= 10
V ?= 1
BENCH_ARGS ?=
WASMTIME_PROFILER ?= perfmap
PERF_DATA ?= /tmp/moonbit-runner-add-loop2000.perf.data

.PHONY: help
help:
	@echo "moonbit-runner Makefile targets"
	@echo "  make check                  - moonbit-runner のコンパイル確認"
	@echo "  make test                   - moonbit-runner のユニットテスト"
	@echo "  make build-plugin           - component Wasm を生成"
	@echo "  make build-plugin-raw       - core Wasm を生成"
	@echo "  make build-plugin-raw-symbolized - 対応付用 core Wasm (debug/name付き) を生成"
	@echo "  make build-plugin-all       - component/core Wasm を両方生成"
	@echo "  make run                    - HTTP サーバーとして起動"
	@echo "  make reload-api             - curl で /plugin/reload に .wasm を POST する"
	@echo "  make rollback-api V=<n>     - バージョン指定ロールバック"
	@echo "  make plugin-status          - プラグイン管理ステータスを確認"
	@echo "  make plugin-versions        - バージョン一覧を確認"
	@echo "  make bench                  - criterion で benchmark 実行"
	@echo "  make bench-raw              - criterion + raw 比較 benchmark 実行"
	@echo "  make bench-raw-symbolized   - name section 付き raw Wasm で benchmark 実行"
	@echo "  make bench-raw-prof         - profiler 有効で raw benchmark 実行"
	@echo "  make perf-add-loop2000      - perf record で add_raw loop2000 を収集"
	@echo "  make perf-add-loop2000-symbolized - 対応付用ビルドで perf 収集"
	@echo ""
	@echo "可変パラメータ"
	@echo "  SERVER_ADDR=<host:port>     - HTTP サーバー bind アドレス（default: $(SERVER_ADDR)）"
	@echo "  PLUGIN_PREFIX=<path>        - Wasm 委譲プレフィックス（default: $(PLUGIN_PREFIX)）"
	@echo "  BENCH_ARGS='<args>'         - cargo bench に渡す追加引数"
	@echo "  WASMTIME_PROFILER=<name>    - none/perfmap/jitdump/vtune/pulley"
	@echo "  PERF_DATA=<path>            - perf record の出力先"

.PHONY: check
check:
	cargo check -p $(RUNNER_CRATE)

.PHONY: test
test:
	cargo test -p $(RUNNER_CRATE)

.PHONY: build-plugin
build-plugin:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) build

.PHONY: build-plugin-raw
build-plugin-raw:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) build-raw

.PHONY: build-plugin-raw-symbolized
build-plugin-raw-symbolized:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) build-raw-symbolized

.PHONY: build-plugin-all
build-plugin-all:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) build-all

.PHONY: run
run: build-plugin
	cargo run -p $(RUNNER_CRATE) -- --wasm $(WASM) --addr $(SERVER_ADDR) --prefix $(PLUGIN_PREFIX) --plugin-dir $(PLUGIN_DIR) --max-versions $(MAX_VERSIONS)

.PHONY: reload-api
reload-api:
	curl -s -X POST http://$(SERVER_ADDR)/plugin/reload \
		--data-binary @$(WASM) -H "Content-Type: application/octet-stream" | python3 -m json.tool

.PHONY: rollback-api
rollback-api:
	curl -s -X POST http://$(SERVER_ADDR)/plugin/rollback/$(V) | python3 -m json.tool

.PHONY: plugin-status
plugin-status:
	curl -s http://$(SERVER_ADDR)/plugin/status | python3 -m json.tool

.PHONY: plugin-versions
plugin-versions:
	curl -s http://$(SERVER_ADDR)/plugin/versions | python3 -m json.tool

.PHONY: bench
bench: build-plugin
	cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: bench-raw
bench-raw: build-plugin build-plugin-raw
	MOONBIT_RUNNER_RAW_BENCH=1 cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: bench-raw-symbolized
bench-raw-symbolized: build-plugin build-plugin-raw-symbolized
	MOONBIT_RUNNER_RAW_BENCH=1 MOONBIT_RUNNER_RAW_WASM_PATH=$(RAW_WASM_SYMBOLIZED) cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: bench-raw-prof
bench-raw-prof: build-plugin build-plugin-raw
	MOONBIT_RUNNER_RAW_BENCH=1 MOONBIT_RUNNER_WASMTIME_PROFILER=$(WASMTIME_PROFILER) cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: perf-add-loop2000
perf-add-loop2000: build-plugin build-plugin-raw
	MOONBIT_RUNNER_RAW_BENCH=1 MOONBIT_RUNNER_WASMTIME_PROFILER=$(WASMTIME_PROFILER) perf record -F 99 -g -o $(PERF_DATA) -- cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- --sample-size=30 --measurement-time=3 moonbit-runner/raw/add_raw/loop2000-16B
	@echo "perf report -i $(PERF_DATA)"

.PHONY: perf-add-loop2000-symbolized
perf-add-loop2000-symbolized: build-plugin build-plugin-raw-symbolized
	MOONBIT_RUNNER_RAW_BENCH=1 MOONBIT_RUNNER_RAW_WASM_PATH=$(RAW_WASM_SYMBOLIZED) MOONBIT_RUNNER_WASMTIME_PROFILER=$(WASMTIME_PROFILER) perf record -F 99 -g -o $(PERF_DATA) -- cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- --profile-time=8 moonbit-runner/raw/add_raw/loop2000-16B
	@echo "perf report -i $(PERF_DATA) --stdio --no-children --sort overhead,comm,dso,symbol"

.PHONY: clean-plugins
clean-plugins:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) clean
