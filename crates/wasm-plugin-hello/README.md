
## Build

```sh
# WASIなし
cargo component build -p wasm-plugin-hello --target wasm32-unknown-unknown --release
# WASI Preview2対応バイナリ
cargo component build -p wasm-plugin-hello --target wasm32-wasip2 --release
```

## Run by wasmtime

```sh
wasmtime --invoke "hello-world()" ./target/wasm32-wasip2/release/wasm_plugin_hello.wasm 
wasmtime --invoke "add(2,5)" ./target/wasm32-wasip2/release/wasm_plugin_hello.wasm
```
