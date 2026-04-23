"""2 チャンネル波形の 6 モード分類データ生成。

モード定義（A 波形 - B 波形、6 つの非順序対）:
  0: sin    - sin
  1: sin    - saw
  2: sin    - square
  3: saw    - saw
  4: saw    - square
  5: square - square

各モードで A の振幅=1.0、B の振幅=0.5（Phase 9 と同じ振幅比）。
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray
from scipy.signal import sawtooth
from scipy.signal import square as _sq

FS: float = 30.0
FREQ: float = 2.0

WAVE_SIN = "sin"
WAVE_SAW = "saw"
WAVE_SQUARE = "square"

# 6 モード: {sin, saw, square} の非順序対（繰り返しあり）
MODES: list[tuple[str, str]] = [
    (WAVE_SIN,    WAVE_SIN),
    (WAVE_SIN,    WAVE_SAW),
    (WAVE_SIN,    WAVE_SQUARE),
    (WAVE_SAW,    WAVE_SAW),
    (WAVE_SAW,    WAVE_SQUARE),
    (WAVE_SQUARE, WAVE_SQUARE),
]
N_MODES: int = len(MODES)
MODE_NAMES: list[str] = [f"{a[:3]}-{b[:3]}" for a, b in MODES]
# → ["sin-sin", "sin-saw", "sin-squ", "saw-saw", "saw-squ", "squ-squ"]


def _wave(wtype: str, t: NDArray[np.float64], amp: float = 1.0) -> NDArray[np.float64]:
    """単一チャンネルの波形を生成する。"""
    omega = 2.0 * np.pi * FREQ
    if wtype == WAVE_SIN:
        return amp * np.sin(omega * t)
    elif wtype == WAVE_SAW:
        return amp * sawtooth(omega * t).astype(np.float64)
    elif wtype == WAVE_SQUARE:
        return amp * _sq(omega * t).astype(np.float64)
    else:
        raise ValueError(f"不明な波形種別: {wtype!r}")


def generate_mode_segment(
    mode_idx: int,
    n_steps: int,
    fs: float = FS,
    amp_a: float = 1.0,
    amp_b: float = 0.5,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
    t_offset: float = 0.0,
) -> NDArray[np.float64]:
    """モード mode_idx の 2ch 信号を生成する。

    Args:
        mode_idx:  モードインデックス 0..5
        n_steps:   ステップ数
        fs:        サンプリング周波数 [Hz]
        amp_a:     チャンネル A の振幅
        amp_b:     チャンネル B の振幅
        noise_std: ガウスノイズの標準偏差
        rng:       乱数ジェネレータ
        t_offset:  時刻オフセット（位相の連続性が必要な場合に使用）

    Returns:
        shape (n_steps, 2) の 2ch 信号
    """
    if rng is None:
        rng = np.random.default_rng()
    if mode_idx < 0 or mode_idx >= N_MODES:
        raise ValueError(f"mode_idx は 0..{N_MODES - 1} の範囲で指定してください: {mode_idx}")
    wave_a, wave_b = MODES[mode_idx]
    t = np.arange(n_steps) / fs + t_offset
    a = _wave(wave_a, t, amp_a) + rng.normal(0.0, noise_std, n_steps)
    b = _wave(wave_b, t, amp_b) + rng.normal(0.0, noise_std, n_steps)
    return np.stack([a, b], axis=1)


def generate_train_dataset(
    n_steps_per_mode: int = 2000,
    fs: float = FS,
    amp_a: float = 1.0,
    amp_b: float = 0.5,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[list[NDArray[np.float64]], list[int]]:
    """全 6 モードの訓練セグメントを生成する。

    Returns:
        segments:     モードごとのセグメントリスト [shape (n, 2), ...]  長さ 6
        mode_indices: 各セグメントのモードインデックス [0, 1, ..., 5]
    """
    if rng is None:
        rng = np.random.default_rng()
    segments = [
        generate_mode_segment(i, n_steps_per_mode, fs, amp_a, amp_b, noise_std, rng)
        for i in range(N_MODES)
    ]
    return segments, list(range(N_MODES))


UNKNOWN_MODE_NAME: str = "sin-noise"
"""未知モードの表示名。A=sin 波形、B=ノイズのみ（波形なし）。"""


def generate_unknown_segment(
    n_steps: int,
    fs: float = FS,
    amp_a: float = 1.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """未知モードの 2ch 信号を生成する。

    A チャンネルは通常の sin 波形、B チャンネルはノイズのみ（波形なし）。
    学習済み 6 モードのいずれにも属さない入力として使用する。

    Args:
        n_steps:   ステップ数
        fs:        サンプリング周波数 [Hz]
        amp_a:     チャンネル A の振幅
        noise_std: ガウスノイズの標準偏差
        rng:       乱数ジェネレータ

    Returns:
        shape (n_steps, 2) の 2ch 信号
    """
    if rng is None:
        rng = np.random.default_rng()
    t = np.arange(n_steps) / fs
    a = amp_a * np.sin(2.0 * np.pi * FREQ * t) + rng.normal(0.0, noise_std, n_steps)
    b = rng.normal(0.0, noise_std, n_steps)   # 波形なし、ノイズのみ
    return np.stack([a, b], axis=1)


def generate_test_sequence(
    n_steps_per_mode: int = 1000,
    shuffle_order: bool = True,
    fs: float = FS,
    amp_a: float = 1.0,
    amp_b: float = 0.5,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.int64], list[tuple[int, int, int]]]:
    """全モードを連続して並べたテスト系列を生成する。

    Returns:
        X:        shape (N_MODES * n_steps_per_mode, 2) 連続 2ch 信号
        labels:   shape (N_MODES * n_steps_per_mode,) 各ステップのモードインデックス
        segments: [(mode_idx, start, end), ...] 各モードブロックの情報
    """
    if rng is None:
        rng = np.random.default_rng()
    order = list(range(N_MODES))
    if shuffle_order:
        rng.shuffle(order)

    x_parts: list[NDArray[np.float64]] = []
    label_parts: list[NDArray[np.int64]] = []
    segments: list[tuple[int, int, int]] = []
    pos = 0

    for mode_idx in order:
        seg = generate_mode_segment(
            mode_idx, n_steps_per_mode, fs, amp_a, amp_b, noise_std, rng
        )
        x_parts.append(seg)
        label_parts.append(np.full(n_steps_per_mode, mode_idx, dtype=np.int64))
        segments.append((mode_idx, pos, pos + n_steps_per_mode))
        pos += n_steps_per_mode

    return (
        np.concatenate(x_parts, axis=0),
        np.concatenate(label_parts, axis=0),
        segments,
    )
