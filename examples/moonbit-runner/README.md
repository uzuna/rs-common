# moonbit-runner

MoonBit 製 Wasm プラグインを Rust ホスト（Wasmtime）から実行し、
WIT 経路と線形メモリ直接書き込み経路（raw）を比較計測するサンプルです。

通常起動は HTTP サーバーモードのみを提供します。

## HTTP サーバー起動

```bash
make -C examples/moonbit-runner run
```

主なエンドポイント:
- `GET /status`（ホスト状態）
- `GET /api/status`（Wasm `get-status`）
- `POST /api/update`（Wasm `update`）

`/api` は `--prefix` で変更できます。

## プラグイン管理 API

| エンドポイント                    | 説明                               |
| --------------------------------- | ---------------------------------- |
| `GET /plugin/status`              | 現在のプラグイン状態を確認         |
| `POST /plugin/reload`             | .wasm バイナリを POST してリロード |
| `GET /plugin/versions`            | 保存済みバージョン一覧             |
| `POST /plugin/rollback/{version}` | 指定バージョンへロールバック       |

```bash
# プラグイン状態確認
make -C examples/moonbit-runner plugin-status

# バイナリをポストしてホットリロード
make -C examples/moonbit-runner reload-api

# バージョン一覧確認
make -C examples/moonbit-runner plugin-versions

# バージョン 2 にロールバック
make -C examples/moonbit-runner rollback-api V=2
```

## ホットリロードトリガー

- **ファイル変更監視**: `--wasm` に指定したファイルが更新されると自動リロード
- **SIGUSR1**: `kill -USR1 <pid>` で任意タイミングにリロード
- **SIGINT / SIGTERM**: グレースフルシャットダウン

## 所感

- add_loop の比較では `native < raw < WIT` の順でオーバーヘッドが大きくなる
- `add_loop1` は native 約 1.10ns / raw 約 22.7ns / WIT 約 328ns
- `add_loop2000` は計算支配になり、native 約 241ns に対して raw 約 816ns（約 3.4x）、WIT 約 1.13us（約 4.7x）
- 計算コストはnative比で3倍程度の時間がかかる可能性がある
- SharedObject (so) の hello は WIT と同程度（約 225ns）、add_loop1 は JSON 変換コストで約 1.70us（WIT の約 5.2x）

## ベンチマーク実行方法

### WIT 経路のみ（criterion）

```bash
make -C examples/moonbit-runner bench
```

### WIT + raw 経路（criterion）

```bash
make -C examples/moonbit-runner bench-raw
```

### native / WASM / SharedObject 比較（criterion）

```bash
make -C examples/moonbit-runner bench-compare
```

### 短時間で再現する例

```bash
make -C examples/moonbit-runner bench-raw BENCH_ARGS='--sample-size=10 --measurement-time=1'
```

## 対応付プロファイル用ビルド

`add_raw/loop2000-16B` の perf 結果で `wasm[0]::function[54]` のような関数 index を追跡しやすくするため、
name section を含む debug core Wasm を使うターゲットを追加しています。

### 対応付用 core Wasm を生成

```bash
make -C examples/moonbit-runner build-plugin-raw-symbolized
```

生成物:
- `examples/moonbit-runner/plugins/control.core.symbolized.wasm`

### 対応付用ビルドで perf 収集

```bash
make -C examples/moonbit-runner perf-add-loop2000-symbolized PERF_DATA=/tmp/moonbit-runner-add-loop2000.symbolized.perf.data
perf report -i /tmp/moonbit-runner-add-loop2000.symbolized.perf.data --stdio --no-children --sort overhead,comm,dso,symbol
```

必要に応じて profiler 形式を変更:

```bash
make -C examples/moonbit-runner perf-add-loop2000-symbolized WASMTIME_PROFILER=jitdump
```

### raw Wasm の明示指定（任意）

benchmark 側は `MOONBIT_RUNNER_RAW_WASM_PATH` で raw Wasm パスを上書きできます。

```bash
MOONBIT_RUNNER_RAW_BENCH=1 MOONBIT_RUNNER_RAW_WASM_PATH=examples/moonbit-runner/plugins/control.core.symbolized.wasm \
	cargo bench -p moonbit-runner --bench criterion_bench -- moonbit-runner/raw/add_raw/loop2000-16B
```

## メモリリークチェック

ホットリロードを繰り返した際のメモリ増加を確認します。

```bash
# 100 回リロード（許容: 100 MB 以内）
make -C examples/moonbit-runner memory-check

# 1000 回リロード（ストレステスト）
cargo test -p moonbit-runner reload_memory_check_1000times -- --ignored --nocapture
```

## 今回のベンチマーク結果（criterion）

計測コマンド:

```bash
MOONBIT_RUNNER_SO_BENCH=1 MOONBIT_RUNNER_RAW_BENCH=1 cargo bench -p moonbit-runner --bench criterion_bench -- --sample-size=20 --measurement-time=3
```

前提:
- profile: `bench`（`cargo bench`）
- raw benchmark は `MOONBIT_RUNNER_RAW_BENCH=1` で有効化
- native benchmark（`moonbit-runner/native/*`）は常時有効
- compare benchmark は `MOONBIT_RUNNER_SO_BENCH=1` で SharedObject も含む
- `benchmark_raw_*` は payload 長検証 + 先頭/末尾 byte への軽いアクセス（最小処理）
- `add_raw` は 16B 転送（a, b, loop_count, result）で result を返却

### WIT 経路

| ケース           | time (概算中央値) |  Throughput |
| ---------------- | ----------------: | ----------: |
| `update`         |           8.14 µs |           - |
| `add_loop1`      |          328.2 ns |           - |
| `add_loop2000`   |          1.130 µs |           - |
| `benchmark 128B` |          1.335 µs | 91.45 MiB/s |
| `benchmark 1KB`  |          6.722 µs | 145.3 MiB/s |
| `benchmark 4KB`  |          25.16 µs | 155.2 MiB/s |

### raw 線形メモリ経路

| ケース                        | time (概算中央値) |  Throughput |
| ----------------------------- | ----------------: | ----------: |
| `add_raw loop1-16B`           |          22.70 ns | 672.1 MiB/s |
| `add_raw loop2000-16B`        |          815.7 ns |  18.7 MiB/s |
| `benchmark_raw 128B`          |          22.08 ns | 5.399 GiB/s |
| `benchmark_raw 1KB`           |          37.38 ns | 25.52 GiB/s |
| `benchmark_raw 4KB`           |          69.46 ns | 54.92 GiB/s |
| `add_raw_breakdown write16B`  |           3.98 ns | 3.747 GiB/s |
| `add_raw_breakdown read16B`   |           2.32 ns | 6.419 GiB/s |
| `add_raw_breakdown call-only` |          17.13 ns |           - |
| `add_raw_breakdown roundtrip` |          22.42 ns |           - |

### native (Rust) 経路

| ケース         | time (概算中央値) |
| -------------- | ----------------: |
| `add_loop1`    |          1.097 ns |
| `add_loop2000` |          241.5 ns |

### native / WASM / SharedObject 呼び出しコスト比較

| ケース      |   native |     WASM |       SO |
| ----------- | -------: | -------: | -------: |
| `hello`     | 210.8 ps | 358.8 ns | 225.4 ns |
| `add_loop1` | 1.116 ns | 326.1 ns | 1.696 µs |

> SO の add はホスト→SO→ホストの JSON シリアライズ・デシリアライズを含むため WASM より遅い。
> WASM の hello/add は canonical ABI 経由の直接呼び出しのため SO より高速。

### add_loop 比較（WIT / raw / native）

| ケース      |      WIT |      raw |   native | WIT / native | raw / native |
| ----------- | -------: | -------: | -------: | -----------: | -----------: |
| loop1 (16B) | 328.2 ns | 22.70 ns | 1.097 ns |    約 299.2x |     約 20.7x |
| loop2000    | 1.130 µs | 815.7 ns | 241.5 ns |      約 4.7x |      約 3.4x |

### WIT 比の高速化率（参考）

| サイズ          |      WIT |      raw | 速度比 (WIT / raw) |
| --------------- | -------: | -------: | -----------------: |
| add_loop1 (16B) | 328.2 ns | 22.70 ns |           約 14.5x |
| 128B            | 1.335 µs | 22.08 ns |           約 60.5x |
| 1KB             | 6.722 µs | 37.38 ns |          約 179.8x |
| 4KB             | 25.16 µs | 69.46 ns |          約 362.3x |

## 結果の読み方

- `benchmark_*` と `benchmark_raw_*` の差分は主に「データ受け渡し経路の差」を反映します。
- `add_loop1` と `add_raw loop1-16B` は、16B の最小転送を伴う軽量計算で ABI 経路差を確認するケースです。
- `add_loop2000` は計算量が支配的なケースで、転送オーバーヘッドの比率が下がることを確認できます。
- raw 側は canonical ABI の lower/lift や post-return free を通さず、固定領域への直接 read/write です。
- `compare` ベンチは WASM / SO / native の呼び出し単体コストを同一インターフェース（`PluginHandle`）で比較します。

## データサイズ別の推奨データ渡し方法

### 128B クラス（小サイズ・制御系メタデータ中心）

推奨: **WIT の型付き Input/Output を優先**

理由:
- 型安全で保守しやすい
- バージョン管理・互換性管理がしやすい
- 本計測でも 1.34µs 程度で、30Hz 制御（33.3ms 周期）に対して十分小さい

### 1KB クラス（中サイズ・センサー塊データ）

推奨: **基本は WIT、呼び出し頻度が高い場合はハイブリッド**

理由:
- 単一プラグイン / 低頻度なら WIT で十分
- 高頻度ループ、多プラグイン同時実行、厳しいレイテンシ制約では raw 併用が有効

推奨構成（ハイブリッド）:
- 制御コマンド・ステータス: WIT
- バルク payload: 線形メモリ（ptr/len）

### 4KB 以上（大きめ payload / ストリーミング寄り）

推奨: **線形メモリ直接書き込みを第一候補**

理由:
- 受け渡しオーバーヘッド差が顕著（WIT の約 362x 高速）
- コピー回数・アロケーションの影響が無視しにくい

設計上の注意:
- メモリレイアウト規約（input/output 領域、所有権、ライフタイム）を明文化する
- 破壊的変更を避けるため、WIT は control plane、raw は data plane として責務分離する

## 実運用での推奨パターン

1. デフォルトは WIT（可読性・保守性を優先）
2. ボトルネックが確認できた経路のみ raw に切り替え
3. raw 化後も API 入口は WIT で残し、切り替え可能な構造にする

この方針により、性能と保守性のバランスを段階的に最適化できます。
