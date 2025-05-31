# 異なるターゲット向けのWasmコンポーネントをビルドする
.PHONY: build
build:
	cargo component build -p wasm-plugin-hello --target wasm32-unknown-unknown --release
	cargo component build -p wasm-plugin-hello --target wasm32-wasip2 --release

# バイナリのインターフェース情報表示
.PHONY: info
info:
	wasm-tools component wit target/wasm32-unknown-unknown/release/hello.wasm

# Rust/Wasmバイナリ比較のベンチマーク
.PHONY: bench
bench:
	cargo bench -p wasm-plugin-hello
