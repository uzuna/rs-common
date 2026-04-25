"""実機向けサーボ異常検知ストリーミングインタフェース（Phase 15）。

ブラックボックス（指示値→(pos, load) のストリーム）に対して
1ステップずつリアルタイムで異常検知を行う。

使い方（訓練データから直接構築）::

    from esn_anomaly.servo.online import ServoAnomalyDetector

    # 訓練データから検知器を構築
    det = ServoAnomalyDetector.from_training_data(u_raw_train, cmd_raw_train)

    # 実機ストリームで逐次処理
    for cmd, pos, load in hardware_stream:
        result = det.step(cmd, pos, load)
        if result["esn_anomaly"] or result["phys_anomaly"]:
            print("異常検知:", result)

result dict のキー:
    cmd_mrad:       コマンド [mrad]
    pos_mrad:       位置観測値 [mrad]
    load_mA:        負荷観測値 [mA]
    esn_res_pos:    ESN 位置残差（正規化・スムージング済み）
    esn_res_load:   ESN 負荷残差（正規化・スムージング済み）
    esn_anomaly:    ESN 異常フラグ (bool)
    phys_residual:  物理推定器残差 [mA]
    phys_is_steady: 定常状態フラグ (bool)
    phys_anomaly:   物理推定器異常フラグ (bool)
"""

from __future__ import annotations

from collections import deque

import numpy as np
from numpy.typing import NDArray

from esn_anomaly.detector import compute_threshold, smooth
from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.servo.data import (
    LOAD_MA_MAX,
    LOAD_MA_MIN,
    POS_MRAD_MAX,
    POS_MRAD_MIN,
    normalize_cmd,
    normalize_servo_obs,
)
from esn_anomaly.servo.estimator import PhysicalEstimator, PhysicalEstimatorOnline


class ServoAnomalyDetector:
    """実機向けストリーミング異常検知器。

    3ch ESN（pos, load, cmd）と物理推定器（ff モード）を組み合わせて
    1ステップずつ異常を検知する。

    Args:
        esn_model: 学習済み ESNModel
        thr_pos: ESN 位置残差の閾値（正規化スムージング済み残差に対して）
        thr_load: ESN 負荷残差の閾値（同上）
        smooth_win: ESN 残差スムージング窓幅
        est_ff: 物理推定器 ff モード（None なら物理検知を行わない）
        est_full: 物理推定器 full モード（None なら使用しない）
    """

    def __init__(
        self,
        esn_model: ESNModel,
        thr_pos: float,
        thr_load: float,
        smooth_win: int = 30,
        est_ff: PhysicalEstimatorOnline | None = None,
        est_full: PhysicalEstimatorOnline | None = None,
    ) -> None:
        self._esn = esn_model
        self._thr_pos = thr_pos
        self._thr_load = thr_load
        self._smooth_win = smooth_win
        self._est_ff = est_ff
        self._est_full = est_full

        # 正規化定数（固定: data.py の定数と一致）
        self._pos_center = (POS_MRAD_MAX + POS_MRAD_MIN) / 2.0
        self._pos_scale = (POS_MRAD_MAX - POS_MRAD_MIN) / 2.0
        self._load_center = (LOAD_MA_MAX + LOAD_MA_MIN) / 2.0
        self._load_scale = (LOAD_MA_MAX - LOAD_MA_MIN) / 2.0

        # ESN 残差スムージング用 ring buffer
        self._esn_pos_buf: deque[float] = deque(maxlen=smooth_win)
        self._esn_load_buf: deque[float] = deque(maxlen=smooth_win)

        # 前ステップの ESN 予測（pos_norm, load_norm）
        self._prev_pred: NDArray[np.float64] | None = None

    # ------------------------------------------------------------------
    # 正規化ヘルパー
    # ------------------------------------------------------------------

    def _norm_pos(self, pos_mrad: float) -> float:
        return (pos_mrad - self._pos_center) / self._pos_scale

    def _norm_load(self, load_mA: float) -> float:
        return (load_mA - self._load_center) / self._load_scale

    def _norm_cmd(self, cmd_mrad: float) -> float:
        return (cmd_mrad - self._pos_center) / self._pos_scale

    # ------------------------------------------------------------------
    # 公開インタフェース
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
    ) -> "ServoAnomalyDetector":
        """正常訓練データから検知器を構築する。

        Args:
            u_raw_train:    shape (n, 2) の観測データ [pos_mrad, load_mA]
            cmd_raw_train:  shape (n,) のコマンドデータ [mrad]
            esn_config:     ESN ハイパーパラメータ（None なら既定値）
            smooth_win:     ESN 残差スムージング窓幅
            phys_smooth_win: 物理推定器残差スムージング窓幅
            vel_smooth_win: 物理推定器の速度推定窓幅
            vel_threshold:  定常状態判定速度閾値 [deg/s]
            sat_fraction:   飽和判定割合
            warmup:         ESN ウォームアップ区間長

        Returns:
            学習済み ServoAnomalyDetector
        """
        config = esn_config or ESNConfig(
            units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=warmup
        )

        # ESN 訓練
        u_norm = normalize_servo_obs(u_raw_train)
        cmd_norm = normalize_cmd(cmd_raw_train)
        X = np.column_stack([u_norm[:-1, 0], u_norm[:-1, 1], cmd_norm[:-1]])
        y = u_norm[1:]
        esn = ESNModel(config)
        esn.fit(X, y)

        # ESN 閾値計算（訓練残差の 3σ）
        pred_tr = esn.predict(X)
        e_pos = np.abs(y - pred_tr)[:, 0]
        e_load = np.abs(y - pred_tr)[:, 1]
        s_pos = smooth(e_pos, smooth_win)
        s_load = smooth(e_load, smooth_win)
        thr_pos = float(compute_threshold(s_pos[warmup:], "3sigma"))
        thr_load = float(compute_threshold(s_load[warmup:], "3sigma"))

        # 物理推定器閾値計算
        est_ff_batch = PhysicalEstimator(
            mode="ff",
            smooth_win=phys_smooth_win,
            vel_smooth_win=vel_smooth_win,
            vel_threshold=vel_threshold,
            sat_fraction=sat_fraction,
        )
        thr_ff = est_ff_batch.fit_threshold(
            u_raw_train[:, 0], u_raw_train[:, 1], cmd_raw_train, warmup=warmup
        )

        est_full_batch = PhysicalEstimator(
            mode="full",
            smooth_win=phys_smooth_win,
            vel_smooth_win=vel_smooth_win,
            vel_threshold=vel_threshold,
            sat_fraction=sat_fraction,
        )
        thr_full = est_full_batch.fit_threshold(
            u_raw_train[:, 0], u_raw_train[:, 1], cmd_raw_train, warmup=warmup
        )

        # online 推定器を構築
        est_ff = PhysicalEstimatorOnline(
            threshold=thr_ff,
            mode="ff",
            smooth_win=phys_smooth_win,
            vel_smooth_win=vel_smooth_win,
            vel_threshold=vel_threshold,
            sat_fraction=sat_fraction,
        )
        est_full = PhysicalEstimatorOnline(
            threshold=thr_full,
            mode="full",
            smooth_win=phys_smooth_win,
            vel_smooth_win=vel_smooth_win,
            vel_threshold=vel_threshold,
            sat_fraction=sat_fraction,
        )

        return cls(
            esn_model=esn,
            thr_pos=thr_pos,
            thr_load=thr_load,
            smooth_win=smooth_win,
            est_ff=est_ff,
            est_full=est_full,
        )

    def warmup(
        self,
        u_raw: NDArray[np.float64],
        cmd_raw: NDArray[np.float64],
    ) -> None:
        """ストリーミング開始前にリザーバを安定化する（オプション）。

        Args:
            u_raw:   shape (m, 2) の観測データ [pos_mrad, load_mA]
            cmd_raw: shape (m,) のコマンドデータ [mrad]
        """
        u_norm = normalize_servo_obs(u_raw)
        cmd_norm = normalize_cmd(cmd_raw)
        warmup_X = np.column_stack([u_norm[:, 0], u_norm[:, 1], cmd_norm])
        self._esn.reset(warmup_data=warmup_X)
        self._esn_pos_buf.clear()
        self._esn_load_buf.clear()
        self._prev_pred = None

    def step(
        self,
        cmd_mrad: float,
        pos_mrad: float,
        load_mA: float,
    ) -> dict:
        """1ステップ処理。

        Args:
            cmd_mrad: コマンド位置 [mrad]
            pos_mrad: 位置観測値 [mrad]
            load_mA:  負荷観測値 [mA]

        Returns:
            検知結果 dict（キーは本モジュールの docstring 参照）
        """
        pos_norm = self._norm_pos(pos_mrad)
        load_norm = self._norm_load(load_mA)
        cmd_norm = self._norm_cmd(cmd_mrad)

        # ESN: 前ステップの予測 vs 今ステップの実測
        x = np.array([pos_norm, load_norm, cmd_norm], dtype=np.float64)

        if self._prev_pred is not None:
            # 前ステップで予測した (pos_{t}, load_{t}) と実測 (pos_t, load_t) を比較
            esn_err_pos = abs(float(self._prev_pred[0]) - pos_norm)
            esn_err_load = abs(float(self._prev_pred[1]) - load_norm)
        else:
            esn_err_pos = 0.0
            esn_err_load = 0.0

        pred = self._esn.run_step(x)  # shape (2,): t+1 の予測
        self._prev_pred = pred

        self._esn_pos_buf.append(esn_err_pos)
        self._esn_load_buf.append(esn_err_load)

        esn_res_pos = float(np.mean(self._esn_pos_buf))
        esn_res_load = float(np.mean(self._esn_load_buf))
        esn_anomaly = (esn_res_pos > self._thr_pos) or (esn_res_load > self._thr_load)

        # 物理推定器 ff
        phys_residual = float("nan")
        phys_is_steady = False
        phys_anomaly = False
        if self._est_ff is not None:
            phys_residual, phys_is_steady, phys_anomaly = self._est_ff.step(
                pos_mrad, load_mA, cmd_mrad
            )

        return {
            "cmd_mrad": cmd_mrad,
            "pos_mrad": pos_mrad,
            "load_mA": load_mA,
            "esn_res_pos": esn_res_pos,
            "esn_res_load": esn_res_load,
            "esn_anomaly": esn_anomaly,
            "phys_residual": phys_residual,
            "phys_is_steady": phys_is_steady,
            "phys_anomaly": phys_anomaly,
        }

    def reset(self) -> None:
        """内部状態を全リセット。"""
        self._esn.reset()
        self._esn_pos_buf.clear()
        self._esn_load_buf.clear()
        self._prev_pred = None
        if self._est_ff is not None:
            self._est_ff.reset()
        if self._est_full is not None:
            self._est_full.reset()
