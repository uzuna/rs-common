include ../../common.mk

.PHONY: run
run:
	cargo run -p wasmtime-runner -- --wasi none -n $(PROJECT_DIR)/target/wasm32-unknown-unknown/release/hello.wasm
	cargo run -p wasmtime-runner -- --wasi preview2 -n $(PROJECT_DIR)/target/wasm32-wasip2/release/hello.wasm
