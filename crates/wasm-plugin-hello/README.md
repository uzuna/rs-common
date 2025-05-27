
## Build

```sh
cargo component build --target wasm32-wasip2 --release
```

## Run by wasmtime

```sh
wasmtime --invoke "hello-world()" ./target/wasm32-wasip2/release/wasm_plugin_hello.wasm 
wasmtime --invoke "add(2,5)" ./target/wasm32-wasip2/release/wasm_plugin_hello.wasm
```
