.PHONY: fmt wasm-preview

fmt:
	cargo fmt
	git add -u
	cargo clippy --fix --allow-staged

# wasm-previewの開始
wasm-preview:
	make -C examples/wasm-preview build
	cd examples/preview-server && cargo run
