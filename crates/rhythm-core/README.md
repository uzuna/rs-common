# rhythm-core

u16固定のリズム生成・同期ライブラリです。  
`no_std` で動作し、整数演算のみで位相更新と同期補正を行います。

## 概要

- 内部位相（`u16`）をBPMに従って進行
- 外部位相との差分から蔵本モデル風にBPMを補正
- `RhythmMessage` で位相・BPM・時刻・累積ビート数を共有
- 他言語連携向けに固定24byteのwire形式を提供

## 主要API

- `RhythmGenerator::new(phase, base_bpm, k)`
- `RhythmGenerator::with_bpm(base_bpm, k)`
- `update(dt: core::time::Duration)`
- `sync(target_phase: u16)`
- `to_message(timestamp: Duration) -> RhythmMessage`
- `predict_phase_from_message(message, now)`
- `force_sync_from_beat_messages(older, newer, now)`
- `estimate_bpm_phase_from_beat_messages(older, newer, now)`

## 型仕様（u16固定）

- `phase`: `u16`（0..65535）
- `base_bpm`: `u16`
- `current_bpm`: `u16`
- `k`: `u16`（同期強度）

※ ジェネリクスや `num-traits` には依存しません。

## 固定wire形式（24byte, repr(C), native endian）

`RhythmMessage` は `repr(C)` で定義され、`to_wire_bytes` / `from_wire_bytes` は構造体メモリをそのままコピーします。

| Offset | Size | Type | 内容 |
|---:|---:|---|---|
| 0 | 8 | `u64` | `timestamp_ms` |
| 8 | 8 | `u64` | `beat_count` |
| 16 | 2 | `u16` | `phase` |
| 18 | 2 | `u16` | `bpm` |
| 20 | 4 | reserved | 予約領域（0埋め） |

利用API:

- `RhythmMessage::to_wire_bytes()`
- `RhythmMessage::from_wire_bytes()`
- `RhythmMessage::from_wire_slice()`

## examples

### CLI同期デモ

```bash
cargo run -p rhythm-core --example cli_sync
```

- スペースキーで外部ビート入力
- 入力が2つ以上そろうと同期
- `q` で終了

### UDPマルチキャスト

送信:

```bash
cargo run -p rhythm-core --example udp_multicast -- send
```

```bash
cargo run -p rhythm-core --example udp_multicast -- send --bpm 120 --k 24 --beat-div 4
```

受信:

```bash
cargo run -p rhythm-core --example udp_multicast -- listen
```

```bash
cargo run -p rhythm-core --example udp_multicast -- listen --bpm 110 --k 16 --stale-ms 3000
```

- 既定: `127.0.0.1:12345`（localhostのみ）
- 受信パケットは標準出力に直接表示せず、内部状態更新のみに使用
- `listen` はローカルビートを継続しつつ、受信データとの差分（位相/BPM/beat）をbeatログに表示

## テスト

```bash
cargo test -p rhythm-core
```
