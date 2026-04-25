"""データ生成・正規化ユーティリティ。

サンプリング周波数 30 Hz、サイン波周波数 2 Hz の訓練・テストデータを生成する。
"""

import numpy as np
from numpy.typing import NDArray
from scipy import signal


def generate_sine(
    n_steps: int,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """サイン波を生成する。

    Args:
        n_steps: ステップ数
        fs: サンプリング周波数 [Hz]
        freq: サイン波周波数 [Hz]
        noise_std: ガウスノイズの標準偏差
        rng: 乱数ジェネレータ（再現性のために指定）

    Returns:
        shape (n_steps, 1) の配列
    """
    if rng is None:
        rng = np.random.default_rng()
    t = np.arange(n_steps) / fs
    u = np.sin(2 * np.pi * freq * t) + rng.normal(0.0, noise_std, n_steps)
    return u.reshape(-1, 1)


def generate_square(
    n_steps: int,
    fs: float = 30.0,
    freq: float = 2.0,
) -> NDArray[np.float64]:
    """矩形波を生成する（振幅±1、duty cycle 50%）。

    Args:
        n_steps: ステップ数
        fs: サンプリング周波数 [Hz]
        freq: 矩形波周波数 [Hz]

    Returns:
        shape (n_steps, 1) の配列
    """
    t = np.arange(n_steps) / fs
    u = signal.square(2 * np.pi * freq * t)
    return u.reshape(-1, 1).astype(np.float64)


def generate_train_data(
    n_steps: int = 2000,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """One-step-ahead 予測用の訓練データを生成する。

    Returns:
        (X, y): X は入力 u[0..n-2]、y は教師 u[1..n-1]。shape はそれぞれ (n_steps-1, 1)。
    """
    u = generate_sine(n_steps, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
    return u[:-1], u[1:]


def generate_test_data(
    n_steps: int = 1000,
    switch_at: int = 500,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64], NDArray[np.int32]]:
    """テストデータを生成する。

    前半 (0..switch_at) はサイン波、後半 (switch_at..n_steps) は矩形波。

    Returns:
        (X, y, labels):
            X: 入力 u[0..n-2]、shape (n_steps-1, 1)
            y: 教師 u[1..n-1]、shape (n_steps-1, 1)
            labels: 0=正常, 1=異常, shape (n_steps-1,)
    """
    sine = generate_sine(switch_at, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
    square = generate_square(n_steps - switch_at, fs=fs, freq=freq)
    u = np.concatenate([sine, square], axis=0)
    labels = np.zeros(n_steps - 1, dtype=np.int32)
    labels[switch_at - 1 :] = 1
    return u[:-1], u[1:], labels


def generate_sawtooth(
    n_steps: int,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """ノコギリ波を生成する（振幅±1）。

    Args:
        n_steps: ステップ数
        fs: サンプリング周波数 [Hz]
        freq: ノコギリ波周波数 [Hz]
        noise_std: ガウスノイズの標準偏差
        rng: 乱数ジェネレータ（再現性のために指定）

    Returns:
        shape (n_steps, 1) の配列
    """
    if rng is None:
        rng = np.random.default_rng()
    t = np.arange(n_steps) / fs
    u = signal.sawtooth(2 * np.pi * freq * t).astype(np.float64)
    u += rng.normal(0.0, noise_std, n_steps)
    return u.reshape(-1, 1)


def generate_mixed_train_data(
    n_steps: int = 2000,
    segment_len: int = 250,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """サイン波とノコギリ波を交互に連結した混合訓練データを生成する。

    [sine × segment_len, sawtooth × segment_len] を n_steps になるまで繰り返す。
    セグメント長はデフォルト 250（≒ 16.7 周期）。

    Returns:
        (X, y): One-step-ahead 予測ペア、shape はそれぞれ (n_steps-1, 1)
    """
    if rng is None:
        rng = np.random.default_rng()
    segments = []
    remaining = n_steps
    toggle = True  # True=sine, False=sawtooth
    while remaining > 0:
        length = min(segment_len, remaining)
        if toggle:
            segments.append(generate_sine(length, fs=fs, freq=freq, noise_std=noise_std, rng=rng))
        else:
            segments.append(
                generate_sawtooth(length, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
            )
        remaining -= length
        toggle = not toggle
    u = np.concatenate(segments, axis=0)
    return u[:-1], u[1:]


# テストシナリオ ID の定数
SCENARIO_SINE_ONLY = 1
SCENARIO_SAWTOOTH_ONLY = 2
SCENARIO_SINE_TO_SQUARE = 3
SCENARIO_SAWTOOTH_TO_SQUARE = 4
SCENARIO_SINE_SAWTOOTH_SINE = 5


def generate_test_scenario(
    scenario_id: int,
    n_steps: int = 1000,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], NDArray[np.float64], NDArray[np.int32]]:
    """テストシナリオを生成する。

    シナリオ一覧:
        1 (SINE_ONLY):            sine × n_steps                   → 正常のみ
        2 (SAWTOOTH_ONLY):        sawtooth × n_steps                → 正常のみ
        3 (SINE_TO_SQUARE):       sine × n/2 → square × n/2        → 後半で検知
        4 (SAWTOOTH_TO_SQUARE):   sawtooth × n/2 → square × n/2    → 後半で検知
        5 (SINE_SAWTOOTH_SINE):   sine × n/3 → sawtooth × n/3 → sine × n/3  → 正常のみ

    Returns:
        (X, y, labels):
            X: 入力 u[0..n-2]、shape (n_steps-1, 1)
            y: 教師 u[1..n-1]、shape (n_steps-1, 1)
            labels: 0=正常, 1=異常, shape (n_steps-1,)
    """
    if rng is None:
        rng = np.random.default_rng()

    half = n_steps // 2
    third = n_steps // 3

    if scenario_id == SCENARIO_SINE_ONLY:
        u = generate_sine(n_steps, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        labels = np.zeros(n_steps - 1, dtype=np.int32)

    elif scenario_id == SCENARIO_SAWTOOTH_ONLY:
        u = generate_sawtooth(n_steps, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        labels = np.zeros(n_steps - 1, dtype=np.int32)

    elif scenario_id == SCENARIO_SINE_TO_SQUARE:
        sine = generate_sine(half, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        square = generate_square(n_steps - half, fs=fs, freq=freq)
        u = np.concatenate([sine, square], axis=0)
        labels = np.zeros(n_steps - 1, dtype=np.int32)
        labels[half - 1 :] = 1

    elif scenario_id == SCENARIO_SAWTOOTH_TO_SQUARE:
        saw = generate_sawtooth(half, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        square = generate_square(n_steps - half, fs=fs, freq=freq)
        u = np.concatenate([saw, square], axis=0)
        labels = np.zeros(n_steps - 1, dtype=np.int32)
        labels[half - 1 :] = 1

    elif scenario_id == SCENARIO_SINE_SAWTOOTH_SINE:
        seg1 = generate_sine(third, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        seg2 = generate_sawtooth(third, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        seg3 = generate_sine(n_steps - 2 * third, fs=fs, freq=freq, noise_std=noise_std, rng=rng)
        u = np.concatenate([seg1, seg2, seg3], axis=0)
        labels = np.zeros(n_steps - 1, dtype=np.int32)

    else:
        raise ValueError(f"未知のシナリオID: {scenario_id}。1〜5 を指定してください。")

    return u[:-1], u[1:], labels


def generate_triangle(
    n_steps: int,
    fs: float = 30.0,
    freq: float = 2.0,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """三角波を生成する（振幅±1、width=0.5 のノコギリ波）。

    Args:
        n_steps: ステップ数
        fs: サンプリング周波数 [Hz]
        freq: 三角波周波数 [Hz]
        noise_std: ガウスノイズの標準偏差
        rng: 乱数ジェネレータ（再現性のために指定）

    Returns:
        shape (n_steps, 1) の配列
    """
    if rng is None:
        rng = np.random.default_rng()
    t = np.arange(n_steps) / fs
    u = signal.sawtooth(2 * np.pi * freq * t, width=0.5).astype(np.float64)
    u += rng.normal(0.0, noise_std, n_steps)
    return u.reshape(-1, 1)


def scale_to_unit(x: NDArray[np.float64]) -> NDArray[np.float64]:
    """[-1, 1] へ min-max スケーリングする。

    全体が定数の場合はゼロ配列を返す。
    """
    xmin, xmax = x.min(), x.max()
    if xmax - xmin == 0:
        return np.zeros_like(x)
    return 2.0 * (x - xmin) / (xmax - xmin) - 1.0
