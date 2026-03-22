include ../wasm.mk

BENCH_WASM := $(PROJECT_DIR)/target/wasm32-unknown-unknown/release/hello.wasm
FALLBACK_BENCH_WASM := $(PROJECT_DIR)/examples/wasmtime-runner/plugins/hello.wasm
BENCH_ARGS ?=

.PHONY: info
info:
	$(INFO_CMD)/calc.wasm

# Rust/Wasmバイナリ比較のベンチマーク
.PHONY: bench-prepare
bench-prepare:
	@if [ -f "$(BENCH_WASM)" ]; then \
		echo "Using benchmark wasm: $(BENCH_WASM)"; \
	elif [ -f "$(FALLBACK_BENCH_WASM)" ]; then \
		echo "Preparing benchmark wasm from fallback: $(FALLBACK_BENCH_WASM)"; \
		mkdir -p "$(dir $(BENCH_WASM))"; \
		cp "$(FALLBACK_BENCH_WASM)" "$(BENCH_WASM)"; \
	else \
		echo "Benchmark wasm not found."; \
		echo "Expected one of:"; \
		echo "  - $(BENCH_WASM)"; \
		echo "  - $(FALLBACK_BENCH_WASM)"; \
		echo "Please prepare hello.wasm before running bench."; \
		exit 1; \
	fi

.PHONY: bench
bench: bench-prepare
	cargo bench --bench wasm_call -- $(BENCH_ARGS)
