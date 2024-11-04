include ../../common.mk

.PHONY: test
test:
	wasm-pack test --firefox --headless

.PHONY: build
build:
	wasm-pack build -d ${ASSETS_DIR}/pkg --target web
