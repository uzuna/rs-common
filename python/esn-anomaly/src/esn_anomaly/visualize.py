"""可視化モジュール。

Waveform Plot と Error Plot を生成して PNG に保存する。
"""

from pathlib import Path

import numpy as np
from matplotlib import pyplot as plt
from numpy.typing import NDArray


def plot_results(
    t: NDArray[np.float64],
    y_real: NDArray[np.float64],
    y_pred: NDArray[np.float64],
    smoothed_errors: NDArray[np.float64],
    threshold: float,
    switch_at: int,
    detection_idx: int | None,
    output_path: str | Path = "output/result.png",
    fs: float = 30.0,
) -> None:
    """2段グラフ（波形 + 誤差）を生成して保存する。

    Args:
        t: 時刻配列 [seconds]
        y_real: 実測値 shape (n,)
        y_pred: 予測値 shape (n,)
        smoothed_errors: 平滑化済み誤差 shape (n,)
        threshold: 検知閾値
        switch_at: 切り替えインデックス（サイン→矩形）
        detection_idx: 最初の検知インデックス（None の場合は表示なし）
        output_path: 出力先 PNG パス
        fs: サンプリング周波数（秒変換用）
    """
    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(12, 7), sharex=True)
    fig.suptitle("ESN Anomaly Detection: Sine Wave (train) -> Square Wave (test)", fontsize=12)

    switch_t = t[switch_at] if switch_at < len(t) else t[-1]

    # ─── 上段: 波形プロット ─────────────────────────────────
    ax1.plot(t, y_real.ravel(), label="Actual", alpha=0.7, linewidth=0.8)
    ax1.plot(t, y_pred.ravel(), label="ESN Prediction", linestyle="--", alpha=0.8, linewidth=0.8)
    ax1.axvline(switch_t, color="orange", linestyle=":", linewidth=1.5, label="Wave switch")
    if detection_idx is not None:
        ax1.axvline(
            t[detection_idx],
            color="red",
            linestyle="-",
            linewidth=1.5,
            label=f"Detection t={t[detection_idx]:.2f}s",
        )
    ax1.set_ylabel("Amplitude")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)
    ax1.set_title("Waveform (left: sine / right: square after switch)")

    # ─── 下段: 誤差プロット ─────────────────────────────────
    ax2.plot(
        t, smoothed_errors.ravel(), label="Smoothed Error E(t)", color="steelblue", linewidth=0.9
    )
    ax2.axhline(
        threshold,
        color="crimson",
        linestyle="--",
        linewidth=1.2,
        label=f"Threshold {threshold:.4f}",
    )
    ax2.axvline(switch_t, color="orange", linestyle=":", linewidth=1.5)
    if detection_idx is not None:
        ax2.axvline(
            t[detection_idx],
            color="red",
            linestyle="-",
            linewidth=1.5,
            label=f"First detection t={t[detection_idx]:.2f}s (step={detection_idx})",
        )
        ax2.scatter(
            [t[detection_idx]],
            [smoothed_errors.ravel()[detection_idx]],
            color="red",
            zorder=5,
            s=40,
        )
    ax2.set_xlabel("Time [s]")
    ax2.set_ylabel("Absolute Error")
    ax2.legend(loc="upper left", fontsize=8)
    ax2.grid(True, alpha=0.3)
    ax2.set_title("Prediction Error and Detection Threshold")

    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close(fig)
    print(f"Saved: {output_path}")


def plot_scenario(
    t: NDArray[np.float64],
    y_real: NDArray[np.float64],
    y_pred: NDArray[np.float64],
    smoothed_errors: NDArray[np.float64],
    threshold: float,
    switch_indices: list[int],
    detection_idx: int | None,
    title: str,
    output_path: str | Path = "output/result.png",
) -> None:
    """マルチ波形シナリオ用 2 段グラフを生成して保存する。

    Args:
        t: 時刻配列 [seconds]
        y_real: 実測値 shape (n,)
        y_pred: 予測値 shape (n,)
        smoothed_errors: 平滑化済み誤差 shape (n,)
        threshold: 検知閾値
        switch_indices: 波形切り替えインデックスのリスト（複数可）
        detection_idx: 最初の持続異常検知インデックス（None の場合は表示なし）
        title: グラフタイトル
        output_path: 出力先 PNG パス
    """
    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(12, 7), sharex=True)
    fig.suptitle(title, fontsize=12)

    # ─── 上段: 波形プロット ─────────────────────────────────
    ax1.plot(t, y_real.ravel(), label="Actual", alpha=0.7, linewidth=0.8)
    ax1.plot(t, y_pred.ravel(), label="ESN Prediction", linestyle="--", alpha=0.8, linewidth=0.8)
    for i, sw in enumerate(switch_indices):
        ax1.axvline(
            t[sw] if sw < len(t) else t[-1],
            color="orange",
            linestyle=":",
            linewidth=1.5,
            label="Wave switch" if i == 0 else None,
        )
    if detection_idx is not None:
        ax1.axvline(
            t[detection_idx],
            color="red",
            linestyle="-",
            linewidth=1.5,
            label=f"Detection t={t[detection_idx]:.2f}s",
        )
    ax1.set_ylabel("Amplitude")
    ax1.legend(loc="upper right", fontsize=8)
    ax1.grid(True, alpha=0.3)

    # ─── 下段: 誤差プロット ─────────────────────────────────
    ax2.plot(t, smoothed_errors.ravel(), label="Smoothed Error", color="steelblue", linewidth=0.9)
    ax2.axhline(
        threshold,
        color="crimson",
        linestyle="--",
        linewidth=1.2,
        label=f"Threshold {threshold:.4f}",
    )
    for sw in switch_indices:
        ax2.axvline(
            t[sw] if sw < len(t) else t[-1], color="orange", linestyle=":", linewidth=1.5
        )
    if detection_idx is not None:
        ax2.axvline(
            t[detection_idx],
            color="red",
            linestyle="-",
            linewidth=1.5,
            label=f"First detection t={t[detection_idx]:.2f}s (step={detection_idx})",
        )
        ax2.scatter(
            [t[detection_idx]],
            [smoothed_errors.ravel()[detection_idx]],
            color="red",
            zorder=5,
            s=40,
        )
    ax2.set_xlabel("Time [s]")
    ax2.set_ylabel("Absolute Error")
    ax2.legend(loc="upper left", fontsize=8)
    ax2.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close(fig)
    print(f"Saved: {output_path}")
