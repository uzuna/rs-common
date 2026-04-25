"""ServoModel の物理特性テスト。

検証項目:
  - 電流 0 で安定平衡点 (-45 度) に収束する
  - 最大電流（負方向）で -80 度付近に静止する
  - 最大電流（正方向）で +20 度付近に静止する
  - 機械的限界 (-90/+90 度) を超えない
  - データ生成関数の出力 shape と値域が正しい
  - ServoController による位置追従・負荷特性
  - generate_servo_periodic の shape・正規化・ノイズ
"""

import numpy as np
import pytest

from esn_anomaly.servo.data import (
    DEG_TO_MRAD,
    ControllerParams,
    MeasurementNoise,
    MotionPattern,
    ServoController,
    ServoModel,
    ServoParams,
    _deg_to_mrad,
    _mrad_to_deg,
    generate_servo_periodic,
    generate_servo_step,
    generate_servo_train,
    generate_servo_test_scenario,
    normalize_servo_obs,
)


PARAMS = ServoParams()


# ------------------------------------------------------------------
# ばね定数と平衡点
# ------------------------------------------------------------------

class TestSpringConstants:
    def test_k_neg_formula(self):
        """K_neg = 1/(theta_eq - theta_reach_neg) = 1/35"""
        model = ServoModel(PARAMS)
        expected = 1.0 / (PARAMS.theta_eq - PARAMS.theta_reach_neg)
        assert model.k_neg == pytest.approx(expected)

    def test_k_pos_formula(self):
        """K_pos = 1/(theta_reach_pos - theta_eq) = 1/65"""
        model = ServoModel(PARAMS)
        expected = 1.0 / (PARAMS.theta_reach_pos - PARAMS.theta_eq)
        assert model.k_pos == pytest.approx(expected)

    def test_restoring_torque_at_eq_is_zero(self):
        """安定平衡点での復元トルクが 0"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq) == pytest.approx(0.0)

    def test_restoring_torque_sign_above_eq(self):
        """平衡点より正側: 負方向トルク（引き戻し）"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq + 10) < 0

    def test_restoring_torque_sign_below_eq(self):
        """平衡点より負側: 正方向トルク（引き戻し）"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq - 10) > 0

    def test_stall_torque_neg_equals_pos(self):
        """負方向・正方向の最大復元トルクが等しい（対称モータ条件）"""
        model = ServoModel(PARAMS)
        t_neg = abs(model.restoring_torque(PARAMS.theta_reach_neg))
        t_pos = abs(model.restoring_torque(PARAMS.theta_reach_pos))
        assert t_neg == pytest.approx(t_pos, rel=1e-6)


# ------------------------------------------------------------------
# 静的平衡収束
# ------------------------------------------------------------------

class TestEquilibrium:
    def _run_until_settled(self, current: float, n_steps: int = 5000) -> float:
        model = ServoModel(PARAMS)
        for _ in range(n_steps):
            theta, _ = model.step(current)
        return theta

    def test_zero_current_converges_to_eq(self):
        """電流 0 → -45 度に収束（誤差 ±1 度以内）"""
        theta = self._run_until_settled(0.0)
        assert theta == pytest.approx(PARAMS.theta_eq, abs=1.0)

    def test_max_neg_current_converges_near_reach_neg(self):
        """最大負電流 → -80 度付近に収束（誤差 ±3 度以内）"""
        theta = self._run_until_settled(-PARAMS.current_max)
        assert theta == pytest.approx(PARAMS.theta_reach_neg, abs=3.0)

    def test_max_pos_current_converges_near_reach_pos(self):
        """最大正電流 → +20 度付近に収束（誤差 ±3 度以内）"""
        theta = self._run_until_settled(PARAMS.current_max)
        assert theta == pytest.approx(PARAMS.theta_reach_pos, abs=3.0)

    def test_from_positive_side_free_converges(self):
        """正側初期値から電流 0 で -45 度に収束"""
        model = ServoModel(PARAMS)
        model.reset(theta=20.0)
        for _ in range(5000):
            theta, _ = model.step(0.0)
        assert theta == pytest.approx(PARAMS.theta_eq, abs=1.0)


# ------------------------------------------------------------------
# 機械的限界
# ------------------------------------------------------------------

class TestMechanicalLimits:
    def test_never_exceeds_theta_max(self):
        """最大正電流を長時間与えても +90 度を超えない"""
        model = ServoModel(PARAMS)
        for _ in range(10000):
            theta, _ = model.step(PARAMS.current_max)
        assert theta <= PARAMS.theta_max

    def test_never_below_theta_min(self):
        """最大負電流を長時間与えても -90 度を下回らない"""
        model = ServoModel(PARAMS)
        for _ in range(10000):
            theta, _ = model.step(-PARAMS.current_max)
        assert theta >= PARAMS.theta_min

    def test_overcurrent_is_clipped(self):
        """200 mA 入力は 100 mA に相当する挙動（限界内に留まる）"""
        model = ServoModel(PARAMS)
        for _ in range(5000):
            theta, _ = model.step(200.0)
        assert theta <= PARAMS.theta_max


# ------------------------------------------------------------------
# reset
# ------------------------------------------------------------------

class TestReset:
    def test_reset_to_eq(self):
        model = ServoModel(PARAMS)
        model.step(100.0)
        model.reset()
        assert model.theta == pytest.approx(PARAMS.theta_eq)
        assert model.omega == pytest.approx(0.0)

    def test_reset_to_custom(self):
        model = ServoModel(PARAMS)
        model.reset(theta=10.0, omega=5.0)
        assert model.theta == pytest.approx(10.0)
        assert model.omega == pytest.approx(5.0)


# ------------------------------------------------------------------
# データ生成関数
# ------------------------------------------------------------------

class TestGenerateServoStep:
    def test_output_shape(self):
        u, y = generate_servo_step(n_steps=500, rng=np.random.default_rng(0))
        assert u.shape == (500, 1)
        assert y.shape == (500, 1)

    def test_u_range(self):
        u, _ = generate_servo_step(n_steps=1000, rng=np.random.default_rng(1))
        assert u.min() >= -1.0 - 1e-9
        assert u.max() <= 1.0 + 1e-9

    def test_y_range(self):
        _, y = generate_servo_step(n_steps=1000, rng=np.random.default_rng(2))
        assert y.min() >= -1.0 - 1e-9
        assert y.max() <= 1.0 + 1e-9


class TestGenerateServoTrain:
    def test_output_shape(self):
        X, y = generate_servo_train(n_steps=500, rng=np.random.default_rng(0))
        assert X.shape == (500, 2)
        assert y.shape == (500, 1)

    def test_x_columns_in_range(self):
        X, _ = generate_servo_train(n_steps=500, rng=np.random.default_rng(3))
        assert X[:, 0].min() >= -1.0 - 1e-9  # 電流
        assert X[:, 0].max() <= 1.0 + 1e-9
        assert X[:, 1].min() >= -1.0 - 1e-9  # 前ステップ角度
        assert X[:, 1].max() <= 1.0 + 1e-9


class TestGenerateServoTestScenario:
    @pytest.mark.parametrize("scenario", ["step", "sine", "max_neg", "max_pos", "free"])
    def test_output_keys_and_lengths(self, scenario):
        result = generate_servo_test_scenario(scenario=scenario, n_steps=200)
        for key in ("t", "theta", "omega", "current"):
            assert key in result
            assert len(result[key]) == 200

    def test_max_neg_theta_near_reach_neg(self):
        """最大負電流シナリオ: 最終角度が reach_neg 付近"""
        result = generate_servo_test_scenario(scenario="max_neg", n_steps=2000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_reach_neg, abs=3.0)

    def test_max_pos_theta_near_reach_pos(self):
        """最大正電流シナリオ: 最終角度が reach_pos 付近"""
        result = generate_servo_test_scenario(scenario="max_pos", n_steps=2000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_reach_pos, abs=3.0)

    def test_free_converges_to_eq(self):
        """電流 0 の free シナリオ: 最終角度が平衡点付近"""
        result = generate_servo_test_scenario(scenario="free", n_steps=3000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_eq, abs=1.0)

    def test_theta_always_within_limits(self):
        """全シナリオで角度が機械的限界内"""
        for scenario in ("step", "sine", "max_neg", "max_pos", "free"):
            result = generate_servo_test_scenario(scenario=scenario, n_steps=500)
            assert np.all(result["theta"] >= PARAMS.theta_min)
            assert np.all(result["theta"] <= PARAMS.theta_max)

    def test_unknown_scenario_raises(self):
        with pytest.raises(ValueError, match="未知のシナリオ"):
            generate_servo_test_scenario(scenario="invalid")


# ------------------------------------------------------------------
# 単位変換
# ------------------------------------------------------------------

class TestUnitConversion:
    def test_deg_to_mrad_45(self):
        assert _deg_to_mrad(45.0) == pytest.approx(45.0 * DEG_TO_MRAD)

    def test_mrad_to_deg_roundtrip(self):
        for deg in [-90.0, -45.0, 0.0, 20.0, 90.0]:
            assert _mrad_to_deg(_deg_to_mrad(deg)) == pytest.approx(deg, rel=1e-9)

    def test_equilibrium_in_mrad(self):
        """安定平衡点 -45° ≈ -785.4 mrad"""
        assert _deg_to_mrad(-45.0) == pytest.approx(-785.4, abs=0.1)

    def test_reach_pos_in_mrad(self):
        """最大正到達 +20° ≈ +349.1 mrad"""
        assert _deg_to_mrad(20.0) == pytest.approx(349.1, abs=0.1)

    def test_reach_neg_in_mrad(self):
        """最大負到達 -80° ≈ -1396.3 mrad"""
        assert _deg_to_mrad(-80.0) == pytest.approx(-1396.3, abs=0.1)


# ------------------------------------------------------------------
# ServoController
# ------------------------------------------------------------------

class TestServoController:
    def _run_until_settled(self, target_mrad: float, n_steps: int = 5000) -> tuple[float, float]:
        """目標位置に収束するまで実行し (最終 pos_mrad, 平均 load_mA) を返す。"""
        ctrl = ServoController()
        loads = []
        pos = 0.0
        for _ in range(n_steps):
            pos, load = ctrl.step_with_target(target_mrad)
            loads.append(load)
        # 後半 1/4 を定常とみなして平均負荷を計算
        steady_load = float(np.mean(loads[n_steps * 3 // 4:]))
        return pos, steady_load

    def test_home_pos_converges(self):
        """home (-785.4 mrad) に収束（誤差 ±20 mrad）"""
        pos, _ = self._run_until_settled(_deg_to_mrad(PARAMS.theta_eq))
        assert pos == pytest.approx(_deg_to_mrad(PARAMS.theta_eq), abs=20.0)

    def test_home_load_near_zero(self):
        """home 保持時の定常負荷 ≈ 0 mA（安定点なのでほぼ不要）"""
        _, load = self._run_until_settled(_deg_to_mrad(PARAMS.theta_eq))
        assert load == pytest.approx(0.0, abs=10.0)

    def test_up_pos_converges(self):
        """up (0 mrad) に収束（誤差 ±20 mrad）"""
        pos, _ = self._run_until_settled(0.0)
        assert pos == pytest.approx(0.0, abs=20.0)

    def test_up_load_positive(self):
        """up (0°) 保持時の定常負荷 ≈ +69.2 mA（重力に逆らう）"""
        _, load = self._run_until_settled(0.0)
        assert load == pytest.approx(69.2, abs=15.0)

    def test_down_pos_converges(self):
        """down (-1221.7 mrad ≈ -70°) に収束（誤差 ±20 mrad）"""
        target = _deg_to_mrad(-70.0)
        pos, _ = self._run_until_settled(target)
        assert pos == pytest.approx(target, abs=20.0)

    def test_down_load_negative(self):
        """down (-70°) 保持時の定常負荷 ≈ -71.4 mA（負方向に引き止める）"""
        _, load = self._run_until_settled(_deg_to_mrad(-70.0))
        assert load == pytest.approx(-71.4, abs=15.0)

    def test_pos_within_mechanical_limits(self):
        """全目標位置で出力が機械的限界内"""
        targets = [
            _deg_to_mrad(PARAMS.theta_eq),
            0.0,
            _deg_to_mrad(-70.0),
        ]
        ctrl = ServoController()
        for tgt in targets:
            ctrl.reset(target_mrad=tgt)
            for _ in range(3000):
                pos, _ = ctrl.step_with_target(tgt)
            assert pos >= _deg_to_mrad(PARAMS.theta_min) - 1.0
            assert pos <= _deg_to_mrad(PARAMS.theta_max) + 1.0

    def test_reset_to_custom(self):
        """reset() で任意 mrad に初期化できる"""
        ctrl = ServoController()
        ctrl.reset(target_mrad=0.0)
        assert ctrl.servo.theta == pytest.approx(0.0, abs=0.1)

    def test_reset_to_eq(self):
        """reset(None) で安定平衡点に初期化"""
        ctrl = ServoController()
        # まず別の位置に動かす
        for _ in range(100):
            ctrl.step_with_target(0.0)
        ctrl.reset()
        assert ctrl.servo.theta == pytest.approx(PARAMS.theta_eq, abs=0.1)


# ------------------------------------------------------------------
# generate_servo_periodic
# ------------------------------------------------------------------

class TestGenerateServoPeriodic:
    def test_output_shape_default(self):
        """デフォルト n_cycles=20 の shape"""
        pat = MotionPattern()
        u = generate_servo_periodic(n_cycles=20)
        assert u.shape == (20 * pat.steps_per_cycle, 2)

    def test_output_shape_custom_cycles(self):
        pat = MotionPattern()
        u = generate_servo_periodic(n_cycles=3)
        assert u.shape == (3 * pat.steps_per_cycle, 2)

    def test_dtype(self):
        u = generate_servo_periodic(n_cycles=1)
        assert u.dtype == np.float64

    def test_pos_within_physical_limits(self):
        """ノイズなし: pos が機械的限界内"""
        u = generate_servo_periodic(n_cycles=2, noise=None)
        pos_min = _deg_to_mrad(PARAMS.theta_min)
        pos_max = _deg_to_mrad(PARAMS.theta_max)
        assert np.all(u[:, 0] >= pos_min - 1.0)
        assert np.all(u[:, 0] <= pos_max + 1.0)

    def test_load_within_current_limits(self):
        """ノイズなし: load が電流限界内"""
        u = generate_servo_periodic(n_cycles=2, noise=None)
        assert np.all(u[:, 1] >= -PARAMS.current_max - 1.0)
        assert np.all(u[:, 1] <= +PARAMS.current_max + 1.0)

    def test_reproducibility_no_noise(self):
        """ノイズなし: 同一条件で完全一致"""
        u1 = generate_servo_periodic(n_cycles=2, noise=None)
        u2 = generate_servo_periodic(n_cycles=2, noise=None)
        np.testing.assert_array_equal(u1, u2)

    def test_reproducibility_with_noise(self):
        """ノイズあり: 同一シードで完全一致"""
        noise = MeasurementNoise(pos_std=2.0, load_std=3.0)
        u1 = generate_servo_periodic(n_cycles=2, noise=noise, rng=np.random.default_rng(42))
        u2 = generate_servo_periodic(n_cycles=2, noise=noise, rng=np.random.default_rng(42))
        np.testing.assert_array_equal(u1, u2)

    def test_noise_changes_output(self):
        """ノイズあり/なし で出力が異なる"""
        noise = MeasurementNoise()
        u_clean = generate_servo_periodic(n_cycles=1, noise=None)
        u_noisy = generate_servo_periodic(n_cycles=1, noise=noise, rng=np.random.default_rng(0))
        assert not np.allclose(u_clean, u_noisy)

    def test_esn_train_split_shape(self):
        """train_dual.py と同パターンの X/y split が可能"""
        u = generate_servo_periodic(n_cycles=3)
        X = u[:-1]
        y = u[1:]
        assert X.shape == (len(u) - 1, 2)
        assert y.shape == (len(u) - 1, 2)


# ------------------------------------------------------------------
# normalize_servo_obs
# ------------------------------------------------------------------

class TestNormalizeServoObs:
    def test_output_shape(self):
        u = generate_servo_periodic(n_cycles=1)
        norm = normalize_servo_obs(u)
        assert norm.shape == u.shape

    def test_range_no_noise(self):
        """ノイズなし: 正規化後が [-1, 1] に収まる"""
        u = generate_servo_periodic(n_cycles=5, noise=None)
        norm = normalize_servo_obs(u)
        assert norm[:, 0].min() >= -1.0 - 1e-6
        assert norm[:, 0].max() <= 1.0 + 1e-6
        assert norm[:, 1].min() >= -1.0 - 1e-6
        assert norm[:, 1].max() <= 1.0 + 1e-6

    def test_equilibrium_pos_near_center(self):
        """安定平衡点 (-785.4 mrad) の正規化値が -0.5 付近"""
        from esn_anomaly.servo.data import POS_MRAD_MIN, POS_MRAD_MAX
        center = (POS_MRAD_MAX + POS_MRAD_MIN) / 2.0
        scale = (POS_MRAD_MAX - POS_MRAD_MIN) / 2.0
        eq_mrad = _deg_to_mrad(PARAMS.theta_eq)
        expected = (eq_mrad - center) / scale
        u = np.array([[eq_mrad, 0.0]])
        norm = normalize_servo_obs(u)
        assert norm[0, 0] == pytest.approx(expected, rel=1e-6)



PARAMS = ServoParams()


# ------------------------------------------------------------------
# ばね定数と平衡点
# ------------------------------------------------------------------

class TestSpringConstants:
    def test_k_neg_formula(self):
        """K_neg = 1/(theta_eq - theta_reach_neg) = 1/35"""
        model = ServoModel(PARAMS)
        expected = 1.0 / (PARAMS.theta_eq - PARAMS.theta_reach_neg)
        assert model.k_neg == pytest.approx(expected)

    def test_k_pos_formula(self):
        """K_pos = 1/(theta_reach_pos - theta_eq) = 1/65"""
        model = ServoModel(PARAMS)
        expected = 1.0 / (PARAMS.theta_reach_pos - PARAMS.theta_eq)
        assert model.k_pos == pytest.approx(expected)

    def test_restoring_torque_at_eq_is_zero(self):
        """安定平衡点での復元トルクが 0"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq) == pytest.approx(0.0)

    def test_restoring_torque_sign_above_eq(self):
        """平衡点より正側: 負方向トルク（引き戻し）"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq + 10) < 0

    def test_restoring_torque_sign_below_eq(self):
        """平衡点より負側: 正方向トルク（引き戻し）"""
        model = ServoModel(PARAMS)
        assert model.restoring_torque(PARAMS.theta_eq - 10) > 0

    def test_stall_torque_neg_equals_pos(self):
        """負方向・正方向の最大復元トルクが等しい（対称モータ条件）"""
        model = ServoModel(PARAMS)
        t_neg = abs(model.restoring_torque(PARAMS.theta_reach_neg))
        t_pos = abs(model.restoring_torque(PARAMS.theta_reach_pos))
        assert t_neg == pytest.approx(t_pos, rel=1e-6)


# ------------------------------------------------------------------
# 静的平衡収束
# ------------------------------------------------------------------

class TestEquilibrium:
    def _run_until_settled(self, current: float, n_steps: int = 5000) -> float:
        model = ServoModel(PARAMS)
        for _ in range(n_steps):
            theta, _ = model.step(current)
        return theta

    def test_zero_current_converges_to_eq(self):
        """電流 0 → -45 度に収束（誤差 ±1 度以内）"""
        theta = self._run_until_settled(0.0)
        assert theta == pytest.approx(PARAMS.theta_eq, abs=1.0)

    def test_max_neg_current_converges_near_reach_neg(self):
        """最大負電流 → -80 度付近に収束（誤差 ±3 度以内）"""
        theta = self._run_until_settled(-PARAMS.current_max)
        assert theta == pytest.approx(PARAMS.theta_reach_neg, abs=3.0)

    def test_max_pos_current_converges_near_reach_pos(self):
        """最大正電流 → +20 度付近に収束（誤差 ±3 度以内）"""
        theta = self._run_until_settled(PARAMS.current_max)
        assert theta == pytest.approx(PARAMS.theta_reach_pos, abs=3.0)

    def test_from_positive_side_free_converges(self):
        """正側初期値から電流 0 で -45 度に収束"""
        model = ServoModel(PARAMS)
        model.reset(theta=20.0)
        for _ in range(5000):
            theta, _ = model.step(0.0)
        assert theta == pytest.approx(PARAMS.theta_eq, abs=1.0)


# ------------------------------------------------------------------
# 機械的限界
# ------------------------------------------------------------------

class TestMechanicalLimits:
    def test_never_exceeds_theta_max(self):
        """最大正電流を長時間与えても +90 度を超えない"""
        model = ServoModel(PARAMS)
        for _ in range(10000):
            theta, _ = model.step(PARAMS.current_max)
        assert theta <= PARAMS.theta_max

    def test_never_below_theta_min(self):
        """最大負電流を長時間与えても -90 度を下回らない"""
        model = ServoModel(PARAMS)
        for _ in range(10000):
            theta, _ = model.step(-PARAMS.current_max)
        assert theta >= PARAMS.theta_min

    def test_overcurrent_is_clipped(self):
        """200 mA 入力は 100 mA に相当する挙動（限界内に留まる）"""
        model = ServoModel(PARAMS)
        for _ in range(5000):
            theta, _ = model.step(200.0)
        assert theta <= PARAMS.theta_max


# ------------------------------------------------------------------
# reset
# ------------------------------------------------------------------

class TestReset:
    def test_reset_to_eq(self):
        model = ServoModel(PARAMS)
        model.step(100.0)
        model.reset()
        assert model.theta == pytest.approx(PARAMS.theta_eq)
        assert model.omega == pytest.approx(0.0)

    def test_reset_to_custom(self):
        model = ServoModel(PARAMS)
        model.reset(theta=10.0, omega=5.0)
        assert model.theta == pytest.approx(10.0)
        assert model.omega == pytest.approx(5.0)


# ------------------------------------------------------------------
# データ生成関数
# ------------------------------------------------------------------

class TestGenerateServoStep:
    def test_output_shape(self):
        u, y = generate_servo_step(n_steps=500, rng=np.random.default_rng(0))
        assert u.shape == (500, 1)
        assert y.shape == (500, 1)

    def test_u_range(self):
        u, _ = generate_servo_step(n_steps=1000, rng=np.random.default_rng(1))
        assert u.min() >= -1.0 - 1e-9
        assert u.max() <= 1.0 + 1e-9

    def test_y_range(self):
        _, y = generate_servo_step(n_steps=1000, rng=np.random.default_rng(2))
        assert y.min() >= -1.0 - 1e-9
        assert y.max() <= 1.0 + 1e-9


class TestGenerateServoTrain:
    def test_output_shape(self):
        X, y = generate_servo_train(n_steps=500, rng=np.random.default_rng(0))
        assert X.shape == (500, 2)
        assert y.shape == (500, 1)

    def test_x_columns_in_range(self):
        X, _ = generate_servo_train(n_steps=500, rng=np.random.default_rng(3))
        assert X[:, 0].min() >= -1.0 - 1e-9  # 電流
        assert X[:, 0].max() <= 1.0 + 1e-9
        assert X[:, 1].min() >= -1.0 - 1e-9  # 前ステップ角度
        assert X[:, 1].max() <= 1.0 + 1e-9


class TestGenerateServoTestScenario:
    @pytest.mark.parametrize("scenario", ["step", "sine", "max_neg", "max_pos", "free"])
    def test_output_keys_and_lengths(self, scenario):
        result = generate_servo_test_scenario(scenario=scenario, n_steps=200)
        for key in ("t", "theta", "omega", "current"):
            assert key in result
            assert len(result[key]) == 200

    def test_max_neg_theta_near_reach_neg(self):
        """最大負電流シナリオ: 最終角度が reach_neg 付近"""
        result = generate_servo_test_scenario(scenario="max_neg", n_steps=2000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_reach_neg, abs=3.0)

    def test_max_pos_theta_near_reach_pos(self):
        """最大正電流シナリオ: 最終角度が reach_pos 付近"""
        result = generate_servo_test_scenario(scenario="max_pos", n_steps=2000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_reach_pos, abs=3.0)

    def test_free_converges_to_eq(self):
        """電流 0 の free シナリオ: 最終角度が平衡点付近"""
        result = generate_servo_test_scenario(scenario="free", n_steps=3000)
        assert result["theta"][-1] == pytest.approx(PARAMS.theta_eq, abs=1.0)

    def test_theta_always_within_limits(self):
        """全シナリオで角度が機械的限界内"""
        for scenario in ("step", "sine", "max_neg", "max_pos", "free"):
            result = generate_servo_test_scenario(scenario=scenario, n_steps=500)
            assert np.all(result["theta"] >= PARAMS.theta_min)
            assert np.all(result["theta"] <= PARAMS.theta_max)

    def test_unknown_scenario_raises(self):
        with pytest.raises(ValueError, match="未知のシナリオ"):
            generate_servo_test_scenario(scenario="invalid")


# ------------------------------------------------------------------
# 異常注入 (Phase 12)
# ------------------------------------------------------------------

from esn_anomaly.servo.data import (
    ANOMALY_FORCE,
    ANOMALY_LOAD_STUCK,
    ANOMALY_POS_DRIFT,
    ANOMALY_POS_SPIKE,
    ServoAnomalySegment,
    inject_anomaly_segment,
    generate_servo_anomaly_test,
)


class TestServoAnomalySegment:
    def test_length_property(self):
        seg = ServoAnomalySegment(start=10, end=60, kind=ANOMALY_FORCE, magnitude=30.0)
        assert seg.length == 50

    def test_frozen(self):
        seg = ServoAnomalySegment(start=0, end=10, kind=ANOMALY_FORCE, magnitude=1.0)
        with pytest.raises((AttributeError, TypeError)):
            seg.start = 5  # type: ignore[misc]


class TestInjectAnomalySegment:
    def _base_data(self) -> np.ndarray:
        return generate_servo_periodic(n_cycles=1, noise=None)[:100]

    def test_force_modifies_load_only(self):
        u = self._base_data()
        seg = ServoAnomalySegment(start=20, end=50, kind=ANOMALY_FORCE, magnitude=30.0)
        u2 = inject_anomaly_segment(u, seg, np.random.default_rng(0))
        np.testing.assert_array_equal(u[:, 0], u2[:, 0])
        assert not np.allclose(u[20:50, 1], u2[20:50, 1])
        np.testing.assert_allclose(u2[20:50, 1] - u[20:50, 1], 30.0)

    def test_pos_spike_modifies_pos_only(self):
        u = self._base_data()
        seg = ServoAnomalySegment(start=10, end=80, kind=ANOMALY_POS_SPIKE, magnitude=200.0)
        u2 = inject_anomaly_segment(u, seg, np.random.default_rng(1))
        np.testing.assert_array_equal(u[:, 1], u2[:, 1])
        assert np.abs(u2[10:80, 0] - u[10:80, 0]).max() > 0

    def test_pos_drift_starts_zero_ends_magnitude(self):
        u = self._base_data()
        seg = ServoAnomalySegment(start=5, end=55, kind=ANOMALY_POS_DRIFT, magnitude=300.0)
        u2 = inject_anomaly_segment(u, seg, np.random.default_rng(2))
        diff = u2[5:55, 0] - u[5:55, 0]
        assert abs(diff[0]) < 1e-9
        assert abs(diff[-1] - 300.0) < 1e-9

    def test_load_stuck_constant_in_segment(self):
        u = self._base_data()
        seg = ServoAnomalySegment(start=30, end=70, kind=ANOMALY_LOAD_STUCK, magnitude=0.0)
        u2 = inject_anomaly_segment(u, seg, np.random.default_rng(3))
        assert np.all(u2[30:70, 1] == u[30, 1])

    def test_returns_copy(self):
        u = self._base_data()
        u_orig = u.copy()
        seg = ServoAnomalySegment(start=0, end=50, kind=ANOMALY_FORCE, magnitude=20.0)
        inject_anomaly_segment(u, seg, np.random.default_rng(0))
        np.testing.assert_array_equal(u, u_orig)

    def test_unknown_kind_raises(self):
        u = self._base_data()
        seg = ServoAnomalySegment(start=0, end=10, kind="unknown", magnitude=1.0)
        with pytest.raises(ValueError, match="未知の異常種別"):
            inject_anomaly_segment(u, seg, np.random.default_rng(0))


class TestGenerateServoAnomalyTest:
    def test_normal_returns_no_segments(self):
        u, segs = generate_servo_anomaly_test(n_cycles=2, anomaly_kinds=[],
                                              rng=np.random.default_rng(0))
        assert segs == []

    def test_output_shape(self):
        pat = MotionPattern()
        u, _ = generate_servo_anomaly_test(n_cycles=3, anomaly_kinds=[ANOMALY_FORCE],
                                           rng=np.random.default_rng(0))
        assert u.shape == (3 * pat.steps_per_cycle, 2)

    def test_segments_within_bounds(self):
        u, segs = generate_servo_anomaly_test(n_cycles=5, anomaly_kinds=[ANOMALY_POS_SPIKE],
                                              rng=np.random.default_rng(1))
        n = len(u)
        for seg in segs:
            assert 0 <= seg.start < seg.end <= n

    def test_anomaly_kinds_are_assigned(self):
        kinds = [ANOMALY_FORCE, ANOMALY_POS_DRIFT]
        _, segs = generate_servo_anomaly_test(n_cycles=5, anomaly_kinds=kinds,
                                              rng=np.random.default_rng(2))
        for seg in segs:
            assert seg.kind in kinds

    def test_all_anomaly_kinds(self):
        all_kinds = [ANOMALY_FORCE, ANOMALY_POS_SPIKE, ANOMALY_POS_DRIFT, ANOMALY_LOAD_STUCK]
        u, segs = generate_servo_anomaly_test(n_cycles=3, anomaly_kinds=all_kinds,
                                              noise=MeasurementNoise(),
                                              rng=np.random.default_rng(42))
        assert u.shape[1] == 2

    def test_reproducibility(self):
        u1, s1 = generate_servo_anomaly_test(n_cycles=2, anomaly_kinds=[ANOMALY_FORCE],
                                             rng=np.random.default_rng(7))
        u2, s2 = generate_servo_anomaly_test(n_cycles=2, anomaly_kinds=[ANOMALY_FORCE],
                                             rng=np.random.default_rng(7))
        np.testing.assert_array_equal(u1, u2)
        assert len(s1) == len(s2)
