include ../../common.mk

WASM_DIR := $(PROJECT_DIR)/examples/wasmtime-runner/plugins

.PHONY: run
run:
	cargo run -p wasmtime-runner -- --config $(PROJECT_DIR)/examples/wasmtime-runner/config/config.yaml

.PHONY: cp
cp:
	cp $(PROJECT_DIR)/target/wasm32-unknown-unknown/release/hello.wasm $(WASM_DIR)/hello.wasm
	cp $(PROJECT_DIR)/target/wasm32-wasip2/release/hello.wasm $(WASM_DIR)/hello-wasip2.wasm
