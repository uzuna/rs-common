"""data_multimode のテスト。"""

import numpy as np
import pytest

from esn_anomaly.data_multimode import (
    MODE_NAMES,
    MODES,
    N_MODES,
    UNKNOWN_MODE_NAME,
    WAVE_SAW,
    WAVE_SIN,
    WAVE_SQUARE,
    generate_mode_segment,
    generate_test_sequence,
    generate_train_dataset,
    generate_unknown_segment,
)


class TestConstants:
    def test_n_modes(self) -> None:
        assert N_MODES == 6

    def test_modes_are_ordered_pairs(self) -> None:
        wave_order = [WAVE_SIN, WAVE_SAW, WAVE_SQUARE]
        for a, b in MODES:
            assert wave_order.index(a) <= wave_order.index(b)

    def test_mode_names_count(self) -> None:
        assert len(MODE_NAMES) == N_MODES

    def test_all_wave_types_appear(self) -> None:
        all_waves = {w for pair in MODES for w in pair}
        assert all_waves == {WAVE_SIN, WAVE_SAW, WAVE_SQUARE}


class TestGenerateModeSegment:
    def test_shape(self) -> None:
        seg = generate_mode_segment(0, 100)
        assert seg.shape == (100, 2)

    def test_dtype(self) -> None:
        seg = generate_mode_segment(0, 100)
        assert seg.dtype == np.float64

    def test_all_modes_valid(self) -> None:
        for i in range(N_MODES):
            seg = generate_mode_segment(i, 50)
            assert seg.shape == (50, 2)

    def test_invalid_mode_negative(self) -> None:
        with pytest.raises(ValueError):
            generate_mode_segment(-1, 50)

    def test_invalid_mode_overflow(self) -> None:
        with pytest.raises(ValueError):
            generate_mode_segment(N_MODES, 50)

    def test_reproducible(self) -> None:
        rng1 = np.random.default_rng(0)
        rng2 = np.random.default_rng(0)
        s1 = generate_mode_segment(0, 100, rng=rng1)
        s2 = generate_mode_segment(0, 100, rng=rng2)
        np.testing.assert_array_equal(s1, s2)

    def test_amplitude_range_noisefree(self) -> None:
        rng = np.random.default_rng(42)
        for i in range(N_MODES):
            seg = generate_mode_segment(i, 500, noise_std=0.0, rng=rng)
            # amp_a=1.0 なので A は [-1, 1] 内
            assert np.abs(seg[:, 0]).max() <= 1.01
            # amp_b=0.5 なので B は [-0.5, 0.5] 内
            assert np.abs(seg[:, 1]).max() <= 0.51

    def test_different_modes_differ(self) -> None:
        rng = np.random.default_rng(0)
        segs = [generate_mode_segment(i, 300, noise_std=0.0, rng=rng) for i in range(N_MODES)]
        for i in range(N_MODES):
            for j in range(i + 1, N_MODES):
                assert not np.allclose(segs[i], segs[j]), f"mode {i} と mode {j} が同一"


class TestGenerateTrainDataset:
    def test_returns_6_segments(self) -> None:
        rng = np.random.default_rng(0)
        segs, indices = generate_train_dataset(100, rng=rng)
        assert len(segs) == N_MODES
        assert indices == list(range(N_MODES))

    def test_segment_shapes(self) -> None:
        rng = np.random.default_rng(0)
        segs, _ = generate_train_dataset(200, rng=rng)
        for seg in segs:
            assert seg.shape == (200, 2)

    def test_segment_dtypes(self) -> None:
        rng = np.random.default_rng(0)
        segs, _ = generate_train_dataset(100, rng=rng)
        for seg in segs:
            assert seg.dtype == np.float64


class TestGenerateTestSequence:
    def test_shape(self) -> None:
        rng = np.random.default_rng(0)
        X, labels, segs = generate_test_sequence(100, rng=rng)
        assert X.shape == (N_MODES * 100, 2)
        assert labels.shape == (N_MODES * 100,)

    def test_all_modes_present(self) -> None:
        rng = np.random.default_rng(0)
        _, labels, _ = generate_test_sequence(100, rng=rng)
        assert set(labels.tolist()) == set(range(N_MODES))

    def test_segments_count(self) -> None:
        rng = np.random.default_rng(0)
        _, _, segs = generate_test_sequence(100, rng=rng)
        assert len(segs) == N_MODES

    def test_segments_cover_all(self) -> None:
        rng = np.random.default_rng(0)
        X, _, segs = generate_test_sequence(100, rng=rng)
        total = sum(end - start for _, start, end in segs)
        assert total == len(X)

    def test_label_consistency(self) -> None:
        rng = np.random.default_rng(0)
        _, labels, segs = generate_test_sequence(100, rng=rng)
        for mode_idx, start, end in segs:
            assert np.all(labels[start:end] == mode_idx)

    def test_no_shuffle_order(self) -> None:
        rng = np.random.default_rng(0)
        _, _, segs = generate_test_sequence(50, shuffle_order=False, rng=rng)
        mode_order = [s[0] for s in segs]
        assert mode_order == list(range(N_MODES))

    def test_dtype(self) -> None:
        rng = np.random.default_rng(0)
        X, labels, _ = generate_test_sequence(50, rng=rng)
        assert X.dtype == np.float64
        assert labels.dtype == np.int64


class TestGenerateUnknownSegment:
    def test_shape(self) -> None:
        seg = generate_unknown_segment(100)
        assert seg.shape == (100, 2)

    def test_dtype(self) -> None:
        seg = generate_unknown_segment(100)
        assert seg.dtype == np.float64

    def test_b_channel_near_zero_noisefree(self) -> None:
        """ノイズなしなら B チャンネルはほぼゼロ（信号成分なし）。"""
        seg = generate_unknown_segment(300, noise_std=1e-9)
        assert np.abs(seg[:, 1]).max() < 1e-6

    def test_a_channel_has_signal(self) -> None:
        """A チャンネルは sin 波形なので振幅が 0.5 以上になる区間がある。"""
        seg = generate_unknown_segment(300, noise_std=0.0)
        assert np.abs(seg[:, 0]).max() > 0.5

    def test_differs_from_known_modes(self) -> None:
        """未知セグメントは B ≈ 0 なので既知モードとは B の RMS が大きく異なる。"""
        rng = np.random.default_rng(0)
        unk = generate_unknown_segment(300, noise_std=0.0, rng=rng)
        for i in range(N_MODES):
            known = generate_mode_segment(i, 300, noise_std=0.0, rng=rng)
            unk_b_rms = float(np.sqrt(np.mean(unk[:, 1] ** 2)))
            known_b_rms = float(np.sqrt(np.mean(known[:, 1] ** 2)))
            assert known_b_rms > unk_b_rms * 5, (
                f"mode {i}: known B RMS {known_b_rms:.4f} が"
                f"未知 B RMS {unk_b_rms:.4f} の 5 倍未満"
            )

    def test_reproducible(self) -> None:
        rng1 = np.random.default_rng(42)
        rng2 = np.random.default_rng(42)
        s1 = generate_unknown_segment(100, rng=rng1)
        s2 = generate_unknown_segment(100, rng=rng2)
        np.testing.assert_array_equal(s1, s2)

    def test_unknown_mode_name_is_str(self) -> None:
        assert isinstance(UNKNOWN_MODE_NAME, str)
        assert len(UNKNOWN_MODE_NAME) > 0
