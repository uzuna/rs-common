# stream-deck

Elgato Stream Deck+ の LCD ストリップに、複数のメトリクスや通知状態を並べて表示するツールです。

この crate は、表示対象を固定の 4 分割ロジックで持つのではなく、`Plugin` と `Section` を分離した構造で実装されています。これにより、同じ種類の表示を履歴長だけ変えて複数並べる、といった構成を設定ファイルだけで切り替えられます。

## アーキテクチャ概要

役割は大きく 4 層です。

1. `MetricsSource` などの共有データソース
2. `DataPlugin` を実装した各プラグイン
3. プラグインと履歴バッファを束ねる `Section`
4. `SectionSpec` の配列を描画する `Renderer`

```text
DashboardConfig
    ↓
build_sections()
    ↓
Section { plugin, history, capacity }
    ↓ tick()
DataPlugin::update()
    ↓
Section::as_spec()
    ↓
Renderer::render(&[SectionSpec])
```

## 各コンポーネントの責務

### `MetricsSource`

CPU、メモリ、ロードアベレージの元データを共有する層です。

- `sysinfo::System` を内部に持つ
- 毎ティック 1 回だけ `refresh()` を実行する
- 各プラグインは `snapshot()` から最新値を読む

この構造により、CPU 用・MEM 用・LOAD 用の各プラグインが個別に OS 情報を更新し直すことを避けています。

### `DataPlugin`

表示データを収集するための境界トレイトです。

```rust
pub trait DataPlugin {
    fn update(&mut self);
    fn latest_normalized(&self) -> f32;
    fn value_text(&self) -> String;
    fn label(&self) -> &'static str;
}
```

各メソッドの役割は次の通りです。

- `update()`: 1 ティック分の最新値を取り込む
- `latest_normalized()`: グラフ描画用の 0.0..=1.0 の値を返す
- `value_text()`: LCD 中央に出す文字列を返す
- `label()`: セクション左上のラベルを返す

現在の実装には以下のプラグインがあります。

- `CpuPlugin`
- `MemPlugin`
- `LoadPlugin`
- `BrightnessPlugin`
- `NotifPlugin`

### `Section`

LCD 上の 1 セクションに相当する表示単位です。

- `Box<dyn DataPlugin>` を 1 つ保持する
- 正規化済み履歴を `VecDeque<f32>` で保持する
- `capacity` で履歴長を制御する

重要なのは、履歴長が `Section` 側の責務になっている点です。これにより、同じ `CpuPlugin` を使っていても、別の `Section` に載せれば独立した履歴を持てます。

```text
Section::new(Box::new(CpuPlugin::new(...)), 10)
Section::new(Box::new(CpuPlugin::new(...)), 30)
```

この 2 つは同じ CPU データを参照しつつ、表示履歴だけが異なる別インスタンスとして扱われます。

### `Renderer`

描画層です。

- 入力は `&[SectionSpec]`
- セクション数は固定ではなく可変
- 各セクション幅は `800 / n` で動的計算
- 各セクションのバー本数は `history.len()` に従う

そのため、4 セクション固定ではなく、設定次第で 1 個でも 5 個でも描画できます。

## 更新フロー

1 ティックごとの流れは次の通りです。

1. `MetricsSource::refresh()` で共有メトリクスを更新する
2. 各 `Section` の `tick()` を呼ぶ
3. `Section::tick()` の中で `plugin.update()` を呼ぶ
4. `latest_normalized()` の値を履歴バッファへ push する
5. 全 `Section` を `SectionSpec` に変換する
6. `Renderer::render()` で LCD 画像を生成する

通知や輝度のように外部入力で更新される値は、プラグインが共有状態を読むだけで扱えます。`BrightnessPlugin` と `NotifPlugin` の `update()` が実質 no-op なのはこのためです。

## モジュール構成

- `src/main.rs`
  - 起動処理、設定読み込み、メインループ、`build_sections()`
- `src/metrics.rs`
  - `MetricsSource` とメトリクス整形処理
- `src/plugin.rs`
  - `DataPlugin` と各プラグイン実装
- `src/section.rs`
  - `Section` と履歴バッファ管理
- `src/renderer.rs`
  - LCD 描画
- `src/notifications.rs`
  - 通知受信、通知スロット状態、ボタン反映用ロジック

## 設定ファイル

レイアウトは `[[layout.sections]]` の配列として定義します。

- `type`: セクション種別
- `capacity`: 履歴バー本数

`capacity` を省略した場合は、コード上のデフォルト値が使われます。

```toml
[[layout.sections]]
type = "cpu"
capacity = 10

[[layout.sections]]
type = "cpu"
capacity = 30

[[layout.sections]]
type = "mem"

[[layout.sections]]
type = "notif"
```

上の例では、CPU を短期履歴と長期履歴で 2 回表示しています。これは Plugin と Section を分離した構造であるため成立しています。

指定できる `type` は以下です。

- `cpu`
- `mem`
- `load`
- `bright`
- `notif`

## 新しい表示項目を追加する手順

新しい表示を追加したい場合は、基本的に次の 3 点を実装します。

1. `plugin.rs` に `DataPlugin` 実装を追加する
2. `main.rs` の `DashboardSectionKind` と `build_sections()` に分岐を追加する
3. 必要なら共有データソースや外部状態を追加する

履歴の保持や描画幅の計算は `Section` と `Renderer` が受け持つため、新しい表示項目ごとに履歴管理や描画レイアウトを作り直す必要はありません。

## 設計上の利点

- セクション数を固定しない
- 履歴長をセクションごとに変えられる
- 同一プラグイン型を複数回使える
- データ収集と描画責務が分離される
- 通知や輝度のような外部イベント駆動の値も同じ枠組みに載せられる

この構造により、今後セクション種別が増えても、既存の描画パイプラインや履歴管理を大きく崩さずに拡張できます。
