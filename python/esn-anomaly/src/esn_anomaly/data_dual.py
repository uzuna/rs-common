"""2 チャンネル同期信号の生成。

通常: A = sin(2πft)、B = 0.5 × sin(2πft)（同期・振幅比リファレンス）
異常セグメント: Poisson 間隔で発生、長さ 180〜520 step（一様ランダム）
  - phase     : B の位相オフセット（0〜360°）
  - amplitude : B の振幅比変化（0.1〜0.8）
  - mixed     : 位相＋振幅の両方を変化
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

FS_DEFAULT: float = 30.0
FREQ_DEFAULT: float = 2.0
AMP_B_NORMAL: float = 0.5   # 通常時の B/A 振幅比

KIND_PHASE = "phase"         # 位相シフト異常
KIND_AMPLITUDE = "amplitude"  # 振幅変化異常
KIND_MIXED = "mixed"          # 両方

SCENARIO_DUAL_NORMAL = 1
SCENARIO_DUAL_PHASE = 2
SCENARIO_DUAL_AMP = 3
SCENARIO_DUAL_MIXED = 4

SEG_LEN_LO: int = 180
SEG_LEN_HI: int = 520


@dataclass(frozen=True)
class DualSegmentInfo:
    """異常セグメントのメタデータ。"""

    start: int        # 開始インデックス (inclusive)
    end: int          # 終了インデックス (exclusive)
    kind: str         # 'phase' / 'amplitude' / 'mixed'
    phase_deg: float  # B の位相オフセット [度]（通常=0）
    amp_ratio: float  # B の振幅比（通常=0.5）

    @property
    def length(self) -> int:
        return self.end - self.start


def _make_ab(
    n_steps: int,
    fs: float,
    freq: float,
    noise_std: float,
    rng: np.random.Generator,
    phase_deg: float = 0.0,
    amp_ratio: float = AMP_B_NORMAL,
) -> NDArray[np.float64]:
    """n_steps の 2ch 信号 [A, B] を生成する（内部用）。"""
    omega = 2.0 * np.pi * freq
    t = np.arange(n_steps) / fs
    a = np.sin(omega * t) + rng.normal(0.0, noise_std, n_steps)
    b = amp_ratio * np.sin(omega * t + np.deg2rad(phase_deg)) + rng.normal(0.0, noise_std, n_steps)
    return np.stack([a.astype(np.float64), b.astype(np.float64)], axis=1)


def generate_dual_train(
    n_steps: int = 5000,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """正常同期 2ch 訓練データを生成する。

    Returns:
        shape (n_steps, 2): 列 0=A、列 1=B
    """
    if rng is None:
        rng = np.random.default_rng()
    return _make_ab(n_steps, fs, freq, noise_std, rng)


def generate_dual_spike_train(
    n_steps: int,
    anomaly_kinds: list[str],
    rate_hz: float = 0.05,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    noise_std: float = 0.01,
    seg_len_lo: int = SEG_LEN_LO,
    seg_len_hi: int = SEG_LEN_HI,
    fixed_phase_deg: float | None = None,
    fixed_amp_ratio: float | None = None,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], list[DualSegmentInfo]]:
    """Poisson 間隔で異常セグメントを埋め込んだ 2ch 時系列を生成する。

    A は常に正常。B は通常 ``AMP_B_NORMAL × A`` だが、
    異常セグメントでは位相・振幅が変化する。

    Args:
        anomaly_kinds: 'phase' / 'amplitude' / 'mixed' のリスト（ループ使用）
        rate_hz: 異常セグメントの平均発生頻度 [Hz]
        seg_len_lo/hi: セグメント長の範囲 [step]（一様分布）
        fixed_phase_deg: 設定時は phase/mixed 異常でこの値を固定（sweep 用）
        fixed_amp_ratio: 設定時は amplitude/mixed 異常でこの値を固定（sweep 用）

    Returns:
        (u, segments): u shape (n_steps, 2)、segments は DualSegmentInfo のリスト
    """
    if rng is None:
        rng = np.random.default_rng()

    # 背景: 正常 A-B 信号を生成しておく
    u = _make_ab(n_steps, fs, freq, noise_std, rng)

    segments: list[DualSegmentInfo] = []
    omega = 2.0 * np.pi * freq
    mean_interval = fs / rate_hz  # Poisson 平均間隔 [step]

    pos = int(rng.exponential(mean_interval))
    kind_idx = 0
    while pos < n_steps:
        slen = int(rng.integers(seg_len_lo, seg_len_hi + 1))
        end = min(pos + slen, n_steps)
        kind = anomaly_kinds[kind_idx % len(anomaly_kinds)]
        kind_idx += 1

        # 異常パラメータを決定
        if kind == KIND_PHASE:
            phase_deg = (
                fixed_phase_deg if fixed_phase_deg is not None
                else float(rng.uniform(30.0, 330.0))
            )
            amp = AMP_B_NORMAL
        elif kind == KIND_AMPLITUDE:
            phase_deg = 0.0
            if fixed_amp_ratio is not None:
                amp = fixed_amp_ratio
            else:
                # 0.5 から著しく外れた振幅（0.35 以下または 0.65 以上）
                amp = float(rng.choice(
                    np.r_[np.linspace(0.1, 0.35, 50), np.linspace(0.65, 0.8, 50)]
                ))
        elif kind == KIND_MIXED:
            phase_deg = (
                fixed_phase_deg if fixed_phase_deg is not None
                else float(rng.uniform(30.0, 330.0))
            )
            if fixed_amp_ratio is not None:
                amp = fixed_amp_ratio
            else:
                amp = float(rng.choice(
                    np.r_[np.linspace(0.1, 0.35, 50), np.linspace(0.65, 0.8, 50)]
                ))
        else:
            raise ValueError(
                f"未知の kind: {kind!r}。'phase' / 'amplitude' / 'mixed' を指定してください。"
            )

        # B チャンネルを異常波形で上書き
        t_seg = np.arange(pos, end) / fs
        b_seg = (
            amp * np.sin(omega * t_seg + np.deg2rad(phase_deg))
            + rng.normal(0.0, noise_std, end - pos)
        )
        u[pos:end, 1] = b_seg.astype(np.float64)
        segments.append(
            DualSegmentInfo(start=pos, end=end, kind=kind, phase_deg=phase_deg, amp_ratio=amp)
        )

        pos = end + int(rng.exponential(mean_interval))

    return u, segments


def generate_dual_test_scenario(
    scenario_id: int,
    n_steps: int = 3000,
    rate_hz: float = 0.05,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], list[DualSegmentInfo]]:
    """テストシナリオを生成する。

    シナリオ一覧:
        1 (NORMAL): 正常のみ（異常セグメントなし）
        2 (PHASE):  B の位相シフト異常
        3 (AMP):    B の振幅変化異常
        4 (MIXED):  位相＋振幅の混合異常

    Returns:
        (u, segments): u shape (n_steps, 2)
    """
    if rng is None:
        rng = np.random.default_rng()

    if scenario_id == SCENARIO_DUAL_NORMAL:
        return generate_dual_train(n_steps, fs, freq, noise_std, rng), []
    elif scenario_id == SCENARIO_DUAL_PHASE:
        kinds = [KIND_PHASE]
    elif scenario_id == SCENARIO_DUAL_AMP:
        kinds = [KIND_AMPLITUDE]
    elif scenario_id == SCENARIO_DUAL_MIXED:
        kinds = [KIND_MIXED]
    else:
        raise ValueError(f"未知のシナリオ ID: {scenario_id}。1〜4 を指定してください。")

    return generate_dual_spike_train(
        n_steps, kinds, rate_hz=rate_hz, fs=fs, freq=freq, noise_std=noise_std, rng=rng
    )
