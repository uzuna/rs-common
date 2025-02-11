.PHONY: fmt wasm-preview

fmt:
	cargo fmt
	git add -u
	cargo clippy --fix --allow-staged --exclude wasm-mls-mpm --workspace
	cargo clippy --fix --allow-staged -p wasm-mls-mpm --target wasm32-unknown-unknown

# wasm-previewの開始
wasm-preview:
	make -C examples/wasm-preview build
	cd examples/preview-server && cargo run
