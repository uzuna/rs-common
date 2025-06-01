
## run

```sh
# wasiなし
cargo run -p wasmtime-runner -- --wasi none -n target/wasm32-unknown-unknown/release/wasm_plugin_hello.wasm
# wasi preview2
cargo run -p wasmtime-runner -- --wasi preview2 -n target/wasm32-wasip2/release/wasm_plugin_hello.wasm
```

## wit-front

バイナリの定義は `wasm-comp/hasdep/wit/world.wit` にあるが、この内exportされている部分だけを使うために切り出した。
ディレクトリ名を`wit`にしないのは rust-analyzer checkに `cargo component check` を使った場合に`binding.rs`を生成してしまう。
これはバイナリ向けでwasmtimeの利用向けではないので生成
