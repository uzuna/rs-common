"""ServoAnomalyDetector (online streaming) のテスト。

検証項目:
  - run_step() がバッチ predict() と同一の出力を返す
  - PhysicalEstimatorOnline.step() の残差がバッチ版と近い（定常区間）
  - ServoAnomalyDetector が正常データで低い誤警報率を示す
  - ServoAnomalyDetector が外力シナリオで検知を示す
"""

import numpy as np
import pytest

from esn_anomaly.model import ESNConfig, ESNModel
from esn_anomaly.servo.command_test import (
    NOISE,
    SEED,
    WARMUP,
    _build_sc,
    _make_periodic_profile,
)
from esn_anomaly.servo.data import (
    generate_servo_with_profile,
    normalize_cmd,
    normalize_servo_obs,
)
from esn_anomaly.servo.estimator import PhysicalEstimator, PhysicalEstimatorOnline
from esn_anomaly.servo.online import ServoAnomalyDetector


TRAIN_CYCLES = 10


def _make_train_data():
    rng = np.random.default_rng(SEED)
    profile = _make_periodic_profile(TRAIN_CYCLES)
    u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)
    return u_raw, cmd_raw


def _train_esn(u_raw, cmd_raw):
    u = normalize_servo_obs(u_raw)
    cmd = normalize_cmd(cmd_raw)
    X = np.column_stack([u[:-1, 0], u[:-1, 1], cmd[:-1]])
    y = u[1:]
    config = ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED)
    model = ESNModel(config)
    model.fit(X, y)
    return model, X, y


# ------------------------------------------------------------------
# ESNModel.run_step() がバッチ predict() と同一の出力を返す
# ------------------------------------------------------------------

class TestRunStep:
    def test_run_step_equals_predict(self):
        """run_step() の逐次出力がバッチ predict() と一致する（誤差 < 1e-6）。"""
        u_raw, cmd_raw = _make_train_data()
        model, X, y = _train_esn(u_raw, cmd_raw)

        # バッチ予測（内部でリセットされる）
        batch_pred = model.predict(X)

        # 逐次予測
        model.reset()
        step_preds = np.array([model.run_step(X[i]) for i in range(len(X))])

        np.testing.assert_allclose(step_preds, batch_pred, atol=1e-6)

    def test_reset_restores_state(self):
        """reset() 後に run_step() を呼ぶと predict() と一致する。"""
        u_raw, cmd_raw = _make_train_data()
        model, X, _ = _train_esn(u_raw, cmd_raw)

        batch_pred = model.predict(X)

        # 2 回目: 別途リセットしてから逐次
        model.reset()
        step_pred_second = np.array([model.run_step(X[i]) for i in range(len(X))])

        np.testing.assert_allclose(step_pred_second, batch_pred, atol=1e-6)

    def test_run_step_unfitted_raises(self):
        """fit() 前に run_step() を呼ぶと RuntimeError を送出する。"""
        model = ESNModel()
        with pytest.raises(RuntimeError, match="未学習"):
            model.run_step(np.zeros(3))


# ------------------------------------------------------------------
# PhysicalEstimatorOnline.step() の残差がバッチ版と近い
# ------------------------------------------------------------------

class TestPhysicalEstimatorOnline:
    def _setup(self):
        u_raw, cmd_raw = _make_train_data()
        est_batch = PhysicalEstimator(mode="ff", smooth_win=20, vel_smooth_win=50)
        thr = est_batch.fit_threshold(u_raw[:, 0], u_raw[:, 1], cmd_raw, warmup=WARMUP)
        est_online = PhysicalEstimatorOnline(threshold=thr, mode="ff", smooth_win=20, vel_smooth_win=50)
        return u_raw, cmd_raw, est_batch, est_online

    def test_online_residual_close_to_batch_at_steady(self):
        """定常区間での online 残差がバッチ版と近い（< 10 mA の平均誤差）。"""
        u_raw, cmd_raw, est_batch, est_online = self._setup()

        batch_r = est_batch.abs_residual(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        steady_mask = est_batch._steady_mask(u_raw[:, 0], u_raw[:, 1], cmd_raw)

        online_rs = []
        for i in range(len(u_raw)):
            r, _, _ = est_online.step(u_raw[i, 0], u_raw[i, 1], cmd_raw[i])
            online_rs.append(r)
        online_r = np.array(online_rs)

        # 定常区間のみで比較（WARMUP 以降）
        valid = steady_mask & (np.arange(len(u_raw)) >= WARMUP)
        if valid.sum() > 0:
            mean_diff = np.mean(np.abs(online_r[valid] - batch_r[valid]))
            assert mean_diff < 10.0, f"定常区間の平均残差差 {mean_diff:.2f} mA > 10 mA"

    def test_online_reset_clears_state(self):
        """reset() 後の step() が初期状態から開始される。"""
        u_raw, cmd_raw, _, est_online = self._setup()

        # 一度実行
        for i in range(10):
            est_online.step(u_raw[i, 0], u_raw[i, 1], cmd_raw[i])

        # リセット後に同じデータで再実行 → 同じ結果になるはず
        est_online.reset()
        r1, _, _ = est_online.step(u_raw[0, 0], u_raw[0, 1], cmd_raw[0])

        est_online.reset()
        r2, _, _ = est_online.step(u_raw[0, 0], u_raw[0, 1], cmd_raw[0])

        assert r1 == r2


# ------------------------------------------------------------------
# ServoAnomalyDetector の統合テスト
# ------------------------------------------------------------------

class TestServoAnomalyDetector:
    def _build_detector(self):
        u_raw_train, cmd_raw_train = _make_train_data()
        det = ServoAnomalyDetector.from_training_data(
            u_raw_train,
            cmd_raw_train,
            esn_config=ESNConfig(units=200, sr=0.9, lr=0.3, ridge=1e-6, warmup=WARMUP, seed=SEED),
            smooth_win=30,
            phys_smooth_win=20,
            vel_smooth_win=50,
            vel_threshold=5.0,
            sat_fraction=0.90,
            warmup=WARMUP,
        )
        return det

    def test_no_false_alarm_on_normal_data(self):
        """正常データで ESN・物理の誤警報率がともに 20% 未満。"""
        det = self._build_detector()

        rng = np.random.default_rng(SEED + 1)
        profile = _make_periodic_profile(5)
        u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)

        results = []
        for i in range(len(u_raw)):
            r = det.step(cmd_raw[i], u_raw[i, 0], u_raw[i, 1])
            results.append(r)

        esn_flags = np.array([r["esn_anomaly"] for r in results[WARMUP:]])
        phys_flags = np.array([r["phys_anomaly"] for r in results[WARMUP:]])

        esn_rate = esn_flags.mean() * 100
        phys_rate = phys_flags.mean() * 100
        assert esn_rate < 20.0, f"ESN 誤警報率 {esn_rate:.1f}% >= 20%"
        assert phys_rate < 20.0, f"物理誤警報率 {phys_rate:.1f}% >= 20%"

    def test_detects_external_force(self):
        """SC シナリオ（定常外力 ext_torque=0.3）で物理推定器が外乱区間を検知する。"""
        det = self._build_detector()

        rng = np.random.default_rng(SEED + 100)
        u_raw, cmd_raw, disturbances = _build_sc(rng)

        results = []
        for i in range(len(u_raw)):
            r = det.step(cmd_raw[i], u_raw[i, 0], u_raw[i, 1])
            results.append(r)

        phys_flags = np.array([r["phys_anomaly"] for r in results])

        # 外乱区間内の検知率
        n = len(phys_flags)
        rates = []
        for d in disturbances:
            lo = max(WARMUP, d.start)
            hi = min(n, d.end)
            if lo < hi:
                rates.append(phys_flags[lo:hi].mean() * 100)

        if rates:
            mean_rate = np.mean(rates)
            assert mean_rate > 30.0, f"外乱区間の物理検知率 {mean_rate:.1f}% <= 30%"

    def test_step_returns_expected_keys(self):
        """step() の返り値が期待するキーをすべて含む。"""
        det = self._build_detector()
        u_raw, cmd_raw = _make_train_data()

        result = det.step(cmd_raw[0], u_raw[0, 0], u_raw[0, 1])

        expected_keys = {
            "cmd_mrad", "pos_mrad", "load_mA",
            "esn_res_pos", "esn_res_load", "esn_anomaly",
            "phys_residual", "phys_is_steady", "phys_anomaly",
        }
        assert expected_keys <= set(result.keys())
