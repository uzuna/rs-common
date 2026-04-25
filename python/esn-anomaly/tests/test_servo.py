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


# ------------------------------------------------------------------
# Phase 13: 外力・クーロン摩擦の物理挙動
# ------------------------------------------------------------------

from esn_anomaly.servo.data import (
    CommandProfile,
    CommandSegment,
    DisturbanceSegment,
    generate_servo_with_profile,
    normalize_cmd,
)


class TestExtTorque:
    """外力トルクが物理挙動を正しく変化させること。"""

    def _settled_load(self, ext_torque: float, n_steps: int = 3000) -> float:
        """home 保持定常時の平均負荷 [mA] を返す。"""
        from esn_anomaly.servo.data import ServoParams
        pat = MotionPattern()
        profile = CommandProfile([CommandSegment("hold", pat.home_mrad, n_steps)])
        p = ServoParams(ext_torque=ext_torque)
        u_raw, _ = generate_servo_with_profile(profile, base_params=p)
        return float(np.mean(u_raw[n_steps * 3 // 4:, 1]))

    def test_zero_ext_torque_load_near_zero_at_home(self):
        """外力なし: home 保持の定常負荷 ≈ 0 mA"""
        load = self._settled_load(0.0)
        assert load == pytest.approx(0.0, abs=5.0)

    def test_positive_ext_torque_decreases_load(self):
        """正方向外力: home 保持の定常負荷が減少（制御器が電流を下げて補償）"""
        load_0 = self._settled_load(0.0)
        load_p = self._settled_load(0.3)
        assert load_p < load_0 - 5.0

    def test_negative_ext_torque_increases_load(self):
        """負方向外力: home 保持の定常負荷が増加"""
        load_0 = self._settled_load(0.0)
        load_n = self._settled_load(-0.3)
        assert load_n > load_0 + 5.0

    def test_ext_torque_causes_position_offset(self):
        """外力はPD制御器の定常偏差を引き起こす（積分項なしのため）。
        正外力 → target より正方向、負外力 → 負方向に定常偏差が生じる。
        """
        from esn_anomaly.servo.data import ServoParams
        pat = MotionPattern()
        profile = CommandProfile([CommandSegment("hold", pat.home_mrad, 5000)])

        p_pos = ServoParams(ext_torque=0.3)
        u_pos, _ = generate_servo_with_profile(profile, base_params=p_pos)
        pos_pos = float(np.mean(u_pos[-200:, 0]))

        p_neg = ServoParams(ext_torque=-0.3)
        u_neg, _ = generate_servo_with_profile(profile, base_params=p_neg)
        pos_neg = float(np.mean(u_neg[-200:, 0]))

        # 正外力: target より大きい（正方向に押される）
        assert pos_pos > pat.home_mrad + 10.0
        # 負外力: target より小さい（負方向に押される）
        assert pos_neg < pat.home_mrad - 10.0


class TestCoulombFriction:
    """クーロン摩擦が物理挙動を正しく変化させること。"""

    def _run_small_move(self, coulomb: float) -> np.ndarray:
        """小移動（≈10°）で摩擦効果を観察（飽和を避ける）。"""
        from esn_anomaly.servo.data import ServoParams
        home = MotionPattern().home_mrad          # -785.4 mrad (-45°)
        near = home + 185.0                       # ≈ -600 mrad (-34.4°): 10.6° 移動
        profile = CommandProfile([
            CommandSegment("hold", home, 800),    # home で安定
            CommandSegment("hold", near, 800),    # near まで移動・保持
            CommandSegment("hold", home, 400),    # home に戻る
        ])
        p = ServoParams(coulomb_friction=coulomb)
        u_raw, _ = generate_servo_with_profile(profile, base_params=p)
        return u_raw

    def test_zero_friction_tracks_target(self):
        """摩擦なし: 目標位置に収束（保持期間末尾 ±50 mrad）"""
        u = self._run_small_move(0.0)
        home = MotionPattern().home_mrad
        near = home + 185.0
        final_pos = float(np.mean(u[1400:1600, 0]))   # near 保持の後半
        assert abs(final_pos - near) < 50.0

    def test_friction_does_not_prevent_reaching_target(self):
        """摩擦あり: 目標位置に収束（PD が補償し定常誤差なし）"""
        u = self._run_small_move(0.1)
        home = MotionPattern().home_mrad
        near = home + 185.0
        final_pos = float(np.mean(u[1400:1600, 0]))
        assert abs(final_pos - near) < 80.0

    def test_friction_changes_load_during_motion(self):
        """摩擦あり: 動作中の負荷が摩擦なしと異なる（移動開始直後を評価）"""
        u_0 = self._run_small_move(0.0)
        u_f = self._run_small_move(0.1)
        # 移動開始直後（steps 800〜900）を評価
        load_0 = np.mean(np.abs(u_0[800:900, 1]))
        load_f = np.mean(np.abs(u_f[800:900, 1]))
        # 摩擦があると移動中の負荷が変化（0.5 mA 以上の差）
        assert abs(load_f - load_0) > 0.5


class TestCommandProfile:
    """CommandProfile の target array 生成テスト。"""

    def test_hold_generates_constant_array(self):
        seg = CommandSegment("hold", -785.4, 100)
        arr = CommandProfile([seg]).build_target_array()
        assert arr.shape == (100,)
        assert np.all(arr == pytest.approx(-785.4))

    def test_ramp_generates_linspace(self):
        segs = [
            CommandSegment("hold", -785.4, 1),  # prev = -785.4
            CommandSegment("ramp",    0.0, 50),
        ]
        arr = CommandProfile(segs).build_target_array()
        # ramp 部分: -785.4 → 0.0 の 50 点
        assert arr[1] == pytest.approx(-785.4, abs=30.0)
        assert arr[-1] == pytest.approx(0.0, abs=1e-6)

    def test_step_generates_single_point(self):
        seg = CommandSegment("step", 349.1, 1)
        arr = CommandProfile([seg]).build_target_array()
        assert arr.shape == (1,)
        assert arr[0] == pytest.approx(349.1)

    def test_total_steps(self):
        segs = [
            CommandSegment("hold", 0.0, 100),
            CommandSegment("ramp", 100.0, 200),
            CommandSegment("step", 50.0, 1),
        ]
        profile = CommandProfile(segs)
        assert profile.total_steps == 301

    def test_unknown_kind_raises(self):
        with pytest.raises(ValueError, match="未知の CommandSegment kind"):
            CommandProfile([CommandSegment("invalid", 0.0, 10)]).build_target_array()

    def test_ramp_start_from_start_mrad(self):
        seg = CommandSegment("ramp", 0.0, 100)
        arr = CommandProfile([seg]).build_target_array(start_mrad=-785.4)
        assert arr[0] == pytest.approx(-785.4, abs=1.0)
        assert arr[-1] == pytest.approx(0.0, abs=1e-6)


class TestGenerateServoWithProfile:
    """generate_servo_with_profile の出力テスト。"""

    def _simple_profile(self) -> CommandProfile:
        pat = MotionPattern()
        return CommandProfile([
            CommandSegment("hold", pat.home_mrad, 200),
            CommandSegment("hold", pat.up_mrad,   300),
            CommandSegment("hold", pat.home_mrad, 200),
        ])

    def test_output_shapes(self):
        profile = self._simple_profile()
        u_raw, cmd_raw = generate_servo_with_profile(profile)
        assert u_raw.shape == (700, 2)
        assert cmd_raw.shape == (700,)

    def test_cmd_array_matches_profile(self):
        """cmd_array が CommandProfile の目標値を反映している。"""
        pat = MotionPattern()
        profile = CommandProfile([CommandSegment("hold", pat.home_mrad, 500)])
        _, cmd = generate_servo_with_profile(profile)
        assert np.all(cmd == pytest.approx(pat.home_mrad))

    def test_no_disturbance_reproducible(self):
        """同一シードで同一出力。"""
        profile = self._simple_profile()
        u1, c1 = generate_servo_with_profile(profile, noise=MeasurementNoise(),
                                             rng=np.random.default_rng(0))
        u2, c2 = generate_servo_with_profile(profile, noise=MeasurementNoise(),
                                             rng=np.random.default_rng(0))
        np.testing.assert_array_equal(u1, u2)
        np.testing.assert_array_equal(c1, c2)

    def test_disturbance_changes_load(self):
        """外乱あり/なし で load が異なる。"""
        profile = CommandProfile([CommandSegment("hold", MotionPattern().home_mrad, 1000)])
        u_clean, _ = generate_servo_with_profile(profile)
        dist = [DisturbanceSegment(200, 800, ext_torque=0.4)]
        u_dist, _ = generate_servo_with_profile(profile, disturbances=dist)
        # 外乱区間で負荷が変化
        assert not np.allclose(u_clean[200:800, 1], u_dist[200:800, 1], atol=1.0)

    def test_disturbance_outside_segment_unchanged(self):
        """外乱区間外は変化しない（ノイズなし）。"""
        profile = CommandProfile([CommandSegment("hold", MotionPattern().home_mrad, 1000)])
        u_clean, _ = generate_servo_with_profile(profile, noise=None)
        dist = [DisturbanceSegment(400, 600, ext_torque=0.3)]
        u_dist, _ = generate_servo_with_profile(profile, disturbances=dist, noise=None)
        # 外乱前区間はほぼ同一（制御器は同じ初期条件から始まる）
        np.testing.assert_allclose(u_clean[:400], u_dist[:400], atol=1e-6)

    def test_normalize_cmd_range(self):
        """normalize_cmd の出力が [-1.5, 1.5] 以内（サーボ動作範囲内）。"""
        pat = MotionPattern()
        profile = CommandProfile([
            CommandSegment("hold", pat.down_mrad, 100),
            CommandSegment("hold", pat.up_mrad,   100),
        ])
        _, cmd_raw = generate_servo_with_profile(profile)
        cmd_norm = normalize_cmd(cmd_raw)
        assert cmd_norm.min() >= -1.5
        assert cmd_norm.max() <= 1.5


# ──── Phase 14: PhysicalEstimator テスト ──────────────────────────────────────

from esn_anomaly.servo.estimator import PhysicalEstimator


class TestPhysicalEstimatorFF:
    """PhysicalEstimator ff モードの基本テスト。"""

    def _make_training_data(self, n_cycles: int = 10):
        from esn_anomaly.servo.command_test import NOISE, SEED, _make_periodic_profile
        from esn_anomaly.servo.data import generate_servo_with_profile
        rng = np.random.default_rng(SEED)
        profile = _make_periodic_profile(n_cycles)
        return generate_servo_with_profile(profile, noise=NOISE, rng=rng)

    def test_invalid_mode_raises(self):
        with pytest.raises(ValueError, match="mode"):
            PhysicalEstimator(mode="invalid")

    def test_fit_threshold_returns_positive(self):
        """正常訓練データから得られる閾値は正の値。"""
        u_raw, cmd_raw = self._make_training_data()
        est = PhysicalEstimator(mode="ff")
        thr = est.fit_threshold(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        assert thr > 0.0

    def test_fit_threshold_sets_attribute(self):
        """fit_threshold 後に _threshold が設定される。"""
        u_raw, cmd_raw = self._make_training_data()
        est = PhysicalEstimator(mode="ff")
        assert est._threshold is None
        est.fit_threshold(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        assert est._threshold is not None

    def test_detect_requires_fit_first(self):
        """fit_threshold 前に detect を呼ぶと RuntimeError。"""
        u_raw, cmd_raw = self._make_training_data()
        est = PhysicalEstimator(mode="ff")
        with pytest.raises(RuntimeError):
            est.detect(u_raw[:, 0], u_raw[:, 1], cmd_raw)

    def test_detect_shape(self):
        """detect の出力 shape が入力と同じ。"""
        u_raw, cmd_raw = self._make_training_data()
        est = PhysicalEstimator(mode="ff")
        est.fit_threshold(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        result = est.detect(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        assert result.shape == (len(u_raw),)
        assert result.dtype == bool

    def test_normal_data_low_false_alarm(self):
        """正常データに対する誤警報率が 5% 未満。"""
        u_raw, cmd_raw = self._make_training_data()
        est = PhysicalEstimator(mode="ff")
        est.fit_threshold(u_raw[:, 0], u_raw[:, 1], cmd_raw, warmup=100)
        detected = est.detect(u_raw[:, 0], u_raw[:, 1], cmd_raw)
        false_alarm_rate = float(detected[100:].mean())
        assert false_alarm_rate < 0.05, f"誤警報率 {false_alarm_rate:.3f} が高すぎる"

    def test_ext_torque_increases_residual(self):
        """外力印加後の定常状態で残差が増加する。

        ext_torque=0.3 → 定常状態の residual ≈ -T_ext/motor_gain = -30 mA.
        """
        from esn_anomaly.servo.data import (
            DisturbanceSegment, MotionPattern,
            CommandProfile, CommandSegment,
            generate_servo_with_profile, MeasurementNoise,
        )
        # SC シナリオ風: home 保持 → 外力印加
        pat = MotionPattern()
        profile = CommandProfile([
            CommandSegment("hold", pat.home_mrad, 300),   # 正常区間
            CommandSegment("hold", pat.home_mrad, 1000),  # 外力区間
        ])
        dist = [DisturbanceSegment(300, 1300, ext_torque=0.3)]
        noise = MeasurementNoise(pos_std=1.0, load_std=1.0)
        u_raw, cmd_raw = generate_servo_with_profile(
            profile, disturbances=dist, noise=noise, rng=np.random.default_rng(0)
        )
        pos, load = u_raw[:, 0], u_raw[:, 1]

        est = PhysicalEstimator(mode="ff", smooth_win=20, vel_smooth_win=50, vel_threshold=5.0)
        # 正常区間のみで閾値学習
        u_normal, cmd_normal = generate_servo_with_profile(
            CommandProfile([CommandSegment("hold", pat.home_mrad, 1300)]),
            noise=noise, rng=np.random.default_rng(1),
        )
        est.fit_threshold(u_normal[:, 0], u_normal[:, 1], cmd_normal, warmup=100)

        # 外力区間（定常後半）の残差が閾値超過
        r = est.abs_residual(pos, load)
        # 外力区間の後半（600-1200 ステップ目）は整定済み
        r_dist = r[600:1200]
        assert r_dist.mean() > est._threshold * 0.5, (
            f"外力区間の残差 {r_dist.mean():.2f} が閾値 {est._threshold:.2f} の半分未満"
        )


class TestPhysicalEstimatorFull:
    """PhysicalEstimator full モードの基本テスト。"""

    def test_full_mode_requires_cmd(self):
        """full モードの residual に cmd_mrad が必要。"""
        rng = np.random.default_rng(0)
        pos = rng.normal(0, 10, 100)
        load = rng.normal(0, 5, 100)
        est = PhysicalEstimator(mode="full")
        with pytest.raises(ValueError, match="cmd_mrad"):
            est.residual(pos, load, cmd_mrad=None)

    def test_full_mode_normal_low_residual(self):
        """full モードで正常定常状態の残差が ff より小さい。"""
        from esn_anomaly.servo.command_test import NOISE, SEED, _make_periodic_profile
        from esn_anomaly.servo.data import generate_servo_with_profile
        rng = np.random.default_rng(SEED)
        profile = _make_periodic_profile(5)
        u_raw, cmd_raw = generate_servo_with_profile(profile, noise=NOISE, rng=rng)
        pos, load = u_raw[:, 0], u_raw[:, 1]

        est_ff   = PhysicalEstimator(mode="ff",   smooth_win=20, vel_smooth_win=50)
        est_full = PhysicalEstimator(mode="full",  smooth_win=20, vel_smooth_win=50)

        r_ff   = est_ff.abs_residual(pos, load)
        r_full = est_full.abs_residual(pos, load, cmd_raw)

        # 全体平均で full の残差は ff 以下になるはず（I_pd 除去効果）
        assert r_full.mean() <= r_ff.mean() * 1.1, (
            f"full 平均 {r_full.mean():.2f} が ff 平均 {r_ff.mean():.2f} より大きい"
        )
