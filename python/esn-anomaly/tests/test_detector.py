"""detector.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.data import generate_test_data, generate_train_data
from esn_anomaly.detector import (
    compute_residual,
    compute_threshold,
    detect,
    first_detection,
    smooth,
)
from esn_anomaly.model import ESNConfig, ESNModel


@pytest.fixture(scope="module")
def pipeline_result():
    """学習済みモデルでテストデータを推論した結果を返す。"""
    rng = np.random.default_rng(0)
    X_tr, y_tr = generate_train_data(n_steps=2000, rng=rng)
    model = ESNModel(ESNConfig(units=100, seed=0))
    model.fit(X_tr, y_tr)

    # 訓練誤差（冒頭 100 ステップはウォームアップ期間として除外）
    pred_tr = model.predict(X_tr)
    train_errors = compute_residual(y_tr, pred_tr)
    WARMUP = 100
    train_errors_stable = train_errors[WARMUP:]

    # テスト推論（訓練データ末尾をウォームアップに使用）
    rng2 = np.random.default_rng(1)
    X_te, y_te, labels = generate_test_data(n_steps=1000, switch_at=500, rng=rng2)
    pred_te = model.predict(X_te, warmup_data=X_tr[-WARMUP:])
    test_errors = compute_residual(y_te, pred_te)
    smoothed = smooth(test_errors, window=15)
    threshold = compute_threshold(train_errors_stable)

    EVAL_START = 30  # ウォームアップ過渡期（約30ステップ）を除いて評価

    return {
        "smoothed": smoothed,
        "threshold": threshold,
        "labels": labels,
        "switch_at": 500,
        "eval_start": EVAL_START,
    }


class TestComputeResidual:
    def test_shape(self):
        y = np.ones((10, 1))
        p = np.zeros((10, 1))
        e = compute_residual(y, p)
        assert e.shape == (10,)

    def test_values(self):
        y = np.array([[1.0], [2.0], [3.0]])
        p = np.array([[0.0], [2.0], [4.0]])
        e = compute_residual(y, p)
        np.testing.assert_allclose(e, [1.0, 0.0, 1.0])


class TestSmooth:
    def test_output_same_length(self):
        e = np.random.rand(100)
        s = smooth(e, window=10)
        assert s.shape == e.shape

    def test_constant_input(self):
        e = np.ones(50)
        s = smooth(e, window=5)
        np.testing.assert_allclose(s, np.ones(50), atol=1e-10)

    def test_window_1_is_identity(self):
        e = np.random.rand(30)
        s = smooth(e, window=1)
        np.testing.assert_array_equal(s, e)


class TestComputeThreshold:
    def test_3sigma(self):
        e = np.array([1.0, 2.0, 3.0, 4.0, 5.0])
        th = compute_threshold(e, method="3sigma")
        expected = e.mean() + 3.0 * e.std()
        assert pytest.approx(th) == expected

    def test_max(self):
        e = np.array([1.0, 2.0, 5.0, 3.0])
        th = compute_threshold(e, method="max")
        assert th == 5.0

    def test_invalid_method(self):
        with pytest.raises(ValueError):
            compute_threshold(np.ones(10), method="invalid")


class TestDetect:
    def test_empty_when_below_threshold(self):
        e = np.array([0.1, 0.2, 0.3])
        result = detect(e, threshold=1.0)
        assert len(result) == 0

    def test_returns_correct_indices(self):
        e = np.array([0.1, 1.5, 0.2, 2.0])
        result = detect(e, threshold=1.0)
        np.testing.assert_array_equal(result, [1, 3])


class TestFirstDetection:
    def test_none_when_no_detection(self):
        e = np.array([0.1, 0.2])
        assert first_detection(e, threshold=1.0) is None

    def test_returns_first_index(self):
        e = np.array([0.1, 0.2, 1.5, 2.0])
        assert first_detection(e, threshold=1.0) == 2


class TestEndToEnd:
    def test_no_false_positive_in_sine_region(self, pipeline_result):
        """サイン波区間（ステップ eval_start〜switch-20）で誤検知しないこと。"""
        switch = pipeline_result["switch_at"]
        eval_start = pipeline_result["eval_start"]
        smoothed = pipeline_result["smoothed"]
        threshold = pipeline_result["threshold"]
        pre = smoothed[eval_start : switch - 20]
        assert (pre <= threshold).all(), (
            f"サイン波区間で誤検知: max={pre.max():.4f}, threshold={threshold:.4f}"
        )

    def test_detects_in_square_region(self, pipeline_result):
        """矩形波区間（ステップ 500 以降）で検知されること。"""
        switch = pipeline_result["switch_at"]
        smoothed = pipeline_result["smoothed"]
        threshold = pipeline_result["threshold"]
        post = smoothed[switch:]
        assert (post > threshold).any(), (
            f"矩形波区間で検知なし: max={post.max():.4f}, threshold={threshold:.4f}"
        )

    def test_first_detection_after_switch(self, pipeline_result):
        """最初の検知が eval_start 以降で、かつ切り替え前後 20 ステップ以内であること。"""
        switch = pipeline_result["switch_at"]
        eval_start = pipeline_result["eval_start"]
        smoothed = pipeline_result["smoothed"]
        threshold = pipeline_result["threshold"]
        idx = first_detection(smoothed[eval_start:], threshold)
        if idx is not None:
            idx += eval_start
        assert idx is not None, "全区間で検知なし"
        assert idx >= switch - 20, f"切り替え前に検知: idx={idx}, switch_at={switch}"
