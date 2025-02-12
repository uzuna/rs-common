include ../../common.mk

.PHONY: test
test:
	wasm-pack test --firefox --headless

.PHONY: build
build:
	RUSTFLAGS='--cfg getrandom_backend="wasm_js"' wasm-pack build -d ${ASSETS_DIR}/mls-mpm/pkg --target web
