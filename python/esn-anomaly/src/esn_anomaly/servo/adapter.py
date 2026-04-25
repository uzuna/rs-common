"""実機計測アダプタ（Phase 15）。

コマンド出力ストリームと計測入力ストリームを分離し、
1 ステップずつリアルタイムで異常検知を行うためのインタフェース。

## 全体設計

```
   [コマンド生成]              [実機]             [計測取得]
CommandScheduler ──cmd──▶ サーボドライバ ──▶ シリアル/ネット
       │                                          │
       ▼                                          ▼
adapter.update_cmd(cmd)           adapter.on_measurement(ts, pos, load)
                                         │
                                         ▼
                                    StepResult
                                    (ESN + 物理 検知結果)
```

コマンドと計測は**非同期**に扱われる:
  - `update_cmd()` はコマンド変更時に随時呼び出す
  - `on_measurement()` は計測周期ごとに呼び出す（直前の最新コマンドを参照）

## 典型的な使い方

```python
from esn_anomaly.servo.adapter import ServoStreamAdapter, CommandScheduler
from esn_anomaly.servo.data import CommandProfile, CommandSegment

# 1. 訓練データから検知器を構築
adapter = ServoStreamAdapter.from_training_data(u_raw_train, cmd_raw_train)

# 2. コマンドスケジューラを準備
profile = CommandProfile([
    CommandSegment("hold",  0.0, 500),
    CommandSegment("ramp", 500.0, 300),
    CommandSegment("hold", 500.0, 500),
])
scheduler = CommandScheduler.from_profile(profile)

# 3. 計測ループ（例: 100 Hz 定周期）
for cmd in scheduler:
    adapter.update_cmd(cmd)           # コマンドを送信するタイミングで呼ぶ
    # ... ここで実機に cmd を送信 ...

    ts, pos, load = read_hardware()   # 計測値をパース済みで取得
    result = adapter.on_measurement(ts, pos, load)

    if result.esn_anomaly or result.phys_anomaly:
        handle_alert(result)

# 4. 結果を CSV に保存
adapter.save_csv("measurement.csv")
```
"""

from __future__ import annotations

import csv
from collections import deque
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Iterator

import numpy as np
from numpy.typing import NDArray

from esn_anomaly.model import ESNConfig
from esn_anomaly.servo.data import CommandProfile
from esn_anomaly.servo.online import ServoAnomalyDetector


# ------------------------------------------------------------------
# StepResult: 1 ステップの計測・検知結果
# ------------------------------------------------------------------

@dataclass
class StepResult:
    """1 ステップの計測値と異常検知結果。

    Attributes:
        ts:           タイムスタンプ [s]（計測側から受け取る）
        cmd_mrad:     そのステップで有効だったコマンド位置 [mrad]
        pos_mrad:     位置観測値 [mrad]
        load_mA:      負荷観測値 [mA]
        esn_res_pos:  ESN 位置残差（正規化・スムージング済み）
        esn_res_load: ESN 負荷残差（正規化・スムージング済み）
        esn_anomaly:  ESN 異常フラグ
        phys_residual: 物理推定器残差 [mA]
        phys_is_steady: 定常状態フラグ（False の間は物理判定無効）
        phys_anomaly:  物理推定器異常フラグ（定常かつ閾値超過）
    """

    ts: float
    cmd_mrad: float
    pos_mrad: float
    load_mA: float
    esn_res_pos: float
    esn_res_load: float
    esn_anomaly: bool
    phys_residual: float
    phys_is_steady: bool
    phys_anomaly: bool

    @classmethod
    def csv_header(cls) -> list[str]:
        return [
            "ts", "cmd_mrad", "pos_mrad", "load_mA",
            "esn_res_pos", "esn_res_load", "esn_anomaly",
            "phys_residual", "phys_is_steady", "phys_anomaly",
        ]

    def to_row(self) -> list:
        return [
            self.ts, self.cmd_mrad, self.pos_mrad, self.load_mA,
            self.esn_res_pos, self.esn_res_load, int(self.esn_anomaly),
            self.phys_residual, int(self.phys_is_steady), int(self.phys_anomaly),
        ]


# ------------------------------------------------------------------
# ServoStreamAdapter: コマンド/計測を分離したアダプタ
# ------------------------------------------------------------------

class ServoStreamAdapter:
    """実機向けストリーミング計測アダプタ。

    コマンド出力と計測入力を分離し、非同期に扱えるようにする。

    - ``update_cmd(cmd_mrad)`` : コマンド変更時に随時呼び出す
    - ``on_measurement(ts, pos_mrad, load_mA)`` : 計測周期ごとに呼び出す

    Args:
        detector: 学習済み ServoAnomalyDetector
        history_maxlen: 履歴の最大保持数（None で無制限）
        initial_cmd: 初期コマンド値 [mrad]（デフォルト 0.0）
    """

    def __init__(
        self,
        detector: ServoAnomalyDetector,
        history_maxlen: int | None = None,
        initial_cmd: float = 0.0,
    ) -> None:
        self._detector = detector
        self._current_cmd: float = initial_cmd
        self._history: deque[StepResult] = deque(maxlen=history_maxlen)

    # ------------------------------------------------------------------
    # コマンド更新
    # ------------------------------------------------------------------

    def update_cmd(self, cmd_mrad: float) -> None:
        """コマンドを更新する。

        実機にコマンドを送信するタイミングで呼び出す。
        次の ``on_measurement()`` 呼び出しからこのコマンドが適用される。

        Args:
            cmd_mrad: コマンド位置 [mrad]
        """
        self._current_cmd = cmd_mrad

    # ------------------------------------------------------------------
    # 計測入力
    # ------------------------------------------------------------------

    def on_measurement(
        self,
        ts: float,
        pos_mrad: float,
        load_mA: float,
    ) -> StepResult:
        """計測値を 1 ステップ入力して検知結果を返す。

        計測周期（例: 100 Hz = 10 ms ごと）に呼び出す。
        現在保持しているコマンドと計測値を組み合わせて ESN・物理推定を行う。

        Args:
            ts:       タイムスタンプ [s]（計測側の時刻、任意基準）
            pos_mrad: パース済み位置観測値 [mrad]
            load_mA:  パース済み負荷観測値 [mA]

        Returns:
            StepResult（ESN・物理の両検知結果を含む）
        """
        raw = self._detector.step(self._current_cmd, pos_mrad, load_mA)
        result = StepResult(
            ts=ts,
            cmd_mrad=self._current_cmd,
            pos_mrad=pos_mrad,
            load_mA=load_mA,
            esn_res_pos=raw["esn_res_pos"],
            esn_res_load=raw["esn_res_load"],
            esn_anomaly=raw["esn_anomaly"],
            phys_residual=raw["phys_residual"],
            phys_is_steady=raw["phys_is_steady"],
            phys_anomaly=raw["phys_anomaly"],
        )
        self._history.append(result)
        return result

    # ------------------------------------------------------------------
    # ウォームアップ
    # ------------------------------------------------------------------

    def warmup(
        self,
        u_raw: NDArray[np.float64],
        cmd_raw: NDArray[np.float64],
        base_ts: float = 0.0,
    ) -> None:
        """ストリーミング開始前にリザーバを安定化する（オプション）。

        Args:
            u_raw:   shape (m, 2) の観測データ [pos_mrad, load_mA]
            cmd_raw: shape (m,) のコマンドデータ [mrad]
            base_ts: ウォームアップ区間の開始タイムスタンプ [s]
        """
        self._detector.warmup(u_raw, cmd_raw)

    # ------------------------------------------------------------------
    # 状態リセット
    # ------------------------------------------------------------------

    def reset(self, clear_history: bool = False) -> None:
        """内部状態をリセットする。

        Args:
            clear_history: True のとき履歴も消去する（デフォルト False）
        """
        self._detector.reset()
        if clear_history:
            self._history.clear()

    # ------------------------------------------------------------------
    # 履歴アクセス
    # ------------------------------------------------------------------

    @property
    def history(self) -> list[StepResult]:
        """蓄積された StepResult のリストを返す。"""
        return list(self._history)

    def pop_history(self) -> list[StepResult]:
        """蓄積された結果を取り出してクリアする（バッチ転送用）。"""
        results = list(self._history)
        self._history.clear()
        return results

    # ------------------------------------------------------------------
    # CSV 保存
    # ------------------------------------------------------------------

    def save_csv(self, path: str | Path) -> None:
        """蓄積した StepResult を CSV ファイルに保存する。

        Args:
            path: 保存先パス
        """
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w", newline="") as f:
            writer = csv.writer(f)
            writer.writerow(StepResult.csv_header())
            for r in self._history:
                writer.writerow(r.to_row())

    # ------------------------------------------------------------------
    # ファクトリ
    # ------------------------------------------------------------------

    @classmethod
    def from_training_data(
        cls,
        u_raw_train: NDArray[np.float64],
        cmd_raw_train: NDArray[np.float64],
        esn_config: ESNConfig | None = None,
        smooth_win: int = 30,
        phys_smooth_win: int = 20,
        vel_smooth_win: int = 50,
        vel_threshold: float = 5.0,
        sat_fraction: float = 0.90,
        warmup: int = 100,
        history_maxlen: int | None = None,
        initial_cmd: float = 0.0,
    ) -> "ServoStreamAdapter":
        """正常訓練データからアダプタを構築する。

        ``ServoAnomalyDetector.from_training_data()`` のラッパー。
        引数は同一。

        Args:
            u_raw_train:    shape (n, 2) の観測データ [pos_mrad, load_mA]
            cmd_raw_train:  shape (n,) のコマンドデータ [mrad]
            その他引数は ServoAnomalyDetector.from_training_data() と同じ
            history_maxlen: 履歴の最大保持数（None で無制限）
            initial_cmd:    初期コマンド値 [mrad]

        Returns:
            構築済み ServoStreamAdapter
        """
        detector = ServoAnomalyDetector.from_training_data(
            u_raw_train,
            cmd_raw_train,
            esn_config=esn_config,
            smooth_win=smooth_win,
            phys_smooth_win=phys_smooth_win,
            vel_smooth_win=vel_smooth_win,
            vel_threshold=vel_threshold,
            sat_fraction=sat_fraction,
            warmup=warmup,
        )
        return cls(detector, history_maxlen=history_maxlen, initial_cmd=initial_cmd)

    @classmethod
    def from_csv(
        cls,
        csv_path: str | Path,
        detector: ServoAnomalyDetector,
        ts_col: str = "ts",
        pos_col: str = "pos_mrad",
        load_col: str = "load_mA",
        cmd_col: str = "cmd_mrad",
    ) -> tuple["ServoStreamAdapter", list[StepResult]]:
        """保存済み CSV を再生して StepResult のリストを返す。

        訓練後に収集データを事後解析するための便利メソッド。

        Args:
            csv_path:  CSV ファイルパス（タイムスタンプ・計測列を含む）
            detector:  使用する ServoAnomalyDetector（学習済み）
            ts_col, pos_col, load_col, cmd_col: 列名

        Returns:
            (adapter, results) のタプル
        """
        import pandas as pd  # optional dependency
        df = pd.read_csv(csv_path)
        adapter = cls(detector)
        results = []
        for _, row in df.iterrows():
            adapter.update_cmd(float(row[cmd_col]))
            result = adapter.on_measurement(
                ts=float(row[ts_col]),
                pos_mrad=float(row[pos_col]),
                load_mA=float(row[load_col]),
            )
            results.append(result)
        return adapter, results


# ------------------------------------------------------------------
# CommandScheduler: コマンドプロファイルをステップ単位で提供
# ------------------------------------------------------------------

class CommandScheduler:
    """コマンドプロファイルをステップ単位で提供するイテレータ。

    ``CommandProfile`` から目標位置配列を展開し、
    1 ステップずつ cmd_mrad を返す。

    Usage::

        profile = CommandProfile([
            CommandSegment("hold",  0.0,   500),
            CommandSegment("ramp", 500.0,  300),
            CommandSegment("hold", 500.0,  200),
        ])
        scheduler = CommandScheduler.from_profile(profile)

        for cmd in scheduler:
            adapter.update_cmd(cmd)
            ts, pos, load = read_hardware()
            result = adapter.on_measurement(ts, pos, load)

    Args:
        cmds: コマンド位置配列 [mrad]  shape (n,)
    """

    def __init__(self, cmds: NDArray[np.float64]) -> None:
        self._cmds = cmds
        self._idx: int = 0

    @classmethod
    def from_profile(cls, profile: CommandProfile) -> "CommandScheduler":
        """CommandProfile からスケジューラを構築する。"""
        return cls(profile.build_target_array())

    @classmethod
    def from_array(cls, cmds: NDArray[np.float64]) -> "CommandScheduler":
        """コマンド配列から直接スケジューラを構築する。"""
        return cls(cmds.copy())

    # ------------------------------------------------------------------
    # イテレータプロトコル
    # ------------------------------------------------------------------

    def __iter__(self) -> Iterator[float]:
        return self

    def __next__(self) -> float:
        if self._idx >= len(self._cmds):
            raise StopIteration
        cmd = float(self._cmds[self._idx])
        self._idx += 1
        return cmd

    def __len__(self) -> int:
        return len(self._cmds)

    # ------------------------------------------------------------------
    # 状態
    # ------------------------------------------------------------------

    @property
    def current_index(self) -> int:
        """現在のインデックス（次に返すステップ番号）。"""
        return self._idx

    @property
    def remaining(self) -> int:
        """残りステップ数。"""
        return max(0, len(self._cmds) - self._idx)

    @property
    def done(self) -> bool:
        """全ステップを消費した場合 True。"""
        return self._idx >= len(self._cmds)

    def reset(self) -> None:
        """先頭に巻き戻す。"""
        self._idx = 0

    def peek(self) -> float | None:
        """次のコマンドを消費せずに返す（末尾なら None）。"""
        if self._idx < len(self._cmds):
            return float(self._cmds[self._idx])
        return None
