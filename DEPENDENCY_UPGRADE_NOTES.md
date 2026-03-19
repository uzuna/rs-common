# 依存更新メモ

最終更新日: 2026-03-18

このファイルは、直近のメンテナンスで意図的に見送った依存更新と、その理由を記録するためのものです。

## 今回反映した更新

- workspace:
  - `clap 4.5.60 -> 4.6.0`
  - `tempfile 3.26.0 -> 3.27.0`
  - `tracing-subscriber 0.3.22 -> 0.3.23`
- `crates/rhythm-core`:
  - `nix 0.30.1 -> 0.31.2`
  - `socket2 0.5 -> 0.6`
- `crates/hlac`:
  - `wide 0.7.33 -> 1.2.0`
- `crates/wgpu-shader` と `examples/vulkan-demo`:
  - `encase 0.10.0 -> 0.12.0`
  - `glam 0.29.3 -> 0.30.10`
  - `nalgebra` の feature を `convert-glam029 -> convert-glam030` に変更
- `examples/egui-learn`:
  - `encase 0.10.0 -> 0.12.0`
- `pycrates/rhythm-core-py`:
  - `pyo3 0.28.0 -> 0.28.2`

この時点での確認コマンド:

- `cargo check --workspace --exclude wasm-mls-mpm`
- `make test`

どちらも通過済みです。

## 見送った更新

### `wgpu 27.0.1 -> 28.x`

状態: 見送り

理由:

- `wgpu-shader` 自体は `wgpu 28` 向けの API 修正を入れればビルドできますが、`examples/egui-learn` は引き続き `eframe 0.33.3` / `egui 0.33.3` に依存しています。
- この依存系はまだ `wgpu 27` を引くため、依存グラフ内で `wgpu` が二重化し、次のような型不整合が発生します。
  - `wgpu::Device`
  - `wgpu::TextureFormat`
  - `eframe::wgpu` 配下のコールバック trait シグネチャ
- そのため、workspace の `wgpu` を `28` に上げると、`wgpu-shader` を直しても `examples/egui-learn` が壊れます。

次にやること:

- まず、新しい `eframe` が `wgpu 28` を採用しているか確認する。
- もし未対応なら、次のいずれかを選ぶ。
  - workspace の `wgpu` は `27.x` に据え置く
  - `egui-learn` 側で workspace の `wgpu` を直接使わず、`eframe::wgpu` に寄せる
  - `wgpu` 利用箇所を分離し、`egui-learn` だけ旧系列に残して他を先に進める

### `glam 0.30.10 -> 0.32.x`

状態: 見送り

理由:

- `crates/wgpu-shader` と `examples/vulkan-demo` は、`nalgebra = 0.34.1` の変換 feature を使っています。
- `nalgebra 0.34.1` は `convert-glam030` までは対応していますが、`glam 0.32` には対応していません。
- そのため、数値型まわりを広く見直さない限り、現状で実用上の上限は `glam 0.30.x` です。

次にやること:

- `nalgebra` に新しい `convert-glam0xx` feature が追加されていないか再確認する。
- もし未対応のままなら、`glam` 更新前に変換依存の経路を外すか置き換える。

### `bincode =1.3.3 -> 3.x`

状態: 見送り

理由:

- `crates/healed-serde/Cargo.toml` では `bincode` が `=1.3.3` で固定されています。
- `bincode 3` への移行は単なる version 更新ではなく、API の書き換えとシリアライズ形式の互換性確認が必要になる可能性があります。

次にやること:

- `crates/healed-serde` 内の `bincode` 呼び出し箇所を洗い出す。
- `1.3.3` との wire/data 互換性を維持する必要があるか確認する。
- 互換性が重要なら、依存更新前に移行テストを追加する。

## 再確認用コマンド

次回のメンテナンスで使うコマンド:

- `cargo upgrade --dry-run`
- `cargo upgrade --dry-run --incompatible --pinned`
- `cargo check --workspace --exclude wasm-mls-mpm`
- `make test`
