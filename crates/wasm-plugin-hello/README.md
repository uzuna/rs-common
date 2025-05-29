
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


## Benchmark

| ns                      | Rust   | Wasm   |
| ----------------------- | ------ | ------ |
| Add                     | 0.209  | 41.99  |
| ListSum                 | 31.211 | 309.96 |
| ListSum within Resource | -      | 236.96 |
| LoopSum                 | 0.209  | 45.947 |
| String(32)              | 769.24 | 1960.3 |
| String(32) return only  | -      | 149.34 |

1. WASM呼び出しのオーバーヘッドはおそらく40ns
2. メモリアクセスコストがそれなりにある。
   1. メモリ転送コストが60ns?
   2. 集計、アクセスコストがRust比で7倍？
3. 文字列操作はそもそもが遅いので、Rustとは大きな差なく使えそう
   1. 文字列を使う場合は事前に生成してそれを返すだけにしておくのが良さそう
