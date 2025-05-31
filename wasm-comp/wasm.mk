include ../../common.mk

WASM_DIR:=$(PROJECT_DIR)/target/wasm32-unknown-unknown/release
INFO_CMD:=wasm-tools component wit $(WASM_DIR)

# 異なるターゲット向けのWasmコンポーネントをビルドする
.PHONY: build
build:
	cargo component build --target wasm32-unknown-unknown --release
	cargo component build --target wasm32-wasip2 --release

