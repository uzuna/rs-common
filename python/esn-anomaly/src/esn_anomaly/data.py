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


def scale_to_unit(x: NDArray[np.float64]) -> NDArray[np.float64]:
    """[-1, 1] へ min-max スケーリングする。

    全体が定数の場合はゼロ配列を返す。
    """
    xmin, xmax = x.min(), x.max()
    if xmax - xmin == 0:
        return np.zeros_like(x)
    return 2.0 * (x - xmin) / (xmax - xmin) - 1.0
