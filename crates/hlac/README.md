
## Reference

- testdata: https://www.erca.go.jp/yobou/zensoku/sukoyaka/58/pazzle/

## 可視化画像出力（Phase3.1）

- `crates/hlac/tests/phase31_puzzle_test.rs` の可視化テストは、
	`ENABLE_OUTPUT_IMAGE=1` を指定したときだけ画像を書き出す。
- 出力先は `crates/hlac/testdata/phase31_output`。
- 未指定または `1` 以外では、画像は保存せずに検証のみ行う。
