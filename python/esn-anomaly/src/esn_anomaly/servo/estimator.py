"""物理モデルベースの外力・クーロン摩擦推定器（Phase 14）。

観測値 (pos_mrad, load_mA, cmd_mrad) から外力トルクを推定する。

推定原理（定常状態での力の釣り合い）:
    T_motor + T_restore(θ) + T_ext = 0  （定常、摩擦・ダンピング≈0）
    I_cmd × motor_gain + T_restore(θ_actual) + T_ext = 0
    → I_ext_residual = I_cmd - I_ff(θ_actual) = -T_ext / motor_gain

定常状態の識別（速度マスク + 飽和マスク）:
    - 速度推定は大きな窓幅（vel_smooth_win）で平滑化し、
      位置ノイズ起因の偽速度信号を抑制する。
    - |ω_est| ≥ vel_threshold のとき移動中と判定。
    - |I_cmd| ≥ sat_fraction × I_max のとき飽和中と判定。
    - コマンド変化直後もガードバンドで除外。

モード:
  "ff"   — residual = smooth(load_mA - I_ff(pos_mrad))
             定常状態での外力検出に優れる。
             信号: -T_ext / motor_gain（ext_torque=0.3 → −30 mA）
  "full" — residual = smooth(load_mA - I_ff(pos_mrad) - I_pd_est)
             I_pd_est = Kp*(cmd_deg - pos_deg) - Kd*ω_est
             I_pd 寄与を除去。外乱シグナルは位置誤差に比例する弱い信号になるが、
             誤警報が更に少ない。
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

from esn_anomaly.detector import compute_threshold, smooth
from esn_anomaly.servo.data import (
    ControllerParams,
    DEG_TO_MRAD,
    ServoModel,
    ServoParams,
    _mrad_to_deg,
)


class PhysicalEstimator:
    """物理モデルベースの外力推定・検出器。

    Args:
        mode: "ff" または "full"（詳細はモジュール docstring 参照）
        smooth_win: 残差に適用する因果的移動平均窓幅（例: 20）
        vel_smooth_win: 速度推定に使用する平滑化窓幅（例: 50）。
            pos_noise ≈ 2 mrad / dt=0.01 s → raw velocity noise std ≈ 11.5 deg/s。
            vel_smooth_win = 50 で std ≈ 1.6 deg/s に低減。
        vel_threshold: 速度マスクの閾値 [deg/s]（例: 5.0）
        sat_fraction: 飽和マスクの割合（例: 0.90）
        servo_params: サーボ物理パラメータ（None なら既定値）
        ctrl_params: 制御ゲイン（"full" モードのみ使用）

    Usage::

        est = PhysicalEstimator(mode="ff")
        thr = est.fit_threshold(pos_train, load_train, cmd_train)
        detected = est.detect(pos_test, load_test, cmd_test)
    """

    def __init__(
        self,
        mode: str = "ff",
        smooth_win: int = 20,
        vel_smooth_win: int = 50,
        vel_threshold: float = 5.0,
        sat_fraction: float = 0.90,
        servo_params: ServoParams | None = None,
        ctrl_params: ControllerParams | None = None,
    ) -> None:
        if mode not in ("ff", "full"):
            raise ValueError(f"mode は 'ff' か 'full' のいずれかです: {mode!r}")
        self.mode = mode
        self.smooth_win = smooth_win
        self.vel_smooth_win = vel_smooth_win
        self.vel_threshold = vel_threshold
        self.sat_fraction = sat_fraction
        self._servo = ServoModel(servo_params or ServoParams())
        self._cp = ctrl_params or ControllerParams()
        self._threshold: float | None = None

    # ------------------------------------------------------------------
    # 内部計算
    # ------------------------------------------------------------------

    def _i_ff(self, pos_mrad: NDArray[np.float64]) -> NDArray[np.float64]:
        """実際の角度を保持するのに必要な前向き補償電流 [mA]。

        I_ff(θ) = -T_restore(θ) / motor_gain
        """
        return np.array([
            -self._servo.restoring_torque(_mrad_to_deg(p)) / self._servo.motor_gain
            for p in pos_mrad
        ], dtype=np.float64)

    def _estimate_omega(self, pos_mrad: NDArray[np.float64]) -> NDArray[np.float64]:
        """位置差分から角速度 [deg/s] を推定する。

        速度専用の大きな窓幅（vel_smooth_win）でノイズを抑制する。
        位置ノイズ 2 mrad / 0.01 s = 200 mrad/s = 11.5 deg/s を
        vel_smooth_win=50 で ≈ 1.6 deg/s に低減。
        """
        dt = self._servo.p.dt
        diff_deg = np.diff(pos_mrad / DEG_TO_MRAD, prepend=pos_mrad[0] / DEG_TO_MRAD) / dt
        return smooth(diff_deg, self.vel_smooth_win)

    def _steady_mask(
        self,
        pos_mrad: NDArray[np.float64],
        load_mA: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64] | None = None,
    ) -> NDArray[np.bool_]:
        """定常状態（整定済み）の時刻を True とするマスク。

        以下のすべてが成立する区間を「定常」とする:
          1. 速度が低い  |ω_est| < vel_threshold
          2. モーター非飽和  |I_cmd| < sat_fraction × I_max
          3. コマンド変化から vel_smooth_win 以上経過

        各条件に対してガードバンド（前後 vel_smooth_win ステップ）を付与し、
        速度推定の遅延とスムージング尾引きを補正する。
        """
        omega = self._estimate_omega(pos_mrad)
        i_max = self._servo.p.current_max
        sat_limit = self.sat_fraction * i_max

        moving = (np.abs(omega) >= self.vel_threshold) | (np.abs(load_mA) >= sat_limit)

        if cmd_mrad is not None:
            cmd_changed = np.diff(cmd_mrad, prepend=cmd_mrad[0]) != 0.0
            moving = moving | cmd_changed

        # ガードバンド: 前後 vel_smooth_win ステップを除外
        kernel = np.ones(2 * self.vel_smooth_win + 1, dtype=np.float64)
        moving_float = np.convolve(moving.astype(np.float64), kernel, mode="same")
        return moving_float == 0.0

    def _i_pd_est(
        self,
        pos_mrad: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64],
    ) -> NDArray[np.float64]:
        """PD 制御器が出力するはずの電流 [mA] の推定値。

        I_pd_est = Kp * (cmd_deg - pos_deg) - Kd * ω_est
        """
        pos_deg = pos_mrad / DEG_TO_MRAD
        cmd_deg = cmd_mrad / DEG_TO_MRAD
        omega_est = self._estimate_omega(pos_mrad)
        return self._cp.kp * (cmd_deg - pos_deg) - self._cp.kd * omega_est

    # ------------------------------------------------------------------
    # 残差計算
    # ------------------------------------------------------------------

    def residual(
        self,
        pos_mrad: NDArray[np.float64],
        load_mA: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64] | None = None,
    ) -> NDArray[np.float64]:
        """スムージング済み残差を返す。

        Args:
            pos_mrad: 位置観測値 [mrad] shape (n,)
            load_mA:  負荷観測値 [mA]  shape (n,)
            cmd_mrad: コマンド位置 [mrad] shape (n,)（"full" モードで必要）

        Returns:
            shape (n,) の残差配列
        """
        i_ff = self._i_ff(pos_mrad)
        raw = load_mA - i_ff

        if self.mode == "full":
            if cmd_mrad is None:
                raise ValueError("full モードには cmd_mrad が必要です")
            raw = raw - self._i_pd_est(pos_mrad, cmd_mrad)

        return smooth(raw, self.smooth_win)

    def abs_residual(
        self,
        pos_mrad: NDArray[np.float64],
        load_mA: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64] | None = None,
    ) -> NDArray[np.float64]:
        """絶対値残差を返す（閾値との比較用）。"""
        return np.abs(self.residual(pos_mrad, load_mA, cmd_mrad))

    # ------------------------------------------------------------------
    # 閾値学習・検出
    # ------------------------------------------------------------------

    def fit_threshold(
        self,
        pos_mrad: NDArray[np.float64],
        load_mA: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64] | None = None,
        method: str = "3sigma",
        warmup: int = 100,
    ) -> float:
        """正常データから閾値を学習して返す。

        速度・飽和・コマンド変化マスクで真の定常状態を識別し、
        そこでの残差統計から閾値を計算する。

        Args:
            pos_mrad: 訓練用位置観測値 [mrad]
            load_mA:  訓練用負荷観測値 [mA]
            cmd_mrad: 訓練用コマンド [mrad]
            method:   compute_threshold に渡す方式（"3sigma" 等）
            warmup:   ウォームアップ区間（先頭 warmup ステップは評価外）

        Returns:
            推定閾値 [mA]
        """
        r = self.abs_residual(pos_mrad, load_mA, cmd_mrad)
        mask = self._steady_mask(pos_mrad, load_mA, cmd_mrad).copy()
        mask[:warmup] = False

        r_steady = r[mask]
        if len(r_steady) < 10:
            r_steady = r[warmup:]

        self._threshold = float(compute_threshold(r_steady, method))
        return self._threshold

    def detect(
        self,
        pos_mrad: NDArray[np.float64],
        load_mA: NDArray[np.float64],
        cmd_mrad: NDArray[np.float64] | None = None,
        threshold: float | None = None,
    ) -> NDArray[np.bool_]:
        """異常フラグ配列を返す（True = 定常状態かつ閾値超過）。

        Args:
            pos_mrad, load_mA, cmd_mrad: テストデータ
            threshold: 閾値（None なら fit_threshold で学習した値を使用）

        Returns:
            shape (n,) の bool 配列
        """
        thr = threshold if threshold is not None else self._threshold
        if thr is None:
            raise RuntimeError("先に fit_threshold を呼んでください")
        r = self.abs_residual(pos_mrad, load_mA, cmd_mrad)
        steady = self._steady_mask(pos_mrad, load_mA, cmd_mrad)
        return steady & (r > thr)
