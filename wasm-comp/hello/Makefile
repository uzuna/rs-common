include ../wasm.mk

.PHONY: info
info:
	$(INFO_CMD)/calc.wasm

# Rust/Wasmバイナリ比較のベンチマーク
.PHONY: bench
bench:
	cargo bench
