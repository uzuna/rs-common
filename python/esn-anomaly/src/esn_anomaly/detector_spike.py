"""スパイク単位の異常検知ロジック。

バックグラウンド信号からスパイク区間を検出し、
区間内の予測残差の平均絶対誤差（MAE）で正常/異常を分類する。
"""

import numpy as np
from numpy.typing import NDArray


def detect_spikes(
    u: NDArray[np.float64],
    spike_threshold: float = 0.3,
    min_gap: int = 3,
) -> list[tuple[int, int]]:
    """入力信号の振幅からスパイク区間を検出する。

    バックグラウンドはゼロ付近のノイズ（std ≈ 0.01）なので、
    |u(t)| > spike_threshold をスパイクの在・不在とみなす。

    Args:
        u: 入力信号 shape (n,) または (n, 1)
        spike_threshold: スパイク検出の振幅閾値
        min_gap: 隣接する超過区間をつなぐ最小ギャップ長（短い切れ目を無視する）

    Returns:
        スパイク区間 (start, end) のリスト（end は exclusive）
    """
    u1d = np.abs(u.ravel())
    above = u1d > spike_threshold
    # min_gap 以下のギャップを埋めて連続区間に統合
    for i in range(1, len(above)):
        if not above[i] and i + min_gap < len(above):
            if above[i - 1] and np.any(above[i : i + min_gap + 1]):
                above[i : i + min_gap] = True

    regions: list[tuple[int, int]] = []
    in_spike = False
    start = 0
    for i, a in enumerate(above):
        if a and not in_spike:
            in_spike = True
            start = i
        elif not a and in_spike:
            in_spike = False
            regions.append((start, i))
    if in_spike:
        regions.append((start, len(above)))
    return regions


def score_spike(
    errors: NDArray[np.float64],
    start: int,
    end: int,
) -> float:
    """スパイク区間の平均絶対残差（区間 MAE）を返す。

    Args:
        errors: 予測残差系列 shape (n,)
        start: 区間開始インデックス（inclusive）
        end: 区間終了インデックス（exclusive）

    Returns:
        区間 MAE（float）
    """
    return float(errors[start:end].mean())


def compute_spike_threshold(
    spike_scores: list[float],
    method: str = "3sigma",
    sigma_multiplier: float = 3.0,
) -> float:
    """正常スパイクのスコアから異常判定閾値を計算する。

    Args:
        spike_scores: 訓練データの正常スパイクスコアリスト
        method: '3sigma'（平均 + N σ）、'max'（最大値）、または 'percentile99'（99 パーセンタイル）
        sigma_multiplier: 'Nsigma' 法で使用するσの倍率（デフォルト 3.0）

    Returns:
        閾値（float）
    """
    arr = np.array(spike_scores)
    if method == "3sigma":
        return float(arr.mean() + sigma_multiplier * arr.std())
    elif method == "max":
        return float(arr.max())
    elif method == "percentile99":
        return float(np.percentile(arr, 99))
    else:
        raise ValueError(
            f"未知の method: {method!r}。'3sigma' / 'max' / 'percentile99' を指定してください。"
        )


def classify_spike(score: float, anomaly_threshold: float) -> bool:
    """スコアが閾値以上なら異常（True）を返す。"""
    return score >= anomaly_threshold


def evaluate_spikes(
    u: NDArray[np.float64],
    errors: NDArray[np.float64],
    anomaly_threshold: float,
    spike_threshold: float = 0.3,
) -> list[dict]:
    """全スパイクを検出してスコアリングし、正常/異常を分類する。

    Args:
        u: 入力信号 shape (n, 1) または (n,)
        errors: 予測残差系列 shape (n,) （u[1:] と同じ長さ）
        anomaly_threshold: 異常判定閾値
        spike_threshold: スパイク検出の振幅閾値

    Returns:
        各スパイクの情報辞書のリスト:
            {'start': int, 'end': int, 'score': float, 'anomaly': bool}
    """
    regions = detect_spikes(u, spike_threshold=spike_threshold)
    results = []
    for start, end in regions:
        # errors は u[:-1] に対応するため、インデックスはそのまま使える
        err_start = max(0, start - 1)
        err_end = min(len(errors), end - 1)
        score = score_spike(errors, err_start, err_end)
        results.append(
            {
                "start": start,
                "end": end,
                "score": score,
                "anomaly": classify_spike(score, anomaly_threshold),
            }
        )
    return results


def collect_train_spike_scores(
    u_train: NDArray[np.float64],
    errors_train: NDArray[np.float64],
    spike_threshold: float = 0.3,
) -> list[float]:
    """訓練データから正常スパイクのスコアを収集する。

    Args:
        u_train: 訓練入力 shape (n, 1) または (n,)
        errors_train: 訓練残差 shape (n,)
        spike_threshold: スパイク検出の振幅閾値

    Returns:
        正常スパイクの区間 MAE リスト
    """
    regions = detect_spikes(u_train, spike_threshold=spike_threshold)
    scores = []
    for start, end in regions:
        err_start = max(0, start - 1)
        err_end = min(len(errors_train), end - 1)
        scores.append(score_spike(errors_train, err_start, err_end))
    return scores


def score_summary(spike_results: list[dict]) -> dict:
    """スパイク評価結果のサマリを返す。

    Returns:
        {'total': int, 'anomaly': int, 'normal': int}
    """
    total = len(spike_results)
    anomaly = sum(1 for r in spike_results if r["anomaly"])
    return {"total": total, "anomaly": anomaly, "normal": total - anomaly}
