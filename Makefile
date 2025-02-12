.PHONY: fmt wasm-preview

WASM_TARGETS = wasm-mls-mpm wasm-preview

fmt:
	cargo fmt
	git add -u
	cargo clippy --fix --allow-staged --exclude wasm-mls-mpm --workspace
	cargo clippy --fix --allow-staged -p wasm-mls-mpm --target wasm32-unknown-unknown

test:
	cargo test --workspace --exclude wasm-mls-mpm --exclude wasm-preview
	for target in $(WASM_TARGETS); do \
		make -C examples/$$target test; \
	done

# wasm-previewの開始
wasm-preview:
	make -C examples/wasm-preview build
	cd examples/preview-server && cargo run
