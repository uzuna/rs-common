"""AdaptiveThreshold の単体テストと収束テスト（Phase 16）。"""

from __future__ import annotations

import numpy as np
import pytest

from esn_anomaly.servo.adaptive import AdaptiveThreshold


class TestAdaptiveThresholdBasic:
    def test_initial_threshold(self):
        at = AdaptiveThreshold(initial=0.1)
        assert at.threshold == pytest.approx(0.1)

    def test_update_increases_on_error(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01)
        old = at.threshold
        at.update(True)
        assert at.threshold > old

    def test_update_decreases_on_non_error(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01, target_rate=0.05)
        old = at.threshold
        at.update(False)
        assert at.threshold < old

    def test_update_delta_on_error(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01, hard_min=0.0)
        at.update(True)
        assert at.threshold == pytest.approx(0.11)

    def test_update_delta_on_non_error(self):
        at = AdaptiveThreshold(
            initial=0.1, alpha=0.01, target_rate=0.05, hard_min=0.0
        )
        at.update(False)
        assert at.threshold == pytest.approx(0.1 - 0.01 * 0.05)

    def test_hard_min_clamp(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01, target_rate=1.0, hard_min=0.09)
        # alpha * target = 0.01 * 1.0 = 0.01 → threshold would go to 0.09
        at.update(False)
        assert at.threshold == pytest.approx(0.09)
        at.update(False)  # would go below hard_min
        assert at.threshold == pytest.approx(0.09)

    def test_hard_max_clamp(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01, hard_max=0.11)
        at.update(True)  # → 0.11
        assert at.threshold == pytest.approx(0.11)
        at.update(True)  # clamped at hard_max
        assert at.threshold == pytest.approx(0.11)

    def test_hard_max_none_means_no_upper_limit(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.05, hard_max=None)
        for _ in range(20):
            at.update(True)
        assert at.threshold > 0.5  # 0.1 + 20*0.05 = 1.1 なら制限なし

    def test_returns_threshold(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.01)
        ret = at.update(True)
        assert ret == pytest.approx(at.threshold)


class TestAdaptiveThresholdMetrics:
    def test_error_rate_zero_initially(self):
        at = AdaptiveThreshold(initial=0.1)
        assert at.error_rate == 0.0

    def test_error_rate_all_errors(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.0)  # alpha=0 でしきい値固定
        for _ in range(10):
            at.update(True)
        assert at.error_rate == pytest.approx(1.0)

    def test_error_rate_half(self):
        at = AdaptiveThreshold(initial=0.1, alpha=0.0)
        for i in range(10):
            at.update(i % 2 == 0)
        assert at.error_rate == pytest.approx(0.5)

    def test_reset_restores_initial(self):
        at = AdaptiveThreshold(initial=0.1)
        for _ in range(50):
            at.update(True)
        at.reset()
        assert at.threshold == pytest.approx(0.1)
        assert at.error_rate == 0.0
        assert at._n_total == 0

    def test_default_alpha(self):
        at = AdaptiveThreshold(initial=0.2)
        assert at.alpha == pytest.approx(0.002)  # initial * 0.01

    def test_default_hard_min(self):
        """デフォルト hard_min は initial × 0.05。"""
        at = AdaptiveThreshold(initial=0.2)
        assert at.hard_min == pytest.approx(0.01)


class TestAdaptiveThresholdConvergence:
    def test_equilibrium_uniform_residuals(self):
        """U(0,1) の残差に対して、しきい値が目標エラー率の分位点に収束する。

        target_rate=0.10 のとき、平衡点は 0.90（U(0,1) の 90 パーセンタイル）。
        """
        rng = np.random.default_rng(42)
        target_rate = 0.10
        n_samples = 8000

        at = AdaptiveThreshold(
            initial=0.5,
            target_rate=target_rate,
            alpha=0.005,
            hard_min=0.0,
            hard_max=1.0,
        )

        for _ in range(n_samples):
            residual = float(rng.uniform(0.0, 1.0))
            at.update(residual > at.threshold)

        # 平衡点 0.90 ± 0.05 の範囲に収束
        assert abs(at.threshold - 0.90) < 0.05

    def test_high_fpr_converges_down(self):
        """FPR > target の状況（初期しきい値が低すぎ）でしきい値が上昇する。"""
        rng = np.random.default_rng(99)
        # 残差 ~ N(0.5, 0.1)、threshold=0.1 で初期 FPR ≈ 100%
        at = AdaptiveThreshold(
            initial=0.1, target_rate=0.05, alpha=0.005,
            hard_min=0.0, hard_max=2.0,
        )
        for _ in range(3000):
            residual = float(rng.normal(0.5, 0.1))
            at.update(residual > at.threshold)

        # しきい値が上昇して残差分布の上端近くに落ち着く
        assert at.threshold > 0.5

    def test_low_fpr_converges_up(self):
        """FPR < target の状況（初期しきい値が高すぎ）でしきい値が低下する。"""
        rng = np.random.default_rng(7)
        # 残差 ~ U(0, 0.3)、threshold=0.9 で初期 FPR ≈ 0%
        at = AdaptiveThreshold(
            initial=0.9, target_rate=0.10, alpha=0.005,
            hard_min=0.0, hard_max=2.0,
        )
        for _ in range(5000):
            residual = float(rng.uniform(0.0, 0.3))
            at.update(residual > at.threshold)

        # しきい値が低下して 0.3 × 0.90 = 0.27 付近に収束
        assert at.threshold < 0.5

    def test_error_rate_near_target_after_convergence(self):
        """収束後の累積エラー率が target_rate ± 3% の範囲にある。"""
        rng = np.random.default_rng(123)
        target_rate = 0.05
        n_samples = 10000

        at = AdaptiveThreshold(
            initial=0.5,
            target_rate=target_rate,
            alpha=0.002,
            hard_min=0.0,
            hard_max=1.0,
        )
        for _ in range(n_samples):
            residual = float(rng.uniform(0.0, 1.0))
            at.update(residual > at.threshold)

        assert abs(at.error_rate - target_rate) < 0.03
