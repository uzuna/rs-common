include ../../common.mk

RUNNER_CRATE := moonbit-runner
MOONBIT_CONTROL_DIR := $(PROJECT_DIR)/wasm-comp/moonbit-control
WASM := $(PROJECT_DIR)/examples/moonbit-runner/plugins/control.component.wasm
RAW_WASM := $(PROJECT_DIR)/examples/moonbit-runner/plugins/control.core.wasm
ITERATIONS ?= 20000
STATUS_ADDR ?= 127.0.0.1:8080
BENCH_ARGS ?=

.PHONY: help
help:
	@echo "moonbit-runner Makefile targets"
	@echo "  make check                  - moonbit-runner のコンパイル確認"
	@echo "  make test                   - moonbit-runner のユニットテスト"
	@echo "  make build-plugin           - component Wasm を生成"
	@echo "  make build-plugin-raw       - core Wasm を生成"
	@echo "  make build-plugin-all       - component/core Wasm を両方生成"
	@echo "  make run                    - component 経路で実行"
	@echo "  make run-raw                - component + raw 比較実行"
	@echo "  make bench                  - criterion で benchmark 実行"
	@echo "  make bench-raw              - criterion + raw 比較 benchmark 実行"
	@echo "  make serve-status           - /status サーバー起動（release）"
	@echo ""
	@echo "可変パラメータ"
	@echo "  ITERATIONS=<N>              - ベンチマーク回数（default: $(ITERATIONS)）"
	@echo "  STATUS_ADDR=<host:port>     - status bind アドレス（default: $(STATUS_ADDR)）"
	@echo "  BENCH_ARGS='<args>'         - cargo bench に渡す追加引数"

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

.PHONY: build-plugin-all
build-plugin-all:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) build-all

.PHONY: run
run: build-plugin
	cargo run -p $(RUNNER_CRATE) -- --wasm $(WASM)

.PHONY: run-raw
run-raw: build-plugin-all
	cargo run -p $(RUNNER_CRATE) -- --wasm $(WASM) --raw-wasm $(RAW_WASM)

.PHONY: bench
bench: build-plugin
	cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: bench-raw
bench-raw: build-plugin build-plugin-raw
	MOONBIT_RUNNER_RAW_BENCH=1 cargo bench -p $(RUNNER_CRATE) --bench criterion_bench -- $(BENCH_ARGS)

.PHONY: serve-status
serve-status: build-plugin
	cargo run --release -p $(RUNNER_CRATE) -- --wasm $(WASM) --iterations $(ITERATIONS) --serve-status --status-addr $(STATUS_ADDR)

.PHONY: clean-plugins
clean-plugins:
	$(MAKE) -C $(MOONBIT_CONTROL_DIR) clean
