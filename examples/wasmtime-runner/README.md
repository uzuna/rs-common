
## run

```sh
# wasiなし
cargo run -p wasmtime-runner -- --wasi none -n target/wasm32-unknown-unknown/release/wasm_plugin_hello.wasm
# wasi preview2
cargo run -p wasmtime-runner -- --wasi preview2 -n target/wasm32-wasip2/release/wasm_plugin_hello.wasm
```
