.PHONY: fmt wasm-preview

WASM_TARGETS = wasm-mls-mpm wasm-preview
CLIPPY_CRATES = --exclude wasm-mls-mpm --workspace
CLIPPY_WASM = -p wasm-mls-mpm --target wasm32-unknown-unknown

fmt:
	cargo fmt
	git add -u
	cargo clippy --fix --allow-staged $(CLIPPY_CRATES)
	cargo clippy --fix --allow-staged $(CLIPPY_WASM)

check-fmt:
	cargo fmt --check
	cargo clippy $(CLIPPY_CRATES)
	cargo clippy $(CLIPPY_WASM)


test:
	cargo test --workspace --exclude wasm-mls-mpm --exclude wasm-preview
	for target in $(WASM_TARGETS); do \
		make -C examples/$$target test; \
	done

# wasm-previewの開始
wasm-preview:
	make -C examples/wasm-preview build
	cd examples/preview-server && cargo run
