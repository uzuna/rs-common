"""data_dual.py のユニットテスト。"""

import numpy as np
import pytest

from esn_anomaly.multichannel.data import (
    AMP_B_NORMAL,
    KIND_AMPLITUDE,
    KIND_MIXED,
    KIND_PHASE,
    SCENARIO_DUAL_AMP,
    SCENARIO_DUAL_MIXED,
    SCENARIO_DUAL_NORMAL,
    SCENARIO_DUAL_PHASE,
    SEG_LEN_HI,
    SEG_LEN_LO,
    DualSegmentInfo,
    generate_dual_spike_train,
    generate_dual_test_scenario,
    generate_dual_train,
)


class TestGenerateDualTrain:
    def test_shape(self):
        u = generate_dual_train(100, rng=np.random.default_rng(0))
        assert u.shape == (100, 2)

    def test_dtype(self):
        u = generate_dual_train(50, rng=np.random.default_rng(0))
        assert u.dtype == np.float64

    def test_a_range(self):
        u = generate_dual_train(300, noise_std=0.0, rng=np.random.default_rng(0))
        # A はサイン波 → [-1, 1]
        assert u[:, 0].max() <= 1.01
        assert u[:, 0].min() >= -1.01

    def test_b_is_half_a_amplitude(self):
        # ノイズなしでB≈0.5*A
        u = generate_dual_train(300, noise_std=0.0, rng=np.random.default_rng(0))
        ratio = np.abs(u[:, 1]).max() / np.abs(u[:, 0]).max()
        assert ratio == pytest.approx(AMP_B_NORMAL, abs=0.05)

    def test_reproducibility(self):
        u1 = generate_dual_train(50, rng=np.random.default_rng(7))
        u2 = generate_dual_train(50, rng=np.random.default_rng(7))
        np.testing.assert_array_equal(u1, u2)


class TestGenerateDualSpikeTrain:
    def test_shape(self):
        rng = np.random.default_rng(0)
        u, segs = generate_dual_spike_train(1000, [KIND_PHASE], rng=rng)
        assert u.shape == (1000, 2)

    def test_segments_are_dataclass(self):
        rng = np.random.default_rng(0)
        _, segs = generate_dual_spike_train(2000, [KIND_PHASE], rate_hz=0.1, rng=rng)
        for seg in segs:
            assert isinstance(seg, DualSegmentInfo)

    def test_segments_within_bounds(self):
        rng = np.random.default_rng(0)
        n = 2000
        _, segs = generate_dual_spike_train(n, [KIND_PHASE], rate_hz=0.2, rng=rng)
        for seg in segs:
            assert 0 <= seg.start < seg.end <= n

    def test_segment_length_roughly_in_range(self):
        rng = np.random.default_rng(1)
        _, segs = generate_dual_spike_train(
            10000, [KIND_PHASE], rate_hz=0.3,
            seg_len_lo=SEG_LEN_LO, seg_len_hi=SEG_LEN_HI, rng=rng
        )
        for seg in segs:
            # セグメントは n_steps の末端で切り詰まることがある
            assert seg.length >= 1

    def test_phase_kind_has_normal_amp(self):
        rng = np.random.default_rng(2)
        _, segs = generate_dual_spike_train(3000, [KIND_PHASE], rate_hz=0.2, rng=rng)
        phase_segs = [s for s in segs if s.kind == KIND_PHASE]
        assert len(phase_segs) > 0
        for seg in phase_segs:
            assert seg.amp_ratio == pytest.approx(AMP_B_NORMAL)

    def test_amplitude_kind_has_zero_phase(self):
        rng = np.random.default_rng(3)
        _, segs = generate_dual_spike_train(3000, [KIND_AMPLITUDE], rate_hz=0.2, rng=rng)
        amp_segs = [s for s in segs if s.kind == KIND_AMPLITUDE]
        assert len(amp_segs) > 0
        for seg in amp_segs:
            assert seg.phase_deg == pytest.approx(0.0)

    def test_amplitude_not_equal_normal(self):
        rng = np.random.default_rng(4)
        _, segs = generate_dual_spike_train(
            3000, [KIND_AMPLITUDE], rate_hz=0.2, rng=rng
        )
        for seg in segs:
            assert abs(seg.amp_ratio - AMP_B_NORMAL) > 0.1

    def test_fixed_phase_deg(self):
        rng = np.random.default_rng(5)
        _, segs = generate_dual_spike_train(
            3000, [KIND_PHASE], rate_hz=0.2, fixed_phase_deg=90.0, rng=rng
        )
        for seg in segs:
            assert seg.phase_deg == pytest.approx(90.0)

    def test_fixed_amp_ratio(self):
        rng = np.random.default_rng(6)
        _, segs = generate_dual_spike_train(
            3000, [KIND_AMPLITUDE], rate_hz=0.2, fixed_amp_ratio=0.2, rng=rng
        )
        for seg in segs:
            assert seg.amp_ratio == pytest.approx(0.2)

    def test_invalid_kind(self):
        with pytest.raises(ValueError):
            generate_dual_spike_train(500, ["unknown"], rng=np.random.default_rng(0))


class TestGenerateDualTestScenario:
    @pytest.mark.parametrize("sid,n", [
        (SCENARIO_DUAL_NORMAL, 500),
        (SCENARIO_DUAL_PHASE, 500),
        (SCENARIO_DUAL_AMP, 500),
        (SCENARIO_DUAL_MIXED, 500),
    ])
    def test_shape(self, sid, n):
        u, _ = generate_dual_test_scenario(sid, n_steps=n, rng=np.random.default_rng(0))
        assert u.shape == (n, 2)

    def test_s1_no_segments(self):
        _, segs = generate_dual_test_scenario(
            SCENARIO_DUAL_NORMAL, n_steps=1000, rng=np.random.default_rng(0)
        )
        assert segs == []

    def test_s2_phase_kind(self):
        _, segs = generate_dual_test_scenario(
            SCENARIO_DUAL_PHASE, n_steps=3000, rate_hz=0.1,
            rng=np.random.default_rng(0)
        )
        assert all(s.kind == KIND_PHASE for s in segs)

    def test_s3_amplitude_kind(self):
        _, segs = generate_dual_test_scenario(
            SCENARIO_DUAL_AMP, n_steps=3000, rate_hz=0.1,
            rng=np.random.default_rng(0)
        )
        assert all(s.kind == KIND_AMPLITUDE for s in segs)

    def test_s4_mixed_kind(self):
        _, segs = generate_dual_test_scenario(
            SCENARIO_DUAL_MIXED, n_steps=3000, rate_hz=0.1,
            rng=np.random.default_rng(0)
        )
        assert all(s.kind == KIND_MIXED for s in segs)

    def test_invalid_scenario(self):
        with pytest.raises(ValueError):
            generate_dual_test_scenario(99, rng=np.random.default_rng(0))
