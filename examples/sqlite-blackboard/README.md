# sqlite-blackboard

瞬時値の共有だけでなく、区間集計機能も持つ blackboard の PoC。

tmpfs 上の SQLite を共有メモリとして使い、1000Hz 書き込み・複数プロセス 30Hz 読み出しの
安定動作と、CPU 負荷・データ遅延・WAL メモリ消費を検証する。

---

## システム構成

```
/dev/shm/sqlite_poc/blackboard.db   ← tmpfs (OS 標準、マウント不要)
         │
         ├─ Writer  (1000Hz 書き込み、100ms コミット)
         ├─ Reader1 (30Hz 読み出し)
         ├─ Reader2
         └─ ...
```

WAL モード (`journal_mode=WAL`) により Writer と複数 Reader が同時アクセス可能。

---

## スキーマ

```sql
participants  -- 送信者 (GUID / name)
channels      -- チャネル (topic_name × participant)
messages      -- メッセージ履歴 (log_time ns, channel_id, data BLOB) WITHOUT ROWID
latest_states -- 最新値ブラックボード (channel_id → timestamp, data_json)
```

主要 PRAGMA: `journal_mode=WAL` / `synchronous=OFF` / `auto_vacuum=INCREMENTAL` / `mmap_size=256MB`

---

## ビルド

```bash
cargo build --release -p sqlite-blackboard
```

---

## バイナリ

### writer

```
target/release/writer [OPTIONS]

  --db          DBファイルパス       (default: /dev/shm/sqlite_poc/blackboard.db)
  --channels    チャネル数           (default: 1)
  --hz          書き込み周波数 Hz    (default: 1000)
  --data-size   1メッセージのサイズB  (default: 4096)
  --commit-ms   コミット間隔 ms      (default: 100)
  --duration    実行時間秒 (省略=無限)
```

毎秒ログ出力:
```
[writer] tps=987 (98%/1000) | commit: avg=1.234ms max=2.345ms | batch: avg=98 max=105 theory=100 | wal_kb=1234
```

### reader

```
target/release/reader [OPTIONS]

  --db          DBファイルパス                (default: /dev/shm/sqlite_poc/blackboard.db)
  --channel-id  監視チャネル ID               (default: 1)
  --topic       トピック名 (--channel-id と排他、Writer 再起動後も自動追従)
  --reader-id   ログ識別子                    (default: r0)
  --duration    実行時間秒 (省略=無限)
```

毎秒ログ出力 (1秒ウィンドウ + 最大60秒ローリング平均):
```
[r1] 30回/s | latest: avg=0.077ms max=0.241ms | count(987): avg=0.161ms max=0.420ms | stale_max=101ms errors=0 | 1min(1793): latest avg=0.080ms max=1.200ms count avg=0.165ms max=3.400ms
```

---

## ベンチマーク

### bench.sh — 基本動作確認

Writer 1 プロセス + Reader 1 プロセスを同時起動し、基本的な動作を確認する。

```bash
./bench.sh [秒数]          # デフォルト 30 秒
```

**確認内容**
- Writer/Reader が正常に起動・終了すること
- DB ファイルサイズと `/dev/shm` 使用量

---

### bench_scale.sh — リーダースケールテスト

リーダーを **1 → 5 → 10 → 20 → 40** と段階的に累積追加し、
読み出し性能がリーダー数に対してどう変化するかを計測する。

```bash
./bench_scale.sh [ステージ秒数]    # デフォルト 30 秒/ステージ
```

**ステージ遷移**
```
Stage 1 ( 1 reader )  → 30s 計測 → 統計表示
Stage 2 ( 5 readers)  → 30s 計測 → 統計表示
Stage 3 (10 readers)  → 30s 計測 → 統計表示
Stage 4 (20 readers)  → 30s 計測 → 統計表示
Stage 5 (40 readers)  → 30s 計測 → 統計表示
```

**各ステージの出力例**
```
  [stage=40] 各リーダーの最新 stats:
  ID      latest avg/max(ms)  count avg/max(ms)   stale_max     errors
  ------  ------------------  ------------------  ------------  ------
  r1         0.082 / 0.350        0.312 / 1.420     103ms         0
  ...
  全体平均 latest avg: 0.095ms (40/40 リーダー)
```

**注目指標**
| 指標             | 読み方                                                                   |
| ---------------- | ------------------------------------------------------------------------ |
| `latest avg/max` | `latest_states` 点クエリのレイテンシ。リーダー増加でほぼ変化しないはず   |
| `count avg/max`  | `SELECT count(*)` 集計クエリ。WAL 競合でリーダー増加とともに増大しやすい |
| `stale_max`      | データの鮮度。Writer のコミット間隔(100ms)を超えていないか確認           |
| `errors`         | ReadOnly 接続のクエリエラー数。0 であること                              |

詳細ログ: `/tmp/sqlite_blackboard_scale/`

---

### bench_failover.sh — Writer フェイルオーバーテスト

Writer を意図的に SIGKILL で強制終了させ、Reader への影響と再起動後の回復を確認する。

```bash
./bench_failover.sh [フェーズ秒数] [リーダー数]    # デフォルト 15秒 / 5リーダー
```

**フェーズ構成**
```
Phase 1: 通常稼働     → errors=0, stale_max が小さいことを確認
Phase 2: Writer KILL  → errors=0 のまま, stale_max が増大することを確認 (WAL の読み出しは継続)
Phase 3: Writer 再起動 → stale_max が解消し, データが再び流れることを確認
```

**Reader の期待動作**
- Writer が落ちても ReadOnly 接続はエラーにならず、`latest_states` の最後の値を返し続ける
- `stale_max` が Phase 2 で増大し、Phase 3 で `commit_ms` 付近まで戻ること
- `errors` は全フェーズを通じて 0 であること

**注目指標**
| 指標        | Phase 1   | Phase 2 | Phase 3       |
| ----------- | --------- | ------- | ------------- |
| `errors`    | 0         | 0       | 0             |
| `stale_max` | ~100ms    | 増大    | ~100ms に回復 |
| `count(*)`  | ~1000件/s | 減少→0  | 回復          |

詳細ログ: `/tmp/sqlite_blackboard_failover/`

---

### bench_writer.sh — Writer パラメータスイープ

データサイズ・書き込み周波数・コミット間隔の 3 パラメータを個別にスイープし、
スループットとコミット性能への影響を計測する。

```bash
./bench_writer.sh [ステップ秒数]    # デフォルト 15 秒/ケース
```

**Sweep A — データサイズ** (hz=1000, commit_ms=100 固定)

| 変化させるパラメータ           | 固定値                   | 注目指標                                              |
| ------------------------------ | ------------------------ | ----------------------------------------------------- |
| 128B / 256B / 512B / 1KB / 2KB | hz=1000, commit_ms=100ms | `tps`, `wal_kb` — サイズ増でI/Oがボトルネックになるか |

**Sweep B — 書き込み周波数** (size=512B, commit_ms=100 固定)

| 変化させるパラメータ     | 固定値                     | 注目指標                                     |
| ------------------------ | -------------------------- | -------------------------------------------- |
| 30 / 100 / 300 / 1000 Hz | size=512B, commit_ms=100ms | `ach%` — 高Hz ほど OS ジッターで達成率が低下 |

**Sweep C — コミット間隔** (size=512B, hz=1000 固定)

| 変化させるパラメータ  | 固定値             | 注目指標                                                |
| --------------------- | ------------------ | ------------------------------------------------------- |
| 10 / 25 / 50 / 100 ms | size=512B, hz=1000 | `c_avg` vs `commit_ms` 比率、`batch_avg` と理論値の乖離 |

**出力例**
```
════════════════════════════════════════════════════
 Sweep A: データサイズ (hz=1000, commit_ms=100ms)
════════════════════════════════════════════════════
  size(B)          tps   ach%   c_avg(ms)  c_max(ms)  batch_avg  wal_kb
  ------------  -------  -----  ----------  ----------  --------  -------
  128B             1000    100       0.234       0.456        100       12
  512B              998     99       0.278       0.512        100       23
  2048B             971     97       0.445       0.890         97       89
```

**指標の読み方**
| 指標        | 意味                                                                       |
| ----------- | -------------------------------------------------------------------------- |
| `tps`       | 実際に INSERT できた件数/秒                                                |
| `ach%`      | `tps ÷ 目標Hz` の達成率。1000Hz では 95% 前後が現実的な上限目安            |
| `c_avg/max` | BEGIN〜COMMIT の wall time。`commit_ms` より大きければ詰まりの兆候         |
| `batch_avg` | コミット1回あたりの件数。理論値 `= hz × commit_ms / 1000` からの乖離に注目 |
| `wal_kb`    | WAL サイズ。`commit_ms` が短いと細かく刻まれて小さく保たれる               |

詳細ログ: `/tmp/sqlite_blackboard_writer_bench/`

---

## bench_longrun.sh — 長時間負荷テスト

**目的**: Rust / Go / Python 各 40 プロセス（合計 120 リーダー）を最大 24 時間同時稼働させ、
長時間運用時の安定性・レイテンシ推移・エラー発生を記録する。

**スクリプト**: `bench_longrun.sh [計測秒数(default=86400)] [リーダー数/言語(default=40)] [サマリー間隔秒(default=300)]`

```bash
# 24 時間フルラン（デフォルト）
./bench_longrun.sh

# 1 時間短縮ラン（動作確認向け）
./bench_longrun.sh 3600

# 10 プロセス × 3 言語で2時間、60秒ごとにサマリー
./bench_longrun.sh 7200 10 60
```

**Writer 設定**: `--data-size 256 --hz 100 --commit-ms 50`
（24時間継続を想定し、中程度の負荷で設定）

**出力先**: `/tmp/sqlite_blackboard_longrun_<開始日時>/`

| ファイル              | 内容                                                |
| --------------------- | --------------------------------------------------- |
| `reader_r{1..40}.log` | Rust リーダーログ（1秒ごとの統計）                  |
| `reader_g{1..40}.log` | Go リーダーログ                                     |
| `reader_p{1..40}.log` | Python リーダーログ                                 |
| `writer.log`          | Writer ログ                                         |
| `summary.log`         | 定期スナップショット CSV（`SUMMARY_INTERVAL` ごと） |
| `final_report.txt`    | 最終レポート（最終統計 + エラーありプロセス一覧）   |

**summary.log フォーマット（CSV）**:
```
timestamp, elapsed_sec, lang, readers, latest_avg_ms, latest_max_ms,
count_avg_ms, count_max_ms, stale_max_ms, errors, valid_readers
```

**確認ポイント**
- `errors` が 0 のままであること（クエリエラーが発生しないこと）
- `latest_avg_ms` が経過時間とともに増大しないこと（メモリリーク・スローダウンがないこと）
- `stale_max_ms` が Writer の commit 間隔（50ms）に対して極端に大きくならないこと

---

## 計測指標 早見表

| 指標             | 出力元 | 単位 | 説明                                       |
| ---------------- | ------ | ---- | ------------------------------------------ |
| `tps`            | writer | 件/s | 実効スループット                           |
| `ach%`           | writer | %    | 目標 Hz 達成率                             |
| `c_avg/max`      | writer | ms   | コミット wall time                         |
| `batch avg/max`  | writer | 件   | コミット1回のバッチサイズ                  |
| `wal_kb`         | writer | KB   | WAL ファイルサイズ                         |
| `latest avg/max` | reader | ms   | `latest_states` 点クエリレイテンシ         |
| `count avg/max`  | reader | ms   | `SELECT count(*)` 集計クエリレイテンシ     |
| `stale_max`      | reader | ms   | データ鮮度 (現在時刻 − 最終タイムスタンプ) |
| `errors`         | reader | 件   | クエリエラー数 (0 であること)              |
| `1min(N)`        | reader | ms   | 最大60秒ローリング平均の avg/max           |

---

## テスト結果サマリー

実環境 (Linux, tmpfs `/dev/shm`) での計測結果。

### bench_scale — Rust 40 リーダー同時読み出し

Writer: 1000Hz / data=4096B / commit=100ms

|    N readers |  latest avg | latest max |   count avg | count max | errors |
| -----------: | ----------: | ---------: | ----------: | --------: | -----: |
| 1〜40 (最終) | **0.054ms** |    0.535ms | **0.104ms** |   0.763ms |  **0** |

- 40 リーダーまで増加させても `latest` レイテンシはほぼ変化しない
- `count(*)` は WAL 競合の影響でわずかに増加するが 1ms 未満を維持
- 全リーダーで `errors=0`

---

### bench_writer — Writer パラメータスイープ

**Sweep A: データサイズ** (hz=1000, commit_ms=100ms)

| size |  tps | ach% |   c_avg |   c_max | wal_kb |
| ---: | ---: | ---: | ------: | ------: | -----: |
| 128B |  999 |  99% | 0.363ms | 0.525ms |   4063 |
| 256B | 1000 | 100% | 0.500ms | 0.626ms |   4079 |
| 512B | 1000 | 100% | 0.696ms | 0.790ms |   4087 |
|  1KB |  999 |  99% | 1.168ms | 2.379ms |   4534 |
|  2KB | 1000 | 100% | 1.296ms | 1.886ms |   4522 |

- 2KB でもスループット 99〜100% を達成
- コミット時間はサイズに比例して増加。1KB 超で 1ms を超える

**Sweep B: 書き込み周波数** (size=512B, commit_ms=100ms)

|   Hz |  tps | ach% |   c_avg |
| ---: | ---: | ---: | ------: |
|   30 |   29 |  96% | 0.319ms |
|  100 |   99 |  99% | 0.356ms |
|  300 |  300 | 100% | 0.487ms |
| 1000 |  999 |  99% | 0.651ms |

- 全域で 96〜100% の達成率。tmpfs 上では 1000Hz も安定

**Sweep C: コミット間隔** (size=512B, hz=1000)

| commit_ms |  tps | ach% |   c_avg |   c_max | batch_avg |
| --------: | ---: | ---: | ------: | ------: | --------: |
|      10ms | 1000 | 100% | 0.149ms | 0.423ms |        10 |
|      25ms | 1000 | 100% | 0.248ms | 0.587ms |        25 |
|      50ms | 1000 | 100% | 0.508ms | 0.594ms |        50 |
|     100ms |  999 |  99% | 0.691ms | 0.815ms |       100 |

- バッチサイズが大きいほどコミット時間が増加するが、理論値 `hz × commit_ms/1000` と一致
- どの設定でも `c_avg < commit_ms` を満たしており詰まりなし

---

### bench_failover — Writer クラッシュ/復帰

Reader 5 プロセス稼働中に Writer を SIGKILL。

| フェーズ                | errors |               stale_max |
| ----------------------- | -----: | ----------------------: |
| Phase 1 (正常稼働)      |      0 |                   ~70ms |
| Phase 2 (Writer 停止中) |  **0** | 最大 **15102ms** (増大) |
| Phase 3 (Writer 復帰後) |  **0** |            ~76ms (回復) |

- Writer が落ちても Reader はエラーを出さず、最後の値を返し続ける
- `stale_max` だけが増大し、Writer 再起動後に `commit_ms` 付近へ回復

---

### bench_scale_poly — 多言語 20 リーダー/言語比較

Writer: 1000Hz / data=4096B、各言語 20 プロセス同時起動

| 言語   |    N | latest avg | latest max | errors |
| ------ | ---: | ---------: | ---------: | -----: |
| Rust   |   20 |    0.057ms |    0.342ms |  **0** |
| Go     |   20 |    0.035ms |    0.199ms |  **0** |
| Python |   20 |    0.031ms |    0.166ms |  **0** |

- 3 言語ともサブミリ秒を維持
- Go / Python は Rust より数値が低く見えるが、測定オーバーヘッドの差による（Rust は `tracing` フレームワークのフォーマット処理が加わる）

---

### bench_longrun — 長時間安定性テスト (1時間, 10 readers/言語)

Writer: 100Hz / data=256B / commit=50ms / retention=300s (5分)
計測時間: 3600s、スナップショット間隔: 5分

**最終スナップショット:**

| 言語   | latest avg | latest max | count avg | count max | stale_max | errors |
| ------ | ---------: | ---------: | --------: | --------: | --------: | -----: |
| Rust   |    0.038ms |    0.180ms |   0.030ms |   0.142ms |      48ms |  **0** |
| Go     |    0.036ms |    0.169ms |   0.021ms |   0.075ms |      52ms |  **0** |
| Python |    0.032ms |    0.108ms |   0.021ms |   0.076ms |      56ms |  **0** |

**レイテンシ推移（5分ごと、Rust latest_avg）:**

| 経過 | latest avg | count avg |
| ---: | ---------: | --------: |
|   5m |    0.044ms |   0.038ms |
|  10m |    0.082ms |   0.049ms |
|  15m |    0.049ms |   0.038ms |
|  30m |    0.041ms |   0.032ms |
|  45m |    0.035ms |   0.028ms |
|  60m |    0.038ms |   0.030ms |

- 1時間全期間で全言語 `errors=0`
- レイテンシに経時劣化なし（メモリリーク・スローダウンなし）
- `stale_max` は commit_ms(50ms) に近い値で安定（データ鮮度が保たれている）
- 5分ごとの DELETE + incremental_vacuum により DB サイズが上限に張り付かない

