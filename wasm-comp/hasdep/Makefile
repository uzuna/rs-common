include ../wasm.mk

.PHONY: setup
setup:
	cargo install wac-cli

.PHONY: compose
compose:
	wac plug $(WASM_DIR)/hasdep.wasm --plug $(WASM_DIR)/calc.wasm -o composed.wasm

.PHONY: test-composed
test-composed:
	wasmtime run --invoke "add(2,5)" composed.wasm
