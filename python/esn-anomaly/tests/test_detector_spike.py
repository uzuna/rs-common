"""detector_spike.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.data_spike import (
    generate_spike_train,
    spike_length,
)
from esn_anomaly.detector import compute_residual
from esn_anomaly.detector_spike import (
    classify_spike,
    collect_train_spike_scores,
    compute_spike_threshold,
    detect_spikes,
    evaluate_spikes,
    score_spike,
    score_summary,
)
from esn_anomaly.model import ESNConfig, ESNModel

SPIKE_LEN = spike_length()


class TestDetectSpikes:
    def test_no_spike_in_noise(self):
        rng = np.random.default_rng(0)
        u = rng.normal(0, 0.01, (200, 1))
        regions = detect_spikes(u, spike_threshold=0.3)
        assert len(regions) == 0

    def test_single_spike_detected(self):
        u = np.zeros((100, 1))
        u[30:30 + SPIKE_LEN, 0] = 1.0
        regions = detect_spikes(u, spike_threshold=0.3)
        assert len(regions) == 1
        start, end = regions[0]
        assert start <= 30
        assert end >= 30 + SPIKE_LEN

    def test_two_spikes_detected(self):
        u = np.zeros((200, 1))
        u[20:20 + SPIKE_LEN, 0] = 1.0
        u[100:100 + SPIKE_LEN, 0] = -1.0
        regions = detect_spikes(u, spike_threshold=0.3)
        assert len(regions) == 2

    def test_spike_below_threshold_not_detected(self):
        u = np.zeros((100, 1))
        u[30:45, 0] = 0.1  # 閾値 0.3 以下
        regions = detect_spikes(u, spike_threshold=0.3)
        assert len(regions) == 0


class TestScoreSpike:
    def test_zero_errors(self):
        errors = np.zeros(50)
        assert score_spike(errors, 10, 25) == pytest.approx(0.0)

    def test_constant_errors(self):
        errors = np.ones(50) * 2.0
        assert score_spike(errors, 10, 25) == pytest.approx(2.0)

    def test_slice_is_correct(self):
        errors = np.arange(50, dtype=float)
        s = score_spike(errors, 10, 20)
        expected = np.mean(np.arange(10, 20, dtype=float))
        assert s == pytest.approx(expected)

    def test_empty_region_returns_zero(self):
        errors = np.ones(10)
        assert score_spike(errors, 5, 5) == pytest.approx(0.0)


class TestComputeSpikeThreshold:
    def test_3sigma(self):
        scores = [0.1, 0.2, 0.15, 0.12, 0.18]
        arr = np.array(scores)
        th = compute_spike_threshold(scores, method="3sigma")
        assert th == pytest.approx(arr.mean() + 3.0 * arr.std())

    def test_3sigma_custom_multiplier(self):
        scores = [0.1, 0.2, 0.15, 0.12, 0.18]
        arr = np.array(scores)
        th = compute_spike_threshold(scores, method="3sigma", sigma_multiplier=5.0)
        assert th == pytest.approx(arr.mean() + 5.0 * arr.std())

    def test_max(self):
        scores = [0.1, 0.5, 0.3]
        th = compute_spike_threshold(scores, method="max")
        assert th == pytest.approx(0.5)

    def test_percentile99(self):
        scores = list(np.linspace(0.0, 1.0, 100))
        th = compute_spike_threshold(scores, method="percentile99")
        assert th == pytest.approx(np.percentile(scores, 99))

    def test_invalid_method(self):
        with pytest.raises(ValueError):
            compute_spike_threshold([0.1], method="median")


class TestClassifySpike:
    def test_below_threshold_is_normal(self):
        assert classify_spike(0.1, anomaly_threshold=0.5) is False

    def test_above_threshold_is_anomaly(self):
        assert classify_spike(0.9, anomaly_threshold=0.5) is True

    def test_equal_threshold_is_anomaly(self):
        assert classify_spike(0.5, anomaly_threshold=0.5) is True


class TestEndToEnd:
    """ESN + スパイク検知の統合テスト。"""

    @pytest.fixture(scope="class")
    def trained_model_and_threshold(self):
        rng_train = np.random.default_rng(0)
        u_train, _ = generate_spike_train(
            3000, ["sin", "saw"], rate_hz=0.5, rng=rng_train
        )
        X_tr, y_tr = u_train[:-1], u_train[1:]
        model = ESNModel(ESNConfig(units=200, seed=0)).fit(X_tr, y_tr)
        pred_tr = model.predict(X_tr)
        e_tr = compute_residual(y_tr, pred_tr)
        scores = collect_train_spike_scores(u_train[:-1], e_tr, spike_threshold=0.3)
        threshold = compute_spike_threshold(scores, method="3sigma")
        return model, threshold, X_tr

    def test_sin_spike_not_anomaly(self, trained_model_and_threshold):
        model, threshold, X_tr = trained_model_and_threshold
        rng = np.random.default_rng(10)
        u_test, spikes = generate_spike_train(
            200, ["sin"], rate_hz=1.0, rng=rng
        )
        pred = model.predict(u_test[:-1], warmup_data=X_tr[-100:])
        e = compute_residual(u_test[1:], pred)
        results = evaluate_spikes(u_test[:-1], e, threshold, spike_threshold=0.3)
        anomalies = [r for r in results if r["anomaly"]]
        assert len(anomalies) == 0, f"sin スパイクで誤検知: {anomalies}"

    def test_square_spike_is_anomaly(self, trained_model_and_threshold):
        model, threshold, X_tr = trained_model_and_threshold
        rng = np.random.default_rng(20)
        u_test, spikes = generate_spike_train(
            200, ["square"], rate_hz=1.0, rng=rng
        )
        pred = model.predict(u_test[:-1], warmup_data=X_tr[-100:])
        e = compute_residual(u_test[1:], pred)
        results = evaluate_spikes(u_test[:-1], e, threshold, spike_threshold=0.3)
        assert len(results) > 0, "square スパイクが検出されなかった"
        anomalies = [r for r in results if r["anomaly"]]
        assert len(anomalies) == len(results), (
            f"square スパイクの一部が正常と判定: {results}"
        )


class TestScoreSummary:
    def test_counts(self):
        results = [
            {"anomaly": False},
            {"anomaly": True},
            {"anomaly": False},
        ]
        s = score_summary(results)
        assert s["total"] == 3
        assert s["anomaly"] == 1
        assert s["normal"] == 2
