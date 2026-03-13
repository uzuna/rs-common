# rhythm-core-py

`rhythm-core` Rust クレート向けの Python バインディングです。
ビルドには `pyo3` と `maturin` を使います。

## このプロジェクトで使う名前

- Rust パッケージ名: `rhythm-core-py`
- Python パッケージ/インポート名: `py_rhythm_core`
- PyO3 モジュール初期化シンボル: `py_rhythm_core`

## 前提環境

- Rust ツールチェーン (`cargo`, `rustc`)
- Python 3.7 以上
- `pip`

推奨（任意）:

```bash
python -m venv .venv
source .venv/bin/activate
python -m pip install -U pip
```

## ビルドとインストール

### 1. 開発用インストール（推奨）

拡張モジュールをビルドし、現在の Python 環境へインストールします。

リポジトリルートで実行する場合:

```bash
python -m pip install maturin
python -m maturin develop -m pycrates/rhythm-core-py/Cargo.toml
```

`pycrates/rhythm-core-py` ディレクトリで実行する場合:

```bash
cd pycrates/rhythm-core-py
python -m pip install maturin
python -m maturin develop
```

### 2. wheel をビルドする

リポジトリルートで実行:

```bash
python -m pip install maturin
python -m maturin build -m pycrates/rhythm-core-py/Cargo.toml --release
```

生成された wheel は `target/wheels/` に出力されます。

### 3. ビルド済み wheel をインストールする

リポジトリルートで実行:

```bash
python -m pip install target/wheels/py_rhythm_core-*.whl
```

## Rust 側のみビルド確認

Rust 側のビルド確認だけ行う場合:

```bash
cargo check -p rhythm-core-py
```

## スモークテスト

インストール後、import と BPM ヘルパーが動くことを確認します。

```bash
python -c "import py_rhythm_core as m; raw=m.bpm_q8_from_int(120); print(m.BPM_Q8_ONE, raw, m.bpm_q8_to_float(raw))"
```

## `examples/udp_multicast.py` の説明

`examples/udp_multicast.py` は、`rhythm-core` の UDP マルチキャスト同期サンプルを Python から試すためのスクリプトです。

動作概要:

- `listener` モードはメッセージを受信し、`sync()` で位相と BPM を追従します。
- `sender` モードはリズムメッセージを定期送信します。
- `listener` の標準出力は、受信のたびではなく「自身の位相が 0 をまたいだタイミング（1拍ごと）」で `[beat]` 行を表示します。

このスクリプト内の主な初期値:

- マルチキャスト先: `239.0.0.1:12345`（`rhythm-core` の Rust 例と同じ）
- 初期 BPM: `120`
- BPM 制限: `BpmLimitParam(60, 120)`
- 結合係数: `coupling_divisor=12`

指定可能なオプション:

- `--group`: マルチキャストグループ（例: `239.0.0.1`）
- `--port`: ポート（例: `12345`）
- `--bpm`: 初期 BPM
- `--k`: 結合係数（coupling divisor）

実行手順（2ターミナル）:

1. 事前に `maturin develop` まで完了して `py_rhythm_core` をインストールします。
2. ターミナル A で `listener` を起動します。
3. ターミナル B で `sender` を起動します。

リポジトリルートでの実行例:

```bash
python pycrates/rhythm-core-py/examples/udp_multicast.py listener
python pycrates/rhythm-core-py/examples/udp_multicast.py sender
```

`crates/rhythm-core/examples/udp_multicast.rs` から受信する場合（互換実行例）:

```bash
# ターミナル A (Python listener)
python pycrates/rhythm-core-py/examples/udp_multicast.py listener --group 239.0.0.1 --port 12345 --bpm 120 --k 12

# ターミナル B (Rust sender)
cargo run --example udp_multicast -F std -- send --port 12345 --bpm 120 --k 12
```

停止は `Ctrl+C` です。

補足:

- 実行環境のネットワーク設定やファイアウォールによってはマルチキャスト受信が失敗する場合があります。
- ローカル検証では同一マシン上で `listener` と `sender` を別ターミナルで実行できます。

## Python テスト

リポジトリルートで実行:

```bash
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s pycrates/rhythm-core-py/tests -v
```
