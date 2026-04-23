"""単発スパイク入力の時系列データ生成。

不定期（Poisson 分布）に発生する 1 周期スパイクをバックグラウンドに重畳する。
sin / sawtooth スパイクを正常、square スパイクを異常として扱う。
"""

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import signal as sig

# サンプリング周波数 30 Hz、波形周波数 2 Hz のとき 1 周期 = 15 step
FS_DEFAULT: float = 30.0
FREQ_DEFAULT: float = 2.0


@dataclass(frozen=True)
class SpikeInfo:
    """時系列中に埋め込まれたスパイクのメタデータ。"""

    start: int   # 開始インデックス（inclusive）
    end: int     # 終了インデックス（exclusive）
    kind: str    # 'sin' / 'saw' / 'square'

    @property
    def length(self) -> int:
        return self.end - self.start


# テストシナリオ ID の定数
SCENARIO_SPIKE_SIN_ONLY = 1
SCENARIO_SPIKE_SAW_ONLY = 2
SCENARIO_SPIKE_MIXED_WITH_SQUARE = 3
SCENARIO_SPIKE_SQUARE_ONLY = 4


def spike_length(fs: float = FS_DEFAULT, freq: float = FREQ_DEFAULT) -> int:
    """1 周期のステップ数を返す。"""
    return int(fs / freq)


def generate_spike(
    kind: str,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> NDArray[np.float64]:
    """1 周期分のスパイク波形を生成する。

    Args:
        kind: 'sin' / 'saw' / 'square'
        fs: サンプリング周波数 [Hz]
        freq: 波形周波数 [Hz]
        noise_std: ガウスノイズの標準偏差
        rng: 乱数ジェネレータ

    Returns:
        shape (spike_len,) の配列
    """
    if rng is None:
        rng = np.random.default_rng()
    n = spike_length(fs, freq)
    t = np.arange(n) / fs
    if kind == "sin":
        v = np.sin(2 * np.pi * freq * t)
    elif kind == "saw":
        v = sig.sawtooth(2 * np.pi * freq * t).astype(np.float64)
    elif kind == "square":
        v = sig.square(2 * np.pi * freq * t).astype(np.float64)
    else:
        raise ValueError(f"未知の kind: {kind!r}。'sin' / 'saw' / 'square' を指定してください。")
    v = v + rng.normal(0.0, noise_std, n)
    return v


def generate_spike_train(
    n_steps: int,
    kinds: list[str],
    rate_hz: float = 0.5,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    bg_noise_std: float = 0.01,
    spike_noise_std: float = 0.01,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], list[SpikeInfo]]:
    """Poisson 間隔でスパイクを埋め込んだ時系列を生成する。

    Args:
        n_steps: 総ステップ数
        kinds: スパイクの種類リスト（ループして使用）
        rate_hz: スパイクの平均発生頻度 [Hz]
        fs: サンプリング周波数 [Hz]
        freq: スパイク波形の周波数 [Hz]
        bg_noise_std: バックグラウンドノイズの標準偏差
        spike_noise_std: スパイクに重畳するノイズの標準偏差
        rng: 乱数ジェネレータ

    Returns:
        (u, spikes):
            u: shape (n_steps, 1) の時系列
            spikes: 埋め込まれたスパイクのリスト
    """
    if rng is None:
        rng = np.random.default_rng()
    slen = spike_length(fs, freq)
    mean_interval = fs / rate_hz  # Poisson の平均間隔 [step]

    u = rng.normal(0.0, bg_noise_std, n_steps)
    spikes: list[SpikeInfo] = []

    pos = int(rng.exponential(mean_interval))
    kind_idx = 0
    while pos + slen <= n_steps:
        kind = kinds[kind_idx % len(kinds)]
        spike = generate_spike(kind, fs=fs, freq=freq, noise_std=spike_noise_std, rng=rng)
        u[pos : pos + slen] += spike
        spikes.append(SpikeInfo(start=pos, end=pos + slen, kind=kind))
        kind_idx += 1
        pos += slen + int(rng.exponential(mean_interval))

    return u.reshape(-1, 1), spikes


def generate_spike_test_scenario(
    scenario_id: int,
    n_steps: int = 500,
    rate_hz: float = 0.5,
    fs: float = FS_DEFAULT,
    freq: float = FREQ_DEFAULT,
    rng: np.random.Generator | None = None,
) -> tuple[NDArray[np.float64], list[SpikeInfo]]:
    """テストシナリオを生成する。

    シナリオ一覧:
        1 (SIN_ONLY):               sin スパイクのみ
        2 (SAW_ONLY):               sawtooth スパイクのみ
        3 (MIXED_WITH_SQUARE):      sin/saw 混在 + square スパイク
        4 (SQUARE_ONLY):            square スパイクのみ

    Returns:
        (u, spikes):
            u: shape (n_steps, 1) の時系列
            spikes: スパイク情報リスト（kind='square' が異常）
    """
    if rng is None:
        rng = np.random.default_rng()

    if scenario_id == SCENARIO_SPIKE_SIN_ONLY:
        kinds = ["sin"]
    elif scenario_id == SCENARIO_SPIKE_SAW_ONLY:
        kinds = ["saw"]
    elif scenario_id == SCENARIO_SPIKE_MIXED_WITH_SQUARE:
        kinds = ["sin", "saw", "square"]
    elif scenario_id == SCENARIO_SPIKE_SQUARE_ONLY:
        kinds = ["square"]
    else:
        raise ValueError(f"未知のシナリオ ID: {scenario_id}。1〜4 を指定してください。")

    return generate_spike_train(
        n_steps=n_steps,
        kinds=kinds,
        rate_hz=rate_hz,
        fs=fs,
        freq=freq,
        rng=rng,
    )
