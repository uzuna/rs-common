"""data_spike.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.spike.data import (
    NOISE_TYPES,
    SCENARIO_SPIKE_MIXED_WITH_SQUARE,
    SCENARIO_SPIKE_SAW_ONLY,
    SCENARIO_SPIKE_SIN_ONLY,
    SCENARIO_SPIKE_SQUARE_ONLY,
    SpikeInfo,
    _generate_noise,
    generate_spike,
    generate_spike_test_scenario,
    generate_spike_train,
    spike_length,
)

FS = 30.0
FREQ = 2.0
SPIKE_LEN = spike_length(FS, FREQ)  # = 15


class TestSpikeLength:
    def test_default(self):
        assert spike_length() == 15

    def test_custom(self):
        assert spike_length(fs=60.0, freq=2.0) == 30


class TestGenerateSpike:
    @pytest.mark.parametrize("kind", ["sin", "saw", "square"])
    def test_shape(self, kind):
        v = generate_spike(kind)
        assert v.shape == (SPIKE_LEN,)

    @pytest.mark.parametrize("kind", ["sin", "saw", "square"])
    def test_value_range(self, kind):
        v = generate_spike(kind, noise_std=0.0)
        assert v.min() >= -1.01
        assert v.max() <= 1.01

    def test_invalid_kind(self):
        with pytest.raises(ValueError):
            generate_spike("triangle")

    def test_reproducibility(self):
        rng1 = np.random.default_rng(0)
        rng2 = np.random.default_rng(0)
        v1 = generate_spike("sin", rng=rng1)
        v2 = generate_spike("sin", rng=rng2)
        np.testing.assert_array_equal(v1, v2)

    def test_sin_differs_from_square(self):
        v_sin = generate_spike("sin", noise_std=0.0)
        v_sq = generate_spike("square", noise_std=0.0)
        assert not np.allclose(v_sin, v_sq)


class TestGenerateSpikeTrainShape:
    def test_output_shape(self):
        rng = np.random.default_rng(0)
        u, spikes = generate_spike_train(200, ["sin"], rate_hz=1.0, rng=rng)
        assert u.shape == (200, 1)

    def test_spikes_are_spikeinfo(self):
        rng = np.random.default_rng(0)
        _, spikes = generate_spike_train(300, ["sin", "saw"], rate_hz=1.0, rng=rng)
        for s in spikes:
            assert isinstance(s, SpikeInfo)
            assert s.length == SPIKE_LEN

    def test_spike_within_bounds(self):
        n = 200
        rng = np.random.default_rng(7)
        u, spikes = generate_spike_train(n, ["sin"], rate_hz=1.0, rng=rng)
        for s in spikes:
            assert 0 <= s.start
            assert s.end <= n

    def test_kinds_cycle(self):
        """kinds リストが順番通りにループすること。"""
        rng = np.random.default_rng(1)
        _, spikes = generate_spike_train(500, ["sin", "saw", "square"], rate_hz=2.0, rng=rng)
        for i, s in enumerate(spikes):
            expected = ["sin", "saw", "square"][i % 3]
            assert s.kind == expected

    def test_spike_embedded_in_signal(self):
        """スパイク区間の信号が背景より有意に大きいこと。"""
        rng = np.random.default_rng(2)
        u, spikes = generate_spike_train(300, ["sin"], rate_hz=1.0, bg_noise_std=0.01, rng=rng)
        assert len(spikes) > 0
        s = spikes[0]
        spike_amp = np.abs(u[s.start : s.end]).mean()
        assert spike_amp > 0.1  # バックグラウンドノイズ (0.01) より十分大きい


class TestGenerateSpikeTestScenario:
    @pytest.mark.parametrize("sid", [
        SCENARIO_SPIKE_SIN_ONLY,
        SCENARIO_SPIKE_SAW_ONLY,
        SCENARIO_SPIKE_MIXED_WITH_SQUARE,
        SCENARIO_SPIKE_SQUARE_ONLY,
    ])
    def test_returns_correct_type(self, sid):
        rng = np.random.default_rng(sid)
        u, spikes = generate_spike_test_scenario(sid, n_steps=300, rng=rng)
        assert u.shape[1] == 1
        assert isinstance(spikes, list)

    def test_s1_all_sin(self):
        rng = np.random.default_rng(0)
        _, spikes = generate_spike_test_scenario(SCENARIO_SPIKE_SIN_ONLY, n_steps=300, rng=rng)
        for s in spikes:
            assert s.kind == "sin"

    def test_s2_all_saw(self):
        rng = np.random.default_rng(0)
        _, spikes = generate_spike_test_scenario(SCENARIO_SPIKE_SAW_ONLY, n_steps=300, rng=rng)
        for s in spikes:
            assert s.kind == "saw"

    def test_s4_all_square(self):
        rng = np.random.default_rng(0)
        _, spikes = generate_spike_test_scenario(SCENARIO_SPIKE_SQUARE_ONLY, n_steps=300, rng=rng)
        for s in spikes:
            assert s.kind == "square"

    def test_s3_has_square(self):
        rng = np.random.default_rng(0)
        _, spikes = generate_spike_test_scenario(
            SCENARIO_SPIKE_MIXED_WITH_SQUARE, n_steps=500, rng=rng
        )
        kinds = {s.kind for s in spikes}
        assert "square" in kinds

    def test_invalid_scenario(self):
        with pytest.raises(ValueError):
            generate_spike_test_scenario(99, n_steps=100)


class TestGenerateNoise:
    """_generate_noise の形状・統計特性テスト。"""

    @pytest.mark.parametrize("noise_type", list(NOISE_TYPES))
    def test_shape(self, noise_type):
        rng = np.random.default_rng(0)
        out = _generate_noise(100, noise_type, std=0.1, rng=rng)
        assert out.shape == (100,)
        assert out.dtype == np.float64

    @pytest.mark.parametrize("noise_type", list(NOISE_TYPES))
    def test_std_approx(self, noise_type):
        """標準偏差が指定値の ±50% 以内であること（ピンクノイズを含む）。"""
        rng = np.random.default_rng(42)
        out = _generate_noise(10000, noise_type, std=0.5, rng=rng)
        assert 0.25 < out.std() < 0.75

    def test_invalid_noise_type(self):
        rng = np.random.default_rng(0)
        with pytest.raises(ValueError):
            _generate_noise(10, "brown", std=0.1, rng=rng)

    @pytest.mark.parametrize("noise_type", list(NOISE_TYPES))
    def test_noise_type_param_in_generate_spike(self, noise_type):
        """generate_spike が各ノイズ種類で正常に動作すること。"""
        rng = np.random.default_rng(0)
        v = generate_spike("sin", noise_std=0.1, noise_type=noise_type, rng=rng)
        assert v.shape == (SPIKE_LEN,)

    @pytest.mark.parametrize("noise_type", list(NOISE_TYPES))
    def test_noise_type_param_in_spike_train(self, noise_type):
        """generate_spike_train が各ノイズ種類で正常に動作すること。"""
        rng = np.random.default_rng(0)
        u, spikes = generate_spike_train(
            200, ["sin"], rate_hz=1.0, noise_type=noise_type, rng=rng
        )
        assert u.shape == (200, 1)

    def test_noise_types_differ(self):
        """異なるノイズ種類で出力が一致しないこと。"""
        std = 0.3
        n = 500
        outputs = []
        for nt in NOISE_TYPES:
            rng = np.random.default_rng(0)
            outputs.append(_generate_noise(n, nt, std=std, rng=rng))
        # 少なくとも 1 組は異なるはず
        pairs = [(outputs[i], outputs[j]) for i in range(len(outputs))
                 for j in range(i + 1, len(outputs))]
        assert any(not np.allclose(a, b) for a, b in pairs)
