"""異常検知・評価ロジック。

予測誤差の算出、平滑化、閾値設定、検知ポイント特定を行う。
"""

import numpy as np
from numpy.typing import NDArray


def compute_residual(
    y_real: NDArray[np.float64],
    y_pred: NDArray[np.float64],
) -> NDArray[np.float64]:
    """絶対残差誤差 E(t) = |y_real(t) - y_pred(t)| を計算する。

    Args:
        y_real: 実測値 shape (n, 1) または (n,)
        y_pred: 予測値 shape (n, 1) または (n,)

    Returns:
        絶対誤差 shape (n,)
    """
    return np.abs(y_real.ravel() - y_pred.ravel())


def smooth(
    errors: NDArray[np.float64],
    window: int = 15,
) -> NDArray[np.float64]:
    """移動平均による誤差平滑化。

    端点は valid の畳み込みを使い、残りは元の値で埋める。

    Args:
        errors: 絶対誤差系列 shape (n,)
        window: 移動平均ウィンドウ幅

    Returns:
        平滑化後の誤差 shape (n,)
    """
    if window <= 1:
        return errors.copy()
    kernel = np.ones(window) / window
    smoothed = np.convolve(errors, kernel, mode="same")
    # 端点の境界効果を補正（有効サンプル数で再スケール）
    counts = np.convolve(np.ones_like(errors), kernel, mode="same")
    return smoothed / counts * window / window  # counts は1に正規化済み


def compute_threshold(
    train_errors: NDArray[np.float64],
    method: str = "3sigma",
) -> float:
    """訓練誤差から検知閾値を計算する。

    Args:
        train_errors: 訓練データに対する誤差系列
        method: '3sigma'（平均+3σ）または 'max'（最大値）

    Returns:
        閾値（float）
    """
    if method == "3sigma":
        return float(train_errors.mean() + 3.0 * train_errors.std())
    elif method == "max":
        return float(train_errors.max())
    else:
        raise ValueError(f"未知の method: {method!r}。'3sigma' または 'max' を指定してください。")


def detect(
    smoothed_errors: NDArray[np.float64],
    threshold: float,
) -> NDArray[np.intp]:
    """閾値を超えた全インデックスを返す。

    Args:
        smoothed_errors: 平滑化済み誤差系列
        threshold: 検知閾値

    Returns:
        閾値超過のインデックス配列（空の場合は長さ0の配列）
    """
    return np.where(smoothed_errors > threshold)[0]


def first_detection(
    smoothed_errors: NDArray[np.float64],
    threshold: float,
) -> int | None:
    """最初に閾値を超えたインデックスを返す。超えない場合は None。

    Args:
        smoothed_errors: 平滑化済み誤差系列
        threshold: 検知閾値

    Returns:
        最初の検知インデックス、または None
    """
    indices = detect(smoothed_errors, threshold)
    return int(indices[0]) if len(indices) > 0 else None
