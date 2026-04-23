"""data.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.data import (
    SCENARIO_SAWTOOTH_ONLY,
    SCENARIO_SAWTOOTH_TO_SQUARE,
    SCENARIO_SINE_ONLY,
    SCENARIO_SINE_SAWTOOTH_SINE,
    SCENARIO_SINE_TO_SQUARE,
    generate_mixed_train_data,
    generate_sawtooth,
    generate_sine,
    generate_square,
    generate_test_data,
    generate_test_scenario,
    generate_train_data,
    scale_to_unit,
)

FS = 30.0
FREQ = 2.0


class TestGenerateSine:
    def test_shape(self):
        u = generate_sine(100)
        assert u.shape == (100, 1)

    def test_value_range(self):
        u = generate_sine(3000, noise_std=0.0)
        assert u.min() >= -1.01
        assert u.max() <= 1.01

    def test_frequency(self):
        """FFT ピークが 2 Hz 付近にあることを確認。"""
        u = generate_sine(3000, fs=FS, freq=FREQ, noise_std=0.0).ravel()
        freqs = np.fft.rfftfreq(len(u), d=1.0 / FS)
        power = np.abs(np.fft.rfft(u))
        peak_freq = freqs[np.argmax(power)]
        assert abs(peak_freq - FREQ) < 0.5

    def test_reproducibility(self):
        rng = np.random.default_rng(42)
        u1 = generate_sine(100, rng=rng)
        rng = np.random.default_rng(42)
        u2 = generate_sine(100, rng=rng)
        np.testing.assert_array_equal(u1, u2)


class TestGenerateSquare:
    def test_shape(self):
        u = generate_square(100)
        assert u.shape == (100, 1)

    def test_values_are_binary(self):
        u = generate_square(300).ravel()
        unique = np.unique(u)
        assert set(unique).issubset({-1.0, 1.0})


class TestGenerateTrainData:
    def test_shape(self):
        X, y = generate_train_data(200)
        assert X.shape == (199, 1)
        assert y.shape == (199, 1)

    def test_one_step_ahead(self):
        """y[t] == X[t+1] の関係を確認。"""
        X, y = generate_train_data(50, noise_std=0.0)
        np.testing.assert_array_equal(X[1:], y[:-1])


class TestGenerateTestData:
    def test_shape(self):
        X, y, labels = generate_test_data(100, switch_at=50)
        assert X.shape == (99, 1)
        assert y.shape == (99, 1)
        assert labels.shape == (99,)

    def test_switch_point(self):
        """switch_at ステップ以降はラベルが 1 であること。"""
        X, y, labels = generate_test_data(100, switch_at=50)
        assert (labels[:49] == 0).all()
        assert (labels[49:] == 1).all()


class TestScaleToUnit:
    def test_range(self):
        x = np.array([[0.0], [1.0], [2.0], [3.0]])
        s = scale_to_unit(x)
        assert pytest.approx(s.min()) == -1.0
        assert pytest.approx(s.max()) == 1.0

    def test_constant(self):
        x = np.full((10, 1), 5.0)
        s = scale_to_unit(x)
        np.testing.assert_array_equal(s, np.zeros_like(x))


class TestGenerateSawtooth:
    def test_shape(self):
        u = generate_sawtooth(100)
        assert u.shape == (100, 1)

    def test_value_range(self):
        u = generate_sawtooth(3000, noise_std=0.0)
        assert u.min() >= -1.01
        assert u.max() <= 1.01

    def test_frequency(self):
        """FFT ピークが 2 Hz 付近にあることを確認。"""
        u = generate_sawtooth(3000, fs=FS, freq=FREQ, noise_std=0.0).ravel()
        freqs = np.fft.rfftfreq(len(u), d=1.0 / FS)
        power = np.abs(np.fft.rfft(u))
        peak_freq = freqs[np.argmax(power)]
        assert abs(peak_freq - FREQ) < 0.5

    def test_reproducibility(self):
        rng = np.random.default_rng(7)
        u1 = generate_sawtooth(100, rng=rng)
        rng = np.random.default_rng(7)
        u2 = generate_sawtooth(100, rng=rng)
        np.testing.assert_array_equal(u1, u2)

    def test_differs_from_sine(self):
        """ノコギリ波はサイン波と異なること（ノイズなし）。"""
        u_saw = generate_sawtooth(300, noise_std=0.0).ravel()
        u_sin = generate_sine(300, noise_std=0.0).ravel()
        assert not np.allclose(u_saw, u_sin)


class TestGenerateMixedTrainData:
    def test_shape(self):
        X, y = generate_mixed_train_data(n_steps=200, segment_len=50)
        assert X.shape == (199, 1)
        assert y.shape == (199, 1)

    def test_one_step_ahead(self):
        """y[t] == X[t+1] の関係を確認。"""
        X, y = generate_mixed_train_data(n_steps=100, segment_len=50)
        np.testing.assert_array_equal(X[1:], y[:-1])

    def test_segment_alternates(self):
        """セグメントが交互になっていること（第1セグメントと第2セグメントが異なる）。"""
        rng = np.random.default_rng(0)
        X, _ = generate_mixed_train_data(n_steps=100, segment_len=50, noise_std=0.0, rng=rng)
        seg1 = X[:49].ravel()
        seg2 = X[50:99].ravel()
        assert not np.allclose(seg1, seg2)


class TestGenerateTestScenario:
    @pytest.mark.parametrize("scenario_id,n_steps", [
        (SCENARIO_SINE_ONLY, 100),
        (SCENARIO_SAWTOOTH_ONLY, 100),
        (SCENARIO_SINE_TO_SQUARE, 100),
        (SCENARIO_SAWTOOTH_TO_SQUARE, 100),
        (SCENARIO_SINE_SAWTOOTH_SINE, 99),  # 99 // 3 * 3 = 99
    ])
    def test_shape(self, scenario_id, n_steps):
        X, y, labels = generate_test_scenario(scenario_id, n_steps=n_steps)
        assert X.shape == (n_steps - 1, 1)
        assert y.shape == (n_steps - 1, 1)
        assert labels.shape == (n_steps - 1,)

    def test_t1_all_normal(self):
        _, _, labels = generate_test_scenario(SCENARIO_SINE_ONLY, n_steps=100)
        assert (labels == 0).all()

    def test_t2_all_normal(self):
        _, _, labels = generate_test_scenario(SCENARIO_SAWTOOTH_ONLY, n_steps=100)
        assert (labels == 0).all()

    def test_t3_back_half_anomaly(self):
        n = 100
        _, _, labels = generate_test_scenario(SCENARIO_SINE_TO_SQUARE, n_steps=n)
        half = n // 2
        assert (labels[:half - 1] == 0).all()
        assert (labels[half - 1:] == 1).all()

    def test_t4_back_half_anomaly(self):
        n = 100
        _, _, labels = generate_test_scenario(SCENARIO_SAWTOOTH_TO_SQUARE, n_steps=n)
        half = n // 2
        assert (labels[:half - 1] == 0).all()
        assert (labels[half - 1:] == 1).all()

    def test_t5_all_normal(self):
        _, _, labels = generate_test_scenario(SCENARIO_SINE_SAWTOOTH_SINE, n_steps=99)
        assert (labels == 0).all()

    def test_invalid_scenario(self):
        with pytest.raises(ValueError):
            generate_test_scenario(99, n_steps=100)
